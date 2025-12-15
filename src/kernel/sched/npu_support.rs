//! HMP 调度器 NPU 支持
//!
//! 负责:
//! 1. NPU 任务的识别和调度
//! 2. CPU-NPU 协同计算管理
//! 3. 上下文切换和亲和性处理
//! 4. NPU 推理任务优先级管理

use alloc::vec::Vec;
use core::fmt;

/// NPU 任务类型
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum NpuTaskType {
    /// 预处理 (数据准备)
    Preprocess = 0,
    /// NPU 推理
    Inference = 1,
    /// 后处理 (结果处理)
    Postprocess = 2,
}

impl fmt::Display for NpuTaskType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            NpuTaskType::Preprocess => write!(f, "Preprocess"),
            NpuTaskType::Inference => write!(f, "Inference"),
            NpuTaskType::Postprocess => write!(f, "Postprocess"),
        }
    }
}

/// NPU 上下文管理
#[derive(Debug, Clone)]
pub struct NpuContext {
    /// 上下文 ID
    pub context_id: u32,
    /// 模型名称
    pub model_name: &'static str,
    /// 当前任务类型
    pub current_task: NpuTaskType,
    /// 推理状态 (0=空闲, 1=运行中, 2=等待结果)
    pub inference_state: u32,
    /// NPU 利用率 (0-100%)
    pub utilization: u32,
}

impl NpuContext {
    /// 创建新的 NPU 上下文
    pub fn new(context_id: u32, model_name: &'static str) -> Self {
        NpuContext {
            context_id,
            model_name,
            current_task: NpuTaskType::Preprocess,
            inference_state: 0,
            utilization: 0,
        }
    }
    
    /// 启动预处理任务
    pub fn start_preprocess(&mut self) {
        self.current_task = NpuTaskType::Preprocess;
    }
    
    /// 启动推理任务
    pub fn start_inference(&mut self) {
        self.current_task = NpuTaskType::Inference;
        self.inference_state = 1;
    }
    
    /// 推理完成
    pub fn finish_inference(&mut self) {
        self.inference_state = 2;
        self.current_task = NpuTaskType::Postprocess;
    }
    
    /// 启动后处理任务
    pub fn start_postprocess(&mut self) {
        self.current_task = NpuTaskType::Postprocess;
    }
    
    /// 任务完成
    pub fn task_done(&mut self) {
        self.inference_state = 0;
        self.utilization = 0;
    }
}

/// NPU 任务调度策略
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum NpuSchedulePolicy {
    /// 尽早完成 (ASAP) - 最小化延迟
    ASAP = 0,
    /// 最小功耗
    MinPower = 1,
    /// 平衡模式
    Balanced = 2,
}

/// NPU 调度决策
#[derive(Debug, Clone)]
pub struct NpuScheduleDecision {
    /// 建议的 CPU 集合 (位掩码)
    pub suggested_cpus: u8,
    /// 首选 CPU ID
    pub preferred_cpu: Option<u8>,
    /// 推荐的频率档位 (0-4)
    pub freq_level: u8,
    /// 预估的执行时间 (毫秒)
    pub estimated_time_ms: u32,
}

impl fmt::Display for NpuScheduleDecision {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Schedule: CPUs={:08b}, preferred={:?}, freq={}, time={}ms",
            self.suggested_cpus, self.preferred_cpu, self.freq_level, self.estimated_time_ms
        )
    }
}

/// NPU 调度器
pub struct NpuScheduler {
    /// 活跃的 NPU 上下文
    contexts: Vec<NpuContext>,
    /// 调度策略
    policy: NpuSchedulePolicy,
}

impl NpuScheduler {
    /// 创建新的 NPU 调度器
    pub fn new(policy: NpuSchedulePolicy) -> Self {
        NpuScheduler {
            contexts: Vec::new(),
            policy,
        }
    }
    
    /// 注册 NPU 上下文
    pub fn register_context(&mut self, context: NpuContext) -> Result<u32, &'static str> {
        if self.contexts.len() >= 8 {
            return Err("Too many NPU contexts");
        }
        
