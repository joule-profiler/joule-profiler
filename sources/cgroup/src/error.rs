use thiserror::Error;

#[derive(Debug, Error)]
pub enum CgroupError {
    #[error("CGroup v2 not available at `{0}` — is the unified hierarchy mounted?")]
    NotAvailable(String),

    #[error("I/O error on `{path}`: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },

    #[error("Failed to parse `{path}`: expected {expected}, got `{got}`")]
    Parse {
        path: String,
        expected: &'static str,
        got: String,
    },

    #[error("Failed to write PID {pid} to cgroup.procs: {source}")]
    Attach {
        pid: i32,
        #[source]
        source: std::io::Error,
    },

    #[error("Failed to enable cgroup controller `{controller}` in `{path}`: {source}")]
    EnableController {
        controller: &'static str,
        path: String,
        #[source]
        source: std::io::Error,
    },
}
