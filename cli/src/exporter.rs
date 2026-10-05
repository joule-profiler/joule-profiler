use anyhow::Result;
use joule_profiler_core::profiler::JouleProfiler;
use joule_profiler_exporter_csv::CsvExporter;
use joule_profiler_exporter_json::{JsonExporter, Shape};
use joule_profiler_exporter_terminal::TerminalExporter;

use crate::cli::OutputFormat;
use crate::config::Settings;

/// Sets the exporter of the configured format, and logs where its file is.
pub fn set(profiler: &mut JouleProfiler, settings: &Settings) -> Result<()> {
    let file = settings.output_file.as_deref();

    match settings.output_format {
        OutputFormat::Terminal => profiler.set_exporter(TerminalExporter::default()),

        OutputFormat::Json => {
            let shape = if settings.lines {
                Shape::Lines
            } else {
                Shape::Document
            };

            let exporter = match file {
                Some(path) => JsonExporter::to_file(path, shape)?,
                None => JsonExporter::new(shape)?,
            };
            profiler.set_exporter(exporter);
        }

        OutputFormat::Csv => {
            let exporter = match file {
                Some(path) => CsvExporter::to_file(path)?,
                None => CsvExporter::new()?,
            };
            profiler.set_exporter(exporter);
        }
    }

    Ok(())
}
