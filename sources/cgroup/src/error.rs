use std::io;
use std::path::PathBuf;

use thiserror::Error;

pub(crate) type Result<T> = std::result::Result<T, CgroupError>;

/// An error returned by the cgroup source.
#[derive(Debug, Error)]
pub enum CgroupError {
    #[error("the cgroup {0} does not exist")]
    NotFound(PathBuf),

    #[error("{0} is not a cgroup v2")]
    NotACgroup(PathBuf),

    #[error("reading {0}")]
    Read(PathBuf, #[source] io::Error),

    #[error("the source was never attached to a process")]
    NotAttached,
}
