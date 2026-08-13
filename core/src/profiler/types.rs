use crate::JouleProfilerError;
use crate::aggregate::Metrics;
use crate::orchestrator::Orchestrator;
use crate::phase::{PhaseInfo, PhaseToken};
use serde::Serialize;

/// Result type for profiler operations.
pub type Result<T> = std::result::Result<T, JouleProfilerError>;

/// Represents a profiling phase with metrics and timing.
#[derive(Debug, Serialize)]
pub struct Phase {
    /// The index of the phase.
    pub index: usize,

    /// Token marking the start of the phase.
    pub start_token: PhaseToken,

    /// Token marking the end of the phase.
    pub end_token: PhaseToken,

    /// Start timestamp in microsecond.
    pub timestamp: u128,

    /// Duration of the phase in millisecond.
    pub duration_ms: u128,

    /// Optional start line number associated with the phase.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_token_line: Option<usize>,

    /// Optional end line number associated with the phase.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end_token_line: Option<usize>,

    /// Metrics collected during the phase.
    pub metrics: Metrics,
}

impl Phase {
    pub fn get_name(&self) -> String {
        format!("{} -> {}", self.start_token, self.end_token)
    }
}

pub type Phases = Vec<Phase>;

/// Represents the results of a program's profiling.
#[derive(Debug, Serialize)]
pub struct ProfilerResults {
    /// Timestamp of the first measure in microsecond.
    pub timestamp: u128,

    /// Duration of the program in millisecond.
    pub duration_ms: u128,

    /// Exit code of the profiled command.
    pub exit_code: i32,

    /// Phases detected in the program's standard output.
    pub phases: Phases,
}

/// Phases and begin/end timestamps collected by the reader thread.
pub(crate) struct MeasureData {
    pub phases: Vec<PhaseInfo>,
    pub begin_timestamp: u128,
    pub end_timestamp: u128,
}

/// What the reader thread sends back once done: the orchestrator (to reuse
/// its sources) alongside the measurement outcome.
pub(crate) type ReaderResult = (Orchestrator, Result<MeasureData>);
