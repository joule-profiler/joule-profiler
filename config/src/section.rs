use serde::{Deserialize, Serialize};

/// The `[profiler]` section.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ProfilerConfig {
    /// Whether the phases are processed and exported only once the run is over.
    pub defer: bool,
}
