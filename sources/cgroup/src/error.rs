use std::{num::ParseIntError, path::PathBuf};

use thiserror::Error;
use tokio::task::JoinError;

use crate::cgroup::Controller;

/// Main error type for all cgroup-related operations.
#[derive(Debug, Error)]
pub enum CgroupError {
    /// cgroup v2 is not available or not mounted as a unified hierarchy.
    #[error("CGroup v2 not available at `{0}` - is the unified hierarchy mounted?")]
    NotAvailable(String),

    /// Attempted to create a cgroup that already exists.
    #[error("Cgroup with name \"{0}\" already created.")]
    AlreadyCreated(String),

    /// Failed to enable a controller in `cgroup.subtree_control`.
    #[error("Failed to create controller `{controller}` on path `{path}`, cause: {source}")]
    FailedToCreateController {
        controller: Controller,
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    /// Failed to attach a PID to a cgroup.
    #[error("Failed to attach PID `{pid}` to cgroup on path `{path}`, cause: {source}")]
    FailedToAttachPid {
        pid: i32,
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    /// A metric expected to always exist in kernel stats was missing.
    #[error("Missing always present metric `{0}`")]
    MissingAlwaysPresentMetric(&'static str),

    /// Generic I/O error.
    #[error("I/O error")]
    Io(
        #[from]
        #[source]
        std::io::Error,
    ),

    /// I/O error tied to a specific file path.
    #[error("I/O error on `{path}`: {source}")]
    IoPath {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    /// Failed to parse a numeric value from a cgroup file.
    #[error("Failed to parse `{path}`: {source}")]
    Parse {
        path: PathBuf,
        #[source]
        source: ParseIntError,
    },

    /// Tokio task join error (async execution failure).
    #[error("Failed to join tokio task: {0}")]
    JoinError(
        #[from]
        #[source]
        JoinError,
    ),
}
