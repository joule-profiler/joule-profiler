//! Error types for the procfs metric source.

use thiserror::Error;

/// Errors that can occur while reading metrics from `/proc`.
#[derive(Error, Debug)]
pub enum ProcfsError {
    /// The source was used before [`crate::Procfs::init`] was called.
    #[error("Procfs not initialized, call init first")]
    NotInitialized,

    /// An error from the `procfs` crate (e.g., process not found, parse failure).
    #[error(transparent)]
    Procfs(#[from] procfs::ProcError),

    /// An I/O error reading from `/proc` directly (e.g., `/proc/{pid}/task/{tid}/children`).
    #[error(transparent)]
    Io(#[from] std::io::Error),
}
