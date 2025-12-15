//! 全系统集成验证模块
//!
//! 负责:
//! 1. 子系统的健康检查
//! 2. 端到端功能验证
//! 3. 性能基准测试
//! 4. 系统状态报告

use alloc::vec::Vec;
use core::fmt;

/// 子系统类型
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SubsystemType {
    /// 驱动层
    Drivers = 0,
    /// 内核调度
    Kernel = 1,
    /// NPU 推理
    Npu = 2,
    /// 应用层
    Application = 3,
}

impl fmt::Display for SubsystemType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SubsystemType::Drivers => write!(f, "Drivers"),
            SubsystemType::Kernel => write!(f, "Kernel"),
            SubsystemType::Npu => write!(f, "NPU"),
            SubsystemType::Application => write!(f, "Application"),
        }
    }
}

/// 系统组件状态
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ComponentStatus {
    /// 未初始化
    Uninitialized = 0,
    /// 初始化中
    Initializing = 1,
    /// 运行正常
    Running = 2,
    /// 警告状态
    Warning = 3,
    /// 错误状态
    Error = 4,
    /// 已禁用
    Disabled = 5,
}

impl fmt::Display for ComponentStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ComponentStatus::Uninitialized => write!(f, "未初始化"),
            ComponentStatus::Initializing => write!(f, "初始化中"),
            ComponentStatus::Running => write!(f, "正常运行"),
            ComponentStatus::Warning => write!(f, "警告"),
            ComponentStatus::Error => write!(f, "错误"),
            ComponentStatus::Disabled => write!(f, "已禁用"),
        }
    }
}

/// 系统组件信息
#[derive(Debug, Clone)]
pub struct ComponentInfo {
    pub name: &'static str,
    pub subsystem: SubsystemType,
    pub status: ComponentStatus,
    pub error_count: u32,
    pub last_error: Option<&'static str>,
}

impl ComponentInfo {
    pub fn new(name: &'static str, subsystem: SubsystemType) -> Self {
        ComponentInfo {
            name,
            subsystem,
            status: ComponentStatus::Uninitialized,
            error_count: 0,
            last_error: None,
        }
    }
    
    pub fn is_healthy(&self) -> bool {
        self.status == ComponentStatus::Running
    }
}

impl fmt::Display for ComponentInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}: [{}] {} (errors: {})",
            self.name, self.subsystem, self.status, self.error_count
        )
    }
}

/// 系统健康检查报告
#[derive(Debug, Clone)]
pub struct HealthCheckReport {
    /// 总组件数
    pub total_components: u32,
    /// 正常运行的组件数
    pub healthy_components: u32,
    /// 有错误的组件数
    pub error_components: u32,
    /// 系统整体健康度 (0-100%)
    pub overall_health: u32,
    /// 检查时间戳
    pub timestamp: u64,
}

impl HealthCheckReport {
    pub fn new() -> Self {
        HealthCheckReport {
            total_components: 0,
            healthy_components: 0,
            error_components: 0,
            overall_health: 0,
            timestamp: 0,
        }
    }
    
    pub fn is_system_ready(&self) -> bool {
        self.overall_health >= 80 && self.error_components == 0
    }
    
    pub fn update(&mut self, healthy: u32, error: u32, total: u32) {
        self.healthy_components = healthy;
        self.error_components = error;
        self.total_components = total;
        
        if total > 0 {
            self.overall_health = ((healthy * 100) / total) as u32;
        }
    }
}

impl fmt::Display for HealthCheckReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "System Health: {}/{}组件正常 ({}%), 错误: {}",
            self.healthy_components, self.total_components, self.overall_health, self.error_components
        )
    }
}

/// 性能基准数据
#[derive(Debug, Clone)]
pub struct BenchmarkData {
    /// 测试名称
    pub test_name: &'static str,
    /// 迭代次数
    pub iterations: u32,
    /// 最小耗时 (毫秒)
    pub min_time_ms: f32,
    /// 最大耗时 (毫秒)
    pub max_time_ms: f32,
    /// 平均耗时 (毫秒)
    pub avg_time_ms: f32,
    /// 吞吐量 (操作/秒)
    pub throughput: f32,
}

impl BenchmarkData {
    pub fn new(test_name: &'static str, iterations: u32) -> Self {
        BenchmarkData {
            test_name,
            iterations,
            min_time_ms: f32::MAX,
            max_time_ms: 0.0,
            avg_time_ms: 0.0,
            throughput: 0.0,
        }
    }
    
