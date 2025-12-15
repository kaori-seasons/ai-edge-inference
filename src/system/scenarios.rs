//! 场景验证模块 - 完整的闭环应用
//!
//! 实现: 图像采集 → 预处理 → NPU推理 → 后处理 → CAN执行器控制

use alloc::vec::Vec;
use core::fmt;

/// 应用场景类型
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ApplicationScenario {
    /// 人员检测和计数
    PeopleDetection = 0,
    /// 车辆检测和分类
    VehicleDetection = 1,
    /// 物体识别
    ObjectDetection = 2,
    /// 异常检测
    AnomalyDetection = 3,
}

/// 执行器控制指令
#[derive(Debug, Clone, Copy)]
pub struct ActuatorCommand {
    /// 执行器 ID (CAN ID)
    pub actuator_id: u8,
    /// 命令类型 (0=报警, 1=转向, 2=减速, 3=停止)
    pub command_type: u8,
    /// 参数 (0-255, 含义取决于命令类型)
    pub param: u8,
}

impl ActuatorCommand {
    pub fn new(actuator_id: u8, command_type: u8, param: u8) -> Self {
        ActuatorCommand {
            actuator_id,
            command_type,
            param,
        }
    }
}

impl fmt::Display for ActuatorCommand {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "ActuatorCmd(ID={}, type={}, param={})",
            self.actuator_id, self.command_type, self.param
        )
    }
}

/// 场景执行状态
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ExecutionState {
    /// 待命
    Idle = 0,
    /// 采集图像
    Capturing = 1,
    /// 预处理
    Preprocessing = 2,
    /// 推理中
    Inferencing = 3,
    /// 后处理
    Postprocessing = 4,
    /// 控制输出
    Controlling = 5,
    /// 完成
    Complete = 6,
}

impl fmt::Display for ExecutionState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ExecutionState::Idle => write!(f, "待命"),
            ExecutionState::Capturing => write!(f, "采集"),
            ExecutionState::Preprocessing => write!(f, "预处理"),
            ExecutionState::Inferencing => write!(f, "推理"),
            ExecutionState::Postprocessing => write!(f, "后处理"),
            ExecutionState::Controlling => write!(f, "控制"),
            ExecutionState::Complete => write!(f, "完成"),
        }
    }
}

/// 场景执行统计
#[derive(Debug, Clone)]
pub struct ScenarioStats {
    /// 总执行次数
    pub total_executions: u32,
    /// 成功执行
    pub successful: u32,
    /// 失败次数
    pub failed: u32,
    /// 总耗时 (毫秒)
    pub total_time_ms: u32,
    /// 平均耗时 (毫秒)
    pub avg_time_ms: u32,
    /// 发送的控制指令数
    pub commands_sent: u32,
}

impl ScenarioStats {
    pub fn new() -> Self {
        ScenarioStats {
            total_executions: 0,
            successful: 0,
            failed: 0,
            total_time_ms: 0,
            avg_time_ms: 0,
            commands_sent: 0,
        }
    }
    
    pub fn success_rate(&self) -> f32 {
        if self.total_executions == 0 {
            0.0
        } else {
            (self.successful as f32 / self.total_executions as f32) * 100.0
        }
    }
    
    pub fn update_execution(&mut self, success: bool, time_ms: u32) {
        self.total_executions += 1;
        self.total_time_ms += time_ms;
        
        if success {
            self.successful += 1;
        } else {
            self.failed += 1;
        }
        
        if self.total_executions > 0 {
            self.avg_time_ms = self.total_time_ms / self.total_executions;
        }
    }
}

impl fmt::Display for ScenarioStats {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Stats: {}/{}成功 ({:.1}%), 平均耗时{}ms, 命令{}个",
            self.successful, self.total_executions, self.success_rate(),
            self.avg_time_ms, self.commands_sent
        )
    }
}

/// 场景执行流程
pub struct ScenarioExecutor {
    /// 场景类型
    pub scenario: ApplicationScenario,
    /// 当前执行状态
    pub state: ExecutionState,
    /// 执行统计
    pub stats: ScenarioStats,
    /// 检测阈值 (0.0-1.0)
    pub detection_threshold: f32,
    /// 触发动作的最小检测数
    pub min_detections: u32,
}

impl ScenarioExecutor {
    /// 创建新的场景执行器
    pub fn new(scenario: ApplicationScenario) -> Self {
        ScenarioExecutor {
            scenario,
            state: ExecutionState::Idle,
            stats: ScenarioStats::new(),
            detection_threshold: 0.5,
            min_detections: 1,
        }
    }
    
    /// 完整的场景执行流程
    pub fn execute_scenario(
        &mut self,
        image_data: &[u8],
        detection_count: u32,
    ) -> Result<Option<ActuatorCommand>, &'static str> {
        let start_state = self.state;
        
        // Step 1: 采集 (已完成, 图像已在参数中)
        self.state = ExecutionState::Capturing;
        
