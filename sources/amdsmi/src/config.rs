use std::{collections::HashSet, time::Duration};

use serde::Deserialize;

use crate::UUID;

const DEFAULT_POLL_INTERVAL: Duration = Duration::from_millis(20);

fn default_poll_interval() -> Duration {
    DEFAULT_POLL_INTERVAL
}

/// Configuration of the AMD SMI source.
#[derive(Debug, Deserialize)]
pub struct AmdSmiConfig {
    /// Optional background polling interval.
    #[serde(default = "default_poll_interval", with = "humantime_serde")]
    pub poll_interval: Duration,

    /// Optional gpus filter.
    pub gpus_spec: Option<HashSet<UUID>>,
}

impl Default for AmdSmiConfig {
    fn default() -> Self {
        Self {
            poll_interval: DEFAULT_POLL_INTERVAL,
            gpus_spec: None,
        }
    }
}
