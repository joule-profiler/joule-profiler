use crate::source::MetricSourceError;
use crate::source::types::SourceEvent;
use thiserror::Error;
use tokio::{sync::mpsc::error::SendError, task::JoinError};

/// Errors that can occur during orchestration.
#[derive(Debug, Error)]
pub enum OrchestratorError {
    /// Returned when there's no metric sources configured to profile the program with.
    #[error("No metric sources configured.")]
    NoSourceConfigured,

    /// Happens when an error occur while joining sources.
    #[error("Join error")]
    JoinError(
        #[from]
        #[source]
        JoinError,
    ),

    /// Returned when an error occur when sending an event to the sources channel.
    #[error("Send error")]
    SendError(
        #[from]
        #[source]
        SendError<SourceEvent>,
    ),

    /// Returned when an error occured while sending the initialization event.
    #[error("Cannot initialize source: {0}.")]
    InitializationError(&'static str),

    /// Phase count mismatch between two sensor sources during aggregation.
    #[error(
        "Cannot aggregate sensor results: phase count mismatch (lhs={lhs}, rhs={rhs}). \
        Sources must produce the same number of phases."
    )]
    PhaseMismatch { lhs: usize, rhs: usize },

    /// All sources returned zero phases.
    #[error("All sensor sources produced zero phases.")]
    AllSourcesEmpty,

    /// No results to merge.
    #[error("No sensor results to merge.")]
    NoSensorResults,

    /// An error thrown by a metric source.
    #[error(transparent)]
    MetricSourceError(#[from] MetricSourceError),
}
