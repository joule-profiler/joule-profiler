use std::fmt::{self, Display};

use thiserror::Error;

pub type BoxError = Box<dyn std::error::Error + Send + Sync>;

pub type Result<T, E = Error> = std::result::Result<T, E>;

#[derive(Debug, Error)]
pub enum Error {
    #[error("source `{name}` failed")]
    Source {
        name: String,
        #[source]
        error: BoxError,
    },

    #[error("source `{name}` declared {declared} metrics but produced {produced} values")]
    MetricCount {
        name: String,
        declared: usize,
        produced: usize,
    },

    #[error("injector failed")]
    Injector(#[source] BoxError),

    #[error("exporter failed")]
    Exporter(#[source] BoxError),

    #[error("no source to measure with: call `add_source` before `profile`")]
    NoSource,

    #[error("no injector set: call `set_injector` before `profile`")]
    NoInjector,

    #[error("no exporter set: call `set_exporter` before `profile`")]
    NoExporter,

    #[error("could not spawn thread `{name}`")]
    Spawn {
        name: String,
        #[source]
        error: std::io::Error,
    },

    #[error("thread `{0}` panicked")]
    Panicked(String),

    #[error(transparent)]
    Cgroup(#[from] crate::util::cgroup::CgroupError),

    #[error("thread `{0}` ended before it was ready")]
    NotReady(String),
}

impl Error {
    pub(crate) fn source_failed(name: &str, error: impl Into<BoxError>) -> Self {
        Self::Source {
            name: name.to_owned(),
            error: error.into(),
        }
    }

    pub(crate) fn injector_failed(error: impl Into<BoxError>) -> Self {
        Self::Injector(error.into())
    }

    pub(crate) fn exporter_failed(error: impl Into<BoxError>) -> Self {
        Self::Exporter(error.into())
    }
}

/// Writes an error followed by its causes, as `error: cause: cause`, where nothing can return it
/// and it has to be logged.
pub(crate) struct ErrorChain<'a>(pub(crate) &'a (dyn std::error::Error + 'static));

impl Display for ErrorChain<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)?;
        let mut cause = self.0.source();
        while let Some(error) = cause {
            write!(f, ": {error}")?;
            cause = error.source();
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::io;

    use super::*;

    #[test]
    fn chain_includes_causes() {
        let error = Error::source_failed("rapl", io::Error::other("no such file"));

        assert_eq!(
            ErrorChain(&error).to_string(),
            "source `rapl` failed: no such file"
        );
    }
}
