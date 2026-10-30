use std::{error::Error, io, path::PathBuf};

use thiserror::Error;

pub(crate) type Result<T> = std::result::Result<T, ConfigError>;

/// An error returned while reading a configuration.
#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("reading {0}")]
    Read(PathBuf, #[source] io::Error),

    #[error("parsing {0}")]
    Parse(PathBuf, #[source] Box<dyn Error + Send + Sync>),

    #[error("{0} is in no configuration format this build reads")]
    UnknownFormat(PathBuf),

    #[error("expected `key=value`, got `{0}`")]
    Definition(String),

    #[error("`{0}` is not a valid key")]
    Key(String),

    #[error("`{0}` is a value, not a section")]
    NotATable(String),

    #[error("reading the configuration")]
    Invalid(#[source] serde_json::Error),

    #[error("unknown source `{name}`, expected one of: {known}")]
    UnknownSource { name: String, known: String },

    #[error("reading the `{0}` table")]
    SourceTable(&'static str, #[source] serde_json::Error),

    #[error("building the `{0}` source")]
    Unavailable(&'static str, #[source] Box<dyn Error + Send + Sync>),
}
