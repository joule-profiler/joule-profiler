use std::{collections::HashSet, time::Duration};

use crate::UUID;

/// Configuration of the NVML source.
#[derive(Debug)]
pub struct NvmlConfig {
    /// Optional background polling interval.
    pub poll_interval: Option<Duration>,

    /// Optional gpus filter.
    pub gpus_spec: Option<HashSet<UUID>>,
}

impl Default for NvmlConfig {
    fn default() -> Self {
        Self {
            poll_interval: Some(Duration::from_millis(50)),
            gpus_spec: None,
        }
    }
}
