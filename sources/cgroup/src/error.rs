use std::{
    io::ErrorKind,
    num::ParseIntError,
    path::{Path, PathBuf},
};

use thiserror::Error;
use tokio::task::JoinError;

/// Main error type for all cgroup-related operations.
#[derive(Debug, Error)]
pub enum CgroupError {
    /// cgroup v2 is not available or not mounted as a unified hierarchy.
    #[error("Cgroup v2 not available at `{0}`. Check if the unified hierarchy is mounted.")]
    NotAvailable(String),

    /// Attempted to create a cgroup that already exists.
    #[error("Cgroup with name \"{0}\" already created.")]
    AlreadyCreated(String),

    /// Failed to attach a PID to a cgroup.
    #[error("Failed to attach PID `{pid}` to cgroup on path `{path}`. Cause: {source}")]
    FailedToAttachPid {
        pid: i32,
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    /// A cgroup controller is not enabled in `cgroup.subtree_control`.
    #[error(
        "Cgroup controller `{controller}` is not enabled for the parent cgroup. \
        Enable it with: echo \"+{controller}\" | sudo tee {path}/cgroup.subtree_control"
    )]
    ControllerNotEnabled {
        controller: &'static str,
        path: PathBuf,
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

    #[error("Permission denied. Cause: {0}")]
    PermissionDenied(&'static str),

    /// Tokio task join error (async execution failure).
    #[error("Failed to join tokio task: {0}")]
    JoinError(
        #[from]
        #[source]
        JoinError,
    ),
}

impl CgroupError {
    pub(crate) fn into_controller_error(self, controller: &'static str, cgroup: &Path) -> Self {
        if let CgroupError::IoPath { ref source, .. } = self {
            if source.kind() == ErrorKind::NotFound {
                if let Some(parent) = cgroup.parent() {
                    return CgroupError::ControllerNotEnabled {
                        controller,
                        path: parent.to_path_buf(),
                    };
                }
            }
        }
        self
    }
}
