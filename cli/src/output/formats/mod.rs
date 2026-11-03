//! Output formats for `JouleProfiler`.

use clap::ValueEnum;
use std::fmt::{Display, Formatter, Result};

use serde::Deserialize;

pub mod csv;
pub mod json;
pub mod markdown;
pub mod terminal;

/// Represents the supported output formats for `JouleProfiler`.
#[derive(Debug, Clone, Copy, Default, ValueEnum, Deserialize)]
pub enum OutputFormat {
    #[default]
    #[serde(alias = "terminal", alias = "term", alias = "TERM")]
    Terminal,

    #[serde(alias = "json", alias = "JSON")]
    Json,

    #[serde(alias = "csv", alias = "CSV")]
    Csv,

    #[serde(alias = "markdown", alias = "md", alias = "MARKDOWN")]
    Markdown,
}

impl Display for OutputFormat {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result {
        f.write_str(match self {
            OutputFormat::Terminal => "Terminal",
            OutputFormat::Json => "Json",
            OutputFormat::Csv => "CSV",
            OutputFormat::Markdown => "Markdown",
        })?;
        Ok(())
    }
}
