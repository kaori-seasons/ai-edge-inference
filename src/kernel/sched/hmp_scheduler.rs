//! 异构调度器 (HMP Scheduler)
//!
//! 支持A76/A55异构多核架构的任务调度
//! 实现任务亲和性提示和负载均衡

use core::fmt;
use lazy_static::lazy_static;
use alloc::vec::Vec;

/// 任务亲和性提示
#[repr(u32)]
#[derive(Debug, Clone, Copy)]
pub enum TaskHint {
    /// 高性能计算任务 - 分配给A76核心
    /// 用于: NMS、图像缩放、复杂算法
    HighPerf = 0,
    
    /// 低功耗任务 - 优先分配给A55核心
    /// 用于: 后台服务、传感器轮询、I2C通信
    LowPower = 1,
    
    /// NPU前后处理 - 强制A76核心
    /// 用于: 数据准备、输出处理(NMS)
    /// 目的: 确保NPU的前置和后置计算在同一高性能核心
    NpuPrePost = 2,
}

/// 任务结构体
pub struct Task {
    /// 任务ID
    pub id: u32,
    
    /// 任务优先级 (0-255, 越低越高)
    pub priority: u8,
    
    /// 亲和性提示
    pub hint: TaskHint,
    
    /// 分配的CPU ID (0-7, 0-3是A76, 4-7是A55)
    pub assigned_cpu: Option<u32>,
    
    /// 任务状态
    pub state: TaskState,
}

/// 任务状态
#[repr(u8)]
#[derive(Debug, Clone, Copy)]
pub enum TaskState {
    /// 等待调度
    Pending = 0,
    /// 正在运行
    Running = 1,
    /// 等待资源
    Waiting = 2,
    /// 已完成
    Done = 3,
}

impl Task {
    /// 创建新任务
    pub fn new(id: u32, priority: u8, hint: TaskHint) -> Self {
        Task {
            id,
            priority,
            hint,
            assigned_cpu: None,
            state: TaskState::Pending,
        }
    }
}

/// CPU核心负载信息
#[derive(Debug, Clone)]
pub struct CoreLoad {
    /// CPU ID
    pub cpu_id: u32,
    
    /// 当前负载百分比 (0-100)
    pub load_percent: u32,
    
    /// 运行中的任务数
    pub task_count: u32,
}

impl CoreLoad {
    pub fn new(cpu_id: u32) -> Self {
        CoreLoad {
            cpu_id,
            load_percent: 0,
            task_count: 0,
        }
    }
}

/// 异构调度器
pub struct HmpScheduler {
    /// A76核心负载 (CPU 0-3)
    a76_loads: [CoreLoad; 4],
    
    /// A55核心负载 (CPU 4-7)
    a55_loads: [CoreLoad; 4],
    
    /// 任务队列
    task_queue: Vec<Task>,
    
    /// 调度器配置
    config: SchedulerConfig,
}

/// 调度器配置
pub struct SchedulerConfig {
    /// A55负载阈值 (%) - 超过此值降级到A76
    pub a55_load_threshold: u32,
    
    /// A76负载阈值 (%) - 保留空间应对高优先级任务
    pub a76_load_threshold: u32,
    
    /// 是否启用负载均衡
    pub enable_load_balance: bool,
    
    /// 负载均衡时间间隔 (ms)
    pub rebalance_interval_ms: u32,
}

impl Default for SchedulerConfig {
    fn default() -> Self {
        SchedulerConfig {
            a55_load_threshold: 60,   // A55 60%
            a76_load_threshold: 50,   // A76 50% (保留带宽)
            enable_load_balance: true,
            rebalance_interval_ms: 100,
        }
    }
}

impl HmpScheduler {
    /// 创建新的调度器
    pub fn new() -> Self {
        HmpScheduler {
            a76_loads: [
                CoreLoad::new(0),
                CoreLoad::new(1),
                CoreLoad::new(2),
                CoreLoad::new(3),
            ],
            a55_loads: [
                CoreLoad::new(4),
                CoreLoad::new(5),
                CoreLoad::new(6),
                CoreLoad::new(7),
            ],
            task_queue: Vec::new(),
            config: SchedulerConfig::default(),
        }
    }
    
    /// 获取平均A76负载
    fn get_a76_avg_load(&self) -> u32 {
        let sum: u32 = self.a76_loads.iter().map(|l| l.load_percent).sum();
        sum / 4
    }
    
    /// 获取平均A55负载
    fn get_a55_avg_load(&self) -> u32 {
        let sum: u32 = self.a55_loads.iter().map(|l| l.load_percent).sum();
        sum / 4
    }
    
    /// 查找最空闲的A76核心
    fn find_least_loaded_a76(&self) -> u32 {
        let mut min_idx = 0;
        let mut min_load = self.a76_loads[0].load_percent;
        
        for i in 1..4 {
            if self.a76_loads[i].load_percent < min_load {
                min_load = self.a76_loads[i].load_percent;
                min_idx = i;
            }
        }
        
        min_idx as u32
    }
    
