use std::{collections::HashSet, time::Duration};

use crate::UUID;

/// Configuration of the AMD SMI source.
#[derive(Debug, Default)]
pub struct AmdSmiConfig {
    /// Optional background polling interval.
    pub poll_interval: Option<Duration>,

    /// Optional gpus filter.
    pub gpus_spec: Option<HashSet<UUID>>,
}
