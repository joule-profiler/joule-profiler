//! The results of a run as one table, a row per metric.

use std::{
    fs::File,
    io,
    path::{Path, PathBuf},
};

use csv::{QuoteStyle, Writer, WriterBuilder};
use joule_profiler_core::{
    Exporter, PhaseInfo, SourceMetrics, Summary,
    util::fs::{create_file, default_results_filename},
};
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum CsvError {
    #[error("creating {0}")]
    Create(PathBuf, #[source] io::Error),

    #[error("writing the results")]
    Write(#[from] csv::Error),

    #[error("flushing the results")]
    Flush(#[from] io::Error),
}

type Result<T> = std::result::Result<T, CsvError>;

/// Configuration of the CSV exporter.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct CsvExporterConfig {
    /// The file the results are written to. Defaults to `data<timestamp>.csv`.
    pub file: Option<PathBuf>,
}

impl CsvExporterConfig {
    pub fn build(self) -> Result<CsvExporter> {
        match self.file {
            Some(path) => CsvExporter::to_file(path),
            None => CsvExporter::new(),
        }
    }
}

const DELIMITER: u8 = b';';

const COLUMNS: [&str; 12] = [
    "phase_index",
    "phase_name",
    "start_token",
    "end_token",
    "start_token_line",
    "end_token_line",
    "start_timestamp",
    "duration_us",
    "source",
    "metric",
    "value",
    "unit",
];

/// Writes one row per metric, as each phase ends.
pub struct CsvExporter {
    out: Writer<File>,

    path: PathBuf,

    started: bool,
}

impl CsvExporter {
    /// Writes to `data<timestamp>.<ext>`.
    pub fn new() -> Result<Self> {
        Self::to_file(default_results_filename("csv"))
    }

    pub fn to_file(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let file = create_file(path).map_err(|error| CsvError::Create(path.into(), error))?;

        Ok(Self {
            out: WriterBuilder::new()
                .delimiter(DELIMITER)
                .quote_style(QuoteStyle::Necessary)
                .from_writer(file),
            path: path.into(),
            started: false,
        })
    }

    fn start(&mut self) -> Result<()> {
        if !self.started {
            self.out.write_record(COLUMNS)?;
            self.started = true;
        }

        Ok(())
    }
}

impl Exporter for CsvExporter {
    type Error = CsvError;

    fn export(&mut self, phase: &PhaseInfo, sources: &[SourceMetrics<'_>]) -> Result<()> {
        self.start()?;

        let index = phase.index.to_string();
        let name = phase.name();
        let start_line = line(phase.start_line);
        let end_line = line(phase.end_line);
        let timestamp = phase.timestamp_us.to_string();
        let duration = phase.duration_us.to_string();

        for source in sources {
            for metric in &source.metrics {
                self.out.write_record([
                    index.as_str(),
                    name.as_str(),
                    &phase.start_token,
                    &phase.end_token,
                    start_line.as_str(),
                    end_line.as_str(),
                    timestamp.as_str(),
                    duration.as_str(),
                    source.name,
                    metric.name,
                    &metric.value.to_string(),
                    &metric.unit.to_string(),
                ])?;
            }
        }

        Ok(())
    }

    fn flush(&mut self) -> Result<()> {
        Ok(self.out.flush()?)
    }

    fn finish(&mut self, _summary: &Summary) -> Result<()> {
        println!("CSV written to: {}", self.path.display());
        Ok(())
    }
}

/// An unknown line is written as an empty cell.
fn line(line: Option<usize>) -> String {
    line.map(|line| line.to_string()).unwrap_or_default()
}
