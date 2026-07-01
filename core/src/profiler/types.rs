use crate::JouleProfilerError;
use serde::Serialize;

/// Result type for profiler operations.
pub type Result<T> = std::result::Result<T, JouleProfilerError>;

// pub type MeasurePhasesReturnType = (u128, u128, i32, Vec<PhaseInfo>);

// /// Represents a profiling phase with metrics and timing.
// #[derive(Debug, Serialize)]
// pub struct Phase {
//     /// The index of the phase.
//     pub index: usize,

//     /// Token marking the start of the phase.
//     pub start_token: PhaseToken,

//     /// Token marking the end of the phase.
//     pub end_token: PhaseToken,

//     /// Start timestamp in microsecond.
//     pub timestamp: u128,

//     /// Duration of the phase in millisecond.
//     pub duration_ms: u128,

//     /// Optional start line number associated with the phase.
//     #[serde(skip_serializing_if = "Option::is_none")]
//     pub start_token_line: Option<usize>,

//     /// Optional end line number associated with the phase.
//     #[serde(skip_serializing_if = "Option::is_none")]
//     pub end_token_line: Option<usize>,

//     /// Metrics collected during the phase.
//     pub metrics: Metrics,
// }

// impl Phase {
//     pub fn get_name(&self) -> String {
//         format!("{} -> {}", self.start_token, self.end_token)
//     }
// }

// pub type Phases = Vec<Phase>;

/// Represents the results of a program's profiling.
#[derive(Debug, Serialize)]
pub struct ProfilerResults {
    /// Timestamp of the first measure in microsecond.
    pub timestamp: u128,

    /// Duration of the program in millisecond.
    pub duration_ms: u128,
}
