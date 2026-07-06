use std::time::Duration;

use serde::Deserialize;

use crate::utils::MemoryUnit;

const DEFAULT_POLL_INTERVAL: Duration = Duration::from_millis(10);
const DEFAULT_PROCESS_DETECTION_POLL_INTERVAL: Duration = Duration::from_secs(1);

fn default_poll_interval() -> Duration {
    DEFAULT_POLL_INTERVAL
}

fn default_process_detection_poll_interval() -> Duration {
    DEFAULT_PROCESS_DETECTION_POLL_INTERVAL
}

/// Configuration for the `Procfs` metric source.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct ProcfsConfig {
    /// Polling interval.
    #[serde(default = "default_poll_interval", with = "humantime_serde")]
    pub poll_interval: Duration,

    /// Interval at which the process hierarchy is built from the profiled process PID.
    #[serde(
        default = "default_process_detection_poll_interval",
        with = "humantime_serde"
    )]
    pub process_detection_poll_interval: Duration,

    /// Memory unit for process metrics.
    pub proc_memory_unit: MemoryUnit,

    /// Memory unit for global metrics.
    pub global_memory_unit: MemoryUnit,
}

impl Default for ProcfsConfig {
    fn default() -> Self {
        Self {
            poll_interval: DEFAULT_POLL_INTERVAL,
            process_detection_poll_interval: DEFAULT_PROCESS_DETECTION_POLL_INTERVAL,
            proc_memory_unit: MemoryUnit::Bytes,
            global_memory_unit: MemoryUnit::Bytes,
        }
    }
}
