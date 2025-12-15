pub mod hmp_scheduler;
pub mod npu_support;

pub use hmp_scheduler::{HmpScheduler, Task, TaskHint, hmp_init, HMP_SCHEDULER};
pub use npu_support::{NpuScheduler, NpuContext, NpuSchedulePolicy, NpuTaskType, NPU_SCHEDULER};
