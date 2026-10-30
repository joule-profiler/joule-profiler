use std::path::PathBuf;

use anyhow::{Result, bail};
use joule_profiler_core::JouleProfiler;
use joule_profiler_exporter_csv::CsvExporterConfig;
use joule_profiler_exporter_json::JsonExporterConfig;
use joule_profiler_exporter_terminal::TerminalExporterConfig;
use serde::Deserialize;

use crate::cli::OutputFormat;

/// The `[exporter]` table: `format` picks the exporter, the other keys are the configured exporter configuration.
#[derive(Debug, Deserialize)]
#[serde(tag = "format", rename_all = "lowercase")]
pub enum ExporterConfig {
    Terminal(TerminalExporterConfig),
    Json(JsonExporterConfig),
    Csv(CsvExporterConfig),
}

impl Default for ExporterConfig {
    fn default() -> Self {
        ExporterConfig::Terminal(TerminalExporterConfig::default())
    }
}

impl ExporterConfig {
    /// Replaces the exporter with the default one of `format`, unless it already has that format.
    pub fn set_format(&mut self, format: OutputFormat) {
        *self = match (std::mem::take(self), format) {
            (config @ ExporterConfig::Terminal(_), OutputFormat::Terminal)
            | (config @ ExporterConfig::Json(_), OutputFormat::Json)
            | (config @ ExporterConfig::Csv(_), OutputFormat::Csv) => config,
            (_, OutputFormat::Terminal) => {
                ExporterConfig::Terminal(TerminalExporterConfig::default())
            }
            (_, OutputFormat::Json) => ExporterConfig::Json(JsonExporterConfig::default()),
            (_, OutputFormat::Csv) => ExporterConfig::Csv(CsvExporterConfig::default()),
        };
    }

    /// Sets the file the results are written to.
    pub fn set_file(&mut self, path: PathBuf) -> Result<()> {
        match self {
            ExporterConfig::Terminal(_) => bail!(
                "--output-file needs a format that writes a file, such as `--output-format json`"
            ),
            ExporterConfig::Json(JsonExporterConfig { file, .. })
            | ExporterConfig::Csv(CsvExporterConfig { file }) => {
                *file = Some(path);
            }
        }
        Ok(())
    }
}

/// Sets the configured exporter.
pub fn set_exporter(profiler: &mut JouleProfiler, config: ExporterConfig) -> Result<()> {
    match config {
        ExporterConfig::Terminal(config) => profiler.set_exporter(config.build()),
        ExporterConfig::Json(config) => profiler.set_exporter(config.build()?),
        ExporterConfig::Csv(config) => profiler.set_exporter(config.build()?),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_format_keeps_same_format() {
        let mut config = ExporterConfig::Json(JsonExporterConfig {
            file: None,
            lines: true,
        });

        config.set_format(OutputFormat::Json);
        assert!(matches!(
            config,
            ExporterConfig::Json(JsonExporterConfig { lines: true, .. })
        ));

        config.set_format(OutputFormat::Csv);
        assert!(matches!(
            config,
            ExporterConfig::Csv(CsvExporterConfig { file: None })
        ));
    }

    #[test]
    fn terminal_refuses_file() {
        assert!(ExporterConfig::default().set_file("out".into()).is_err());
    }
}
