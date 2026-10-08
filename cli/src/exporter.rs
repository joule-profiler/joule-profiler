use anyhow::Result;
use joule_profiler_core::profiler::JouleProfiler;
use joule_profiler_exporter_csv::CsvExporter;
use joule_profiler_exporter_json::{JsonExporter, Shape};
use joule_profiler_exporter_terminal::TerminalExporter;

use crate::cli::OutputFormat;
use crate::config::ConfigTable;

/// Sets the exporter of the configured format, and logs where its file is.
pub fn set(profiler: &mut JouleProfiler, config_table: &ConfigTable) -> Result<()> {
    let file = config_table.output_file.as_deref();

    match config_table.output_format {
        OutputFormat::Terminal => profiler.set_exporter(TerminalExporter::default()),

        OutputFormat::Json => {
            let shape = if config_table.lines {
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