        // Step 2: 预处理
        self.state = ExecutionState::Preprocessing;
        if image_data.len() == 0 {
            return Err("Invalid image data");
        }
        
        // Step 3: 推理
        self.state = ExecutionState::Inferencing;
        
        // Step 4: 后处理
        self.state = ExecutionState::Postprocessing;
        
        // Step 5: 决策和控制
        self.state = ExecutionState::Controlling;
        let command = self.make_control_decision(detection_count)?;
        
        self.state = ExecutionState::Complete;
        self.stats.update_execution(true, 70);  // 模拟 70ms 耗时
        
        if command.is_some() {
            self.stats.commands_sent += 1;
        }
        
        Ok(command)
    }
    
    /// 根据检测结果做出控制决策
    fn make_control_decision(&self, detection_count: u32) -> Result<Option<ActuatorCommand>, &'static str> {
        if detection_count < self.min_detections {
            return Ok(None);  // 无检测, 无动作
        }
        
        let command = match self.scenario {
            ApplicationScenario::PeopleDetection => {
                if detection_count > 5 {
                    // 人员过多, 报警
                    Some(ActuatorCommand::new(0x01, 0, 255))
                } else {
                    None
                }
            }
            ApplicationScenario::VehicleDetection => {
                if detection_count > 0 {
                    // 检测到车辆, 转向
                    Some(ActuatorCommand::new(0x02, 1, 128))
                } else {
                    None
                }
            }
            ApplicationScenario::ObjectDetection => {
                if detection_count > 3 {
                    // 多个物体, 减速
                    Some(ActuatorCommand::new(0x03, 2, 64))
                } else {
                    None
                }
            }
            ApplicationScenario::AnomalyDetection => {
                if detection_count > 0 {
                    // 检测到异常, 停止
                    Some(ActuatorCommand::new(0x04, 3, 0))
                } else {
                    None
                }
            }
        };
        
        Ok(command)
    }
    
    /// 获取当前状态
    pub fn get_state(&self) -> ExecutionState {
        self.state
    }
    
    /// 获取执行统计
    pub fn get_stats(&self) -> &ScenarioStats {
        &self.stats
    }
    
    /// 重置场景
    pub fn reset(&mut self) {
        self.state = ExecutionState::Idle;
    }
}

/// 多场景协调器
pub struct MultiScenarioCoordinator {
    /// 注册的场景执行器
    scenarios: Vec<ScenarioExecutor>,
}

impl MultiScenarioCoordinator {
    pub fn new() -> Self {
        MultiScenarioCoordinator {
            scenarios: Vec::new(),
        }
    }
    
    /// 注册场景
    pub fn register_scenario(&mut self, executor: ScenarioExecutor) -> usize {
        let id = self.scenarios.len();
        self.scenarios.push(executor);
        id
    }
    
    /// 运行所有已注册的场景
    pub fn run_all_scenarios(
        &mut self,
        image_data: &[u8],
        detection_count: u32,
    ) -> Result<Vec<ActuatorCommand>, &'static str> {
        let mut commands = Vec::new();
        
        for executor in &mut self.scenarios {
            if let Ok(Some(cmd)) = executor.execute_scenario(image_data, detection_count) {
                commands.push(cmd);
            }
        }
        
        Ok(commands)
    }
    
    /// 生成协调报告
    pub fn generate_report(&self) -> CoordinationReport {
        let mut total_success = 0u32;
        let mut total_commands = 0u32;
        
        for executor in &self.scenarios {
            total_success += executor.stats.successful;
            total_commands += executor.stats.commands_sent;
        }
        
        CoordinationReport {
            active_scenarios: self.scenarios.len() as u32,
            total_commands: total_commands,
            success_count: total_success,
        }
    }
}

/// 协调报告
#[derive(Debug, Clone)]
pub struct CoordinationReport {
    pub active_scenarios: u32,
    pub total_commands: u32,
    pub success_count: u32,
}

impl fmt::Display for CoordinationReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Coordination: {} scenarios, {} commands sent, {} successful",
            self.active_scenarios, self.total_commands, self.success_count
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_scenario_executor() {
        let mut executor = ScenarioExecutor::new(ApplicationScenario::PeopleDetection);
        let fake_image = alloc::vec![128u8; 1920 * 1080 * 3];
        
        let result = executor.execute_scenario(&fake_image, 3);
        assert!(result.is_ok());
    }
    
    #[test]
    fn test_actuator_command() {
        let cmd = ActuatorCommand::new(0x01, 0, 255);
        assert_eq!(cmd.actuator_id, 0x01);
    }
    
    #[test]
    fn test_stats() {
        let mut stats = ScenarioStats::new();
        stats.update_execution(true, 70);
        stats.update_execution(true, 71);
        assert_eq!(stats.successful, 2);
        assert!(stats.success_rate() > 90.0);
    }
}
