use thiserror::Error;

pub type BoxError = Box<dyn std::error::Error + Send + Sync>;

pub type Result<T, E = Error> = std::result::Result<T, E>;

#[derive(Debug, Error)]
pub enum Error {
    #[error("source `{name}` failed: {error}")]
    Source { name: String, error: BoxError },

    #[error("source `{name}` declared {declared} metrics but produced {produced} values")]
    MetricCount {
        name: String,
        declared: usize,
        produced: usize,
    },

    #[error("injector failed: {0}")]
    Injector(BoxError),

    #[error("exporter failed: {0}")]
    Exporter(BoxError),

    #[error("no source to measure with: call `add_source` before `profile`")]
    NoSource,

    #[error("no injector set: call `set_injector` before `profile`")]
    NoInjector,

    #[error("no exporter set: call `set_exporter` before `profile`")]
    NoExporter,

    #[error("could not spawn thread `{name}`: {error}")]
    Spawn { name: String, error: std::io::Error },

    #[error("thread `{0}` panicked")]
    Panicked(String),

    #[error(transparent)]
    Cgroup(#[from] crate::util::cgroup::CgroupError),

    #[error("thread `{0}` ended before it was ready")]
    NotReady(String),
}

impl Error {
    pub(crate) fn injector(error: impl Into<BoxError>) -> Self {
        Self::Injector(error.into())
    }

    pub(crate) fn exporter(error: impl Into<BoxError>) -> Self {
        Self::Exporter(error.into())
    }
}
