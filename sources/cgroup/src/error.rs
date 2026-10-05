use std::io;
use std::path::PathBuf;

use thiserror::Error;

pub type Result<T> = std::result::Result<T, CgroupError>;

#[derive(Debug, Error)]
pub enum CgroupError {
    #[error(
        "the cgroup {0} does not exist: make it beforehand, or let the profiler make one with a `[cgroup]` table"
    )]
    NotFound(PathBuf),

    #[error("reading {0}")]
    Read(PathBuf, #[source] io::Error),

    #[error("the cgroup {0} reports none of the values this source reads")]
    NothingToRead(PathBuf),

    #[error("the source was never attached to a process")]
    NotAttached,
}