    /// 查找最空闲的A55核心
    fn find_least_loaded_a55(&self) -> u32 {
        let mut min_idx = 0;
        let mut min_load = self.a55_loads[0].load_percent;
        
        for i in 1..4 {
            if self.a55_loads[i].load_percent < min_load {
                min_load = self.a55_loads[i].load_percent;
                min_idx = i;
            }
        }
        
        min_idx as u32
    }
    
    /// 决策任务应该分配给哪个CPU
    pub fn decide_cpu(&self, task: &Task) -> u32 {
        match task.hint {
            TaskHint::HighPerf | TaskHint::NpuPrePost => {
                // 高性能任务: 强制分配给A76
                let a76_idx = self.find_least_loaded_a76();
                a76_idx  // 返回0-3
            }
            TaskHint::LowPower => {
                // 低功耗任务: 优先分配给A55
                let avg_a55_load = self.get_a55_avg_load();
                
                if avg_a55_load < self.config.a55_load_threshold {
                    // A55空闲, 优先使用
                    let a55_idx = self.find_least_loaded_a55();
                    4 + a55_idx  // 返回4-7
                } else {
                    // A55繁忙, 降级到A76
                    let a76_idx = self.find_least_loaded_a76();
                    a76_idx  // 返回0-3
                }
            }
        }
    }
    
    /// 提交任务到调度器
    pub fn submit_task(&mut self, mut task: Task) -> Result<u32, &'static str> {
        // 决策CPU分配
        let assigned_cpu = self.decide_cpu(&task);
        task.assigned_cpu = Some(assigned_cpu);
        task.state = TaskState::Running;
        
        // 更新CPU负载
        if assigned_cpu < 4 {
            // A76
            self.a76_loads[assigned_cpu as usize].task_count += 1;
            self.a76_loads[assigned_cpu as usize].load_percent =
                (self.a76_loads[assigned_cpu as usize].task_count * 25).min(100);
        } else {
            // A55
            let idx = (assigned_cpu - 4) as usize;
            self.a55_loads[idx].task_count += 1;
            self.a55_loads[idx].load_percent = (self.a55_loads[idx].task_count * 25).min(100);
        }
        
        self.task_queue.push(task);
        
        Ok(assigned_cpu)
    }
    
    /// 完成任务
    pub fn finish_task(&mut self, task_id: u32) -> Result<(), &'static str> {
        if let Some(pos) = self.task_queue.iter().position(|t| t.id == task_id) {
            let task = &self.task_queue[pos];
            
            if let Some(cpu_id) = task.assigned_cpu {
                // 更新CPU负载
                if cpu_id < 4 {
                    self.a76_loads[cpu_id as usize].task_count =
                        self.a76_loads[cpu_id as usize].task_count.saturating_sub(1);
                    self.a76_loads[cpu_id as usize].load_percent =
                        (self.a76_loads[cpu_id as usize].task_count * 25).min(100);
                } else {
                    let idx = (cpu_id - 4) as usize;
                    self.a55_loads[idx].task_count = self.a55_loads[idx].task_count.saturating_sub(1);
                    self.a55_loads[idx].load_percent =
                        (self.a55_loads[idx].task_count * 25).min(100);
                }
            }
            
            self.task_queue.remove(pos);
            Ok(())
        } else {
            Err("Task not found")
        }
    }
    
    /// 获取系统负载统计
    pub fn get_load_stats(&self) -> (u32, u32, u32) {
        let a76_avg = self.get_a76_avg_load();
        let a55_avg = self.get_a55_avg_load();
        let total_avg = (a76_avg + a55_avg) / 2;
        
        (a76_avg, a55_avg, total_avg)
    }
    
    /// 打印调度器状态
    pub fn print_status(&self) {
        // Note: println! debug output - can be enabled in main.rs if needed
        // [HMP Scheduler] Status info stored in self.a76_loads, self.a55_loads
    }
}

/// 全局HMP调度器实例
lazy_static! {
    pub static ref HMP_SCHEDULER: spin::Mutex<HmpScheduler> = 
        spin::Mutex::new(HmpScheduler::new());
}

/// 初始化异构调度器
pub fn hmp_init() {
    // Scheduler initialization in HMP_SCHEDULER static
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_task_creation() {
        let task = Task::new(0, 50, TaskHint::HighPerf);
        assert_eq!(task.id, 0);
        assert_eq!(task.priority, 50);
        assert!(matches!(task.state, TaskState::Pending));
    }
    
    #[test]
    fn test_scheduler_decision() {
        let scheduler = HmpScheduler::new();
        
        let high_perf_task = Task::new(1, 50, TaskHint::HighPerf);
        let cpu_id = scheduler.decide_cpu(&high_perf_task);
        assert!(cpu_id < 4); // 应该分配给A76
        
        let low_power_task = Task::new(2, 50, TaskHint::LowPower);
        let cpu_id = scheduler.decide_cpu(&low_power_task);
        // 在空闲的A55空闲时应该返回4-7
        // 在这个测试中, A55和A76都空闲, 所以应该返回4-7
        assert!(cpu_id >= 4);
    }
}
