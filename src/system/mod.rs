pub mod integration;
pub mod scenarios;

pub use integration::{
    SystemIntegrationManager, ComponentInfo, ComponentStatus, SubsystemType,
    HealthCheckReport, BenchmarkData, SystemReport, SYSTEM_MANAGER
};

pub use scenarios::{
    ScenarioExecutor, ApplicationScenario, ActuatorCommand,
    ExecutionState, ScenarioStats, MultiScenarioCoordinator,
    CoordinationReport
};
