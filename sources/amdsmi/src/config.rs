use std::{collections::HashSet, time::Duration};

use crate::UUID;

/// Configuration of the AMD SMI source.
#[derive(Debug)]
pub struct AmdSmiConfig {
    /// Optional background polling interval.
    pub poll_interval: Option<Duration>,

    /// Optional gpus filter.
    pub gpus_spec: Option<HashSet<UUID>>,
}

impl Default for AmdSmiConfig {
    fn default() -> Self {
        Self {
            poll_interval: Some(Duration::from_millis(20)),
            gpus_spec: None,
        }
    }
}
