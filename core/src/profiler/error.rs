use crate::orchestrator::error::OrchestratorError;
use crate::source::error::MetricSourceError;
use thiserror::Error;
use tokio::task::JoinError;

/// Top-level error type for `JouleProfiler`.
///
/// Represents all possible errors that can occur during profiler execution,
/// from initialization and orchestration to source errors and aggregation.
#[derive(Debug, Error)]
pub enum JouleProfilerError {
    /// The profiled command failed during execution.
    #[error("Failed to execute command: {0}")]
    CommandExecutionFailed(String),

    /// The profiled command binary was not found on the system.
    #[error("Command not found: {0}")]
    CommandNotFound(String),

    /// The output file could not be created at the given path.
    #[error("Failed to create output file: {0}")]
    OutputFileCreationFailed(String),

    /// The provided token pattern is not a valid regular expression.
    #[error("Invalid regex pattern: {0}")]
    InvalidPattern(String),

    /// Failed to capture the profiled command's stdout.
    #[error("Stdout capture failed")]
    StdOutCaptureFail,

    /// Failed to capture the profiled command's stderr.
    #[error("Stderr capture failed")]
    StdErrCaptureFail,

    /// Failed to join the stderr tokio task handle.
    #[error("Join error")]
    StdErrHandleJoinError(
        #[from]
        #[source]
        JoinError,
    ),

    /// Failed to retrieve `SUDO_USER` environment variable.
    #[error(
        "Cannot retrieve SUDO_USER environment variable, please retry without root privileges or use \"--use-root\" CLI argument."
    )]
    CannotRetrieveSudoUser,

    /// The environment variable exists but is not parsable into the wanted format.
    #[error("Unable to parse {0} environment variable.")]
    CannotParseEnvVariable(String),

    /// Unable to retrieve the current effective user id by it's name with the `getpwnam` libc function.
    #[error("Unable to retrieve current user id by it's name.")]
    CannotRetrieveCurrentUserId,

    /// Cannot convert string to a known metric unit.
    #[error("Invalid metric unit: {0}")]
    InvalidUnit(String),

    /// Generic I/O error.
    #[error("I/O error")]
    IoError(
        #[from]
        #[source]
        std::io::Error,
    ),

    /// A process control operation (e.g. kill, wait) failed.
    #[error("Process control failed: {0}")]
    ProcessControlFailed(String),

    /// Error propagated from a metric source.
    #[error(transparent)]
    MetricSourceError(#[from] MetricSourceError),

    /// Error propagated from the source orchestrator.
    #[error(transparent)]
    OrchestratorError(#[from] OrchestratorError),
}
