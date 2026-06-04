use std::{collections::HashSet, time::Duration};

use crate::UUID;

/// Configuration of the NVML source.
#[derive(Debug, Default)]
pub struct NvmlConfig {
    /// Optional background polling interval.
    pub poll_interval: Option<Duration>,

    /// Optional gpus filter.
    pub gpus_spec: Option<HashSet<UUID>>,
}
