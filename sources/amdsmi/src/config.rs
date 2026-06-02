use std::time::Duration;

#[derive(Debug, Default)]
pub struct AmdSmiConfig {
    /// Optional background polling interval.
    pub poll_interval: Option<Duration>,
}