    pub fn update_stats(&mut self, total_time_ms: f32) {
        self.avg_time_ms = total_time_ms / self.iterations as f32;
        
        if self.avg_time_ms < self.min_time_ms {
            self.min_time_ms = self.avg_time_ms;
        }
        if self.avg_time_ms > self.max_time_ms {
            self.max_time_ms = self.avg_time_ms;
        }
        
        if self.avg_time_ms > 0.0 {
            self.throughput = 1000.0 / self.avg_time_ms;
        }
    }
}

impl fmt::Display for BenchmarkData {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}: avg={:.2}ms, min={:.2}ms, max={:.2}ms, throughput={:.1}/s",
            self.test_name, self.avg_time_ms, self.min_time_ms, self.max_time_ms, self.throughput
        )
    }
}

/// 全系统集成管理器
pub struct SystemIntegrationManager {
    /// 注册的组件
    components: Vec<ComponentInfo>,
    /// 健康检查报告
    health_report: HealthCheckReport,
    /// 性能基准数据
    benchmarks: Vec<BenchmarkData>,
}

impl SystemIntegrationManager {
    /// 创建新的系统管理器
    pub fn new() -> Self {
        SystemIntegrationManager {
            components: Vec::new(),
            health_report: HealthCheckReport::new(),
            benchmarks: Vec::new(),
        }
    }
    
    /// 注册系统组件
    pub fn register_component(&mut self, component: ComponentInfo) -> usize {
        let id = self.components.len();
        self.components.push(component);
        id
    }
    
    /// 更新组件状态
    pub fn update_component_status(
        &mut self,
        component_id: usize,
        status: ComponentStatus,
        error: Option<&'static str>,
    ) -> Result<(), &'static str> {
        if component_id >= self.components.len() {
            return Err("Component ID out of bounds");
        }
        
        let component = &mut self.components[component_id];
        component.status = status;
        
        if let Some(err) = error {
            component.error_count += 1;
            component.last_error = Some(err);
        }
        
        Ok(())
    }
    
    /// 执行系统健康检查
    pub fn perform_health_check(&mut self) -> HealthCheckReport {
        let total = self.components.len() as u32;
        let healthy = self.components
            .iter()
            .filter(|c| c.is_healthy())
            .count() as u32;
        let error = self.components
            .iter()
            .filter(|c| c.status == ComponentStatus::Error)
            .count() as u32;
        
        self.health_report.update(healthy, error, total);
        self.health_report.clone()
    }
    
    /// 添加性能基准数据
    pub fn add_benchmark(&mut self, benchmark: BenchmarkData) {
        self.benchmarks.push(benchmark);
    }
    
    /// 生成系统报告
    pub fn generate_report(&self) -> SystemReport {
        let mut component_summaries = Vec::new();
        
        for component in &self.components {
            component_summaries.push(format!("{}", component));
        }
        
        SystemReport {
            total_components: self.components.len() as u32,
            health_status: self.health_report.overall_health,
            component_summaries,
            benchmark_count: self.benchmarks.len() as u32,
        }
    }
    
    /// 获取系统是否准备就绪
    pub fn is_system_ready(&self) -> bool {
        self.health_report.is_system_ready()
    }
}

/// 系统报告
#[derive(Debug, Clone)]
pub struct SystemReport {
    pub total_components: u32,
    pub health_status: u32,
    pub component_summaries: Vec<alloc::string::String>,
    pub benchmark_count: u32,
}

impl fmt::Display for SystemReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "System Report: {} components, Health: {}%, {} benchmarks",
            self.total_components, self.health_status, self.benchmark_count
        )
    }
}

/// 全局系统管理器
use lazy_static::lazy_static;

lazy_static! {
    pub static ref SYSTEM_MANAGER: spin::Mutex<SystemIntegrationManager> =
        spin::Mutex::new(SystemIntegrationManager::new());
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_component_info() {
        let comp = ComponentInfo::new("test_component", SubsystemType::Drivers);
        assert_eq!(comp.name, "test_component");
        assert!(!comp.is_healthy());
    }
    
    #[test]
    fn test_health_check() {
        let mut report = HealthCheckReport::new();
        report.update(8, 1, 10);
        assert_eq!(report.overall_health, 80);
    }
    
    #[test]
    fn test_benchmark_data() {
        let mut bench = BenchmarkData::new("test", 100);
        bench.update_stats(1000.0);
        assert_eq!(bench.avg_time_ms, 10.0);
    }
}
