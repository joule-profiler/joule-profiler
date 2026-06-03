use std::time::Duration;

use crate::UUID;

#[derive(Debug, Default)]
pub struct AmdSmiConfig {
    /// Optional background polling interval.
    pub poll_interval: Option<Duration>,

    /// Optional gpus filter.
    pub gpus_spec: Option<Vec<UUID>>,
}
