//! Output formats for `JouleProfiler`.
//!
//! This module defines the supported output formats and provides utilities
//! for selecting and displaying metrics collected by `JouleProfiler`.
//! It includes built-in formats for terminal display, JSON export, and CSV export.
//!
//! # Overview
//!
//! - [`OutputFormat`] — Enum representing the available output formats.
//! - `csv`, `json`, `terminal` — Submodules implementing the actual display logic for default output formats.

use clap::ValueEnum;
use std::fmt::{Display, Formatter, Result};

use serde::Deserialize;

pub mod csv;
pub mod json;
pub mod terminal;

/// Represents the supported output formats for `JouleProfiler`.
///
/// This enum defines how metrics are displayed or exported. It is used
/// by the profiler to select the appropriate output method based on
/// user preferences or CLI flags.
/// The output format is used to instanciate a [`crate::displayer::Displayer`].
///
/// # Variants
///
/// - `Terminal` - Display metrics directly in the terminal (default).
/// - `Json` - Export metrics as JSON for easy parsing or integration.
/// - `Csv` - Export metrics in CSV format for spreadsheets or analysis.
#[derive(Debug, Clone, Copy, Default, ValueEnum, Deserialize)]
pub enum OutputFormat {
    #[default]
    #[serde(alias = "terminal", alias = "term", alias = "TERM")]
    Terminal,

    #[serde(alias = "json", alias = "JSON")]
    Json,

    #[serde(alias = "csv", alias = "CSV")]
    Csv,
}

impl Display for OutputFormat {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result {
        f.write_str(match self {
            OutputFormat::Terminal => "Terminal",
            OutputFormat::Json => "Json",
            OutputFormat::Csv => "CSV",
        })?;
        Ok(())
    }
}