        let id = context.context_id;
        self.contexts.push(context);
        Ok(id)
    }
    
    /// 获取调度决策
    pub fn get_schedule_decision(&self, task_type: NpuTaskType) -> NpuScheduleDecision {
        match self.policy {
            NpuSchedulePolicy::ASAP => self.schedule_asap(task_type),
            NpuSchedulePolicy::MinPower => self.schedule_min_power(task_type),
            NpuSchedulePolicy::Balanced => self.schedule_balanced(task_type),
        }
    }
    
    /// ASAP 策略 (最小化延迟)
    fn schedule_asap(&self, task_type: NpuTaskType) -> NpuScheduleDecision {
        match task_type {
            NpuTaskType::Preprocess => {
                // 预处理: 在 A76 上执行 (高性能)
                NpuScheduleDecision {
                    suggested_cpus: 0x0F,  // A76 核心 0-3
                    preferred_cpu: Some(0),
                    freq_level: 4,  // 最高频率
                    estimated_time_ms: 10,
                }
            }
            NpuTaskType::Inference => {
                // NPU 推理: 不消耗 CPU，但准备数据的 CPU 应该空闲
                NpuScheduleDecision {
                    suggested_cpus: 0xF0,  // A55 核心可用
                    preferred_cpu: None,
                    freq_level: 0,  // NPU 独立运行
                    estimated_time_ms: 50,
                }
            }
            NpuTaskType::Postprocess => {
                // 后处理: 在 A76 上执行 (高性能)
                NpuScheduleDecision {
                    suggested_cpus: 0x0F,
                    preferred_cpu: Some(1),
                    freq_level: 4,
                    estimated_time_ms: 10,
                }
            }
        }
    }
    
    /// 最小功耗策略
    fn schedule_min_power(&self, task_type: NpuTaskType) -> NpuScheduleDecision {
        match task_type {
            NpuTaskType::Preprocess => {
                // 预处理: 在 A55 上执行 (低功耗)
                NpuScheduleDecision {
                    suggested_cpus: 0xF0,  // A55 核心 4-7
                    preferred_cpu: Some(4),
                    freq_level: 1,  // 低频率
                    estimated_time_ms: 20,
                }
            }
            NpuTaskType::Inference => {
                NpuScheduleDecision {
                    suggested_cpus: 0xF0,
                    preferred_cpu: None,
                    freq_level: 0,
                    estimated_time_ms: 50,
                }
            }
            NpuTaskType::Postprocess => {
                NpuScheduleDecision {
                    suggested_cpus: 0xF0,
                    preferred_cpu: Some(5),
                    freq_level: 1,
                    estimated_time_ms: 20,
                }
            }
        }
    }
    
    /// 平衡策略
    fn schedule_balanced(&self, task_type: NpuTaskType) -> NpuScheduleDecision {
        match task_type {
            NpuTaskType::Preprocess => {
                // 预处理: 在 A76 上 (需要速度)
                NpuScheduleDecision {
                    suggested_cpus: 0x0F,
                    preferred_cpu: Some(0),
                    freq_level: 3,  // 中等高频率
                    estimated_time_ms: 15,
                }
            }
            NpuTaskType::Inference => {
                NpuScheduleDecision {
                    suggested_cpus: 0xFF,  // 任何核心都可以
                    preferred_cpu: None,
                    freq_level: 0,
                    estimated_time_ms: 50,
                }
            }
            NpuTaskType::Postprocess => {
                NpuScheduleDecision {
                    suggested_cpus: 0x0F,
                    preferred_cpu: Some(1),
                    freq_level: 3,
                    estimated_time_ms: 15,
                }
            }
        }
    }
    
    /// 获取所有活跃的 NPU 上下文
    pub fn get_contexts(&self) -> &[NpuContext] {
        &self.contexts
    }
    
    /// 获取 NPU 总利用率
    pub fn get_total_utilization(&self) -> u32 {
        let total: u32 = self.contexts.iter().map(|c| c.utilization as u32).sum();
        (total / (self.contexts.len() as u32).max(1)).min(100)
    }
}

/// 全局 NPU 调度器实例
use lazy_static::lazy_static;

lazy_static! {
    pub static ref NPU_SCHEDULER: spin::Mutex<NpuScheduler> =
        spin::Mutex::new(NpuScheduler::new(NpuSchedulePolicy::Balanced));
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_npu_context() {
        let mut ctx = NpuContext::new(0, "yolov8");
        assert_eq!(ctx.current_task, NpuTaskType::Preprocess);
        
        ctx.start_inference();
        assert_eq!(ctx.current_task, NpuTaskType::Inference);
        assert_eq!(ctx.inference_state, 1);
    }
    
    #[test]
    fn test_npu_scheduler() {
        let mut scheduler = NpuScheduler::new(NpuSchedulePolicy::ASAP);
        let ctx = NpuContext::new(0, "yolov8");
        assert!(scheduler.register_context(ctx).is_ok());
    }
    
    #[test]
    fn test_schedule_decision() {
        let scheduler = NpuScheduler::new(NpuSchedulePolicy::ASAP);
        let decision = scheduler.get_schedule_decision(NpuTaskType::Preprocess);
        assert_eq!(decision.freq_level, 4);
    }
}
