//! The results of a run as one table, a row per metric.

use std::fs::File;
use std::io;
use std::path::{Path, PathBuf};

use csv::{QuoteStyle, Writer, WriterBuilder};
use joule_profiler_core::exporter::Exporter;
use joule_profiler_core::phase::{PhaseInfo, SourceMetrics, Summary};
use joule_profiler_core::util::fs::{create_file, default_results_filename};
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

const DELIMITER: u8 = b';';

const COLUMNS: [&str; 12] = [
    "phase_index",
    "phase_name",
    "start_token",
    "end_token",
    "start_token_line",
    "end_token_line",
    "start_timestamp",
    "duration_ms",
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
        let file = create_file(path).map_err(|e| CsvError::Create(path.into(), e))?;

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

        // Formatted once per phase, not once per row.
        let index = phase.index.to_string();
        let name = phase.name();
        let start_line = line(phase.start_line);
        let end_line = line(phase.end_line);
        let timestamp = phase.timestamp_us.to_string();
        let duration = phase.duration_ms.to_string();

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
