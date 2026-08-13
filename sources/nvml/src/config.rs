use std::{collections::HashSet, time::Duration};

use serde::Deserialize;

const DEFAULT_POLL_INTERVAL: Duration = Duration::from_millis(50);

fn default_poll_interval() -> Duration {
    DEFAULT_POLL_INTERVAL
}

/// Configuration of the NVML source.
#[derive(Debug, Deserialize)]
pub struct NvmlConfig {
    /// Optional background polling interval.
    #[serde(default = "default_poll_interval", with = "humantime_serde")]
    pub poll_interval: Duration,

    /// Optional gpus filter.
    pub gpus_spec: Option<HashSet<u32>>,
}

impl Default for NvmlConfig {
    fn default() -> Self {
        Self {
            poll_interval: DEFAULT_POLL_INTERVAL,
            gpus_spec: None,
        }
    }
}
