use std::time::Duration;

use crate::utils::MemoryUnit;

/// Configuration for the `Procfs` metric source.
#[derive(Debug, Clone)]
pub struct ProcfsConfig {
    /// Polling interval. If `None`, metrics are only captured on phase boundaries.
    pub poll_interval: Option<Duration>,

    /// Memory unit for process metrics (default: `Mega`).
    pub proc_memory_unit: MemoryUnit,

    /// Memory unit for global metrics (default: `Giga`).
    pub global_memory_unit: MemoryUnit,
}

impl Default for ProcfsConfig {
    fn default() -> Self {
        Self {
            poll_interval: None,
            proc_memory_unit: MemoryUnit::Mega,
            global_memory_unit: MemoryUnit::Giga,
        }
    }
}
