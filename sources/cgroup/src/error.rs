use std::{num::ParseIntError, path::PathBuf};

use thiserror::Error;
use tokio::task::JoinError;

use crate::cgroup::Controller;

#[derive(Debug, Error)]
pub enum CgroupError {
    #[error("CGroup v2 not available at `{0}` - is the unified hierarchy mounted?")]
    NotAvailable(String),

    #[error("Cgroup with name \"{0}\" already created.")]
    AlreadyCreated(String),

    #[error("Failed to create controller `{controller}` on path `{path}`, cause: {source}")]
    FailedToCreateController {
        controller: Controller,
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("Failed to attach pid `{pid}` to cgroup on path `{path}`, cause: {source}")]
    FailedToAttachPid {
        pid: i32,
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("Missing always present metric `{0}`")]
    MissingAlwaysPresentMetric(&'static str),

    #[error("I/O error")]
    Io(
        #[from]
        #[source]
        std::io::Error,
    ),

    #[error("I/O error on `{path}`: {source}")]
    IoPath {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("Failed to parse `{path}`: {source}")]
    Parse {
        path: PathBuf,
        #[source]
        source: ParseIntError,
    },

    #[error("Failed to join tokio task: {0}")]
    JoinError(
        #[from]
        #[source]
        JoinError,
    ),
}
