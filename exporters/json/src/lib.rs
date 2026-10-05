//! The results of a run as JSON: one document, or one object per line.

use std::fs::File;
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};

use joule_profiler_core::exporter::Exporter;
use joule_profiler_core::metric::{Metric, MetricValue};
use joule_profiler_core::phase::{PhaseInfo, SourceMetrics, Summary};
use joule_profiler_core::unit::MetricUnit;
use joule_profiler_core::util::fs::{create_file, default_results_filename};
use serde::Serialize;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum JsonError {
    #[error("creating {0}")]
    Create(PathBuf, #[source] io::Error),

    #[error("writing the results")]
    Write(#[from] io::Error),

    #[error("encoding the results")]
    Encode(#[from] serde_json::Error),
}

type Result<T> = std::result::Result<T, JsonError>;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum Shape {
    /// One document, written at the end of the run.
    #[default]
    Document,

    /// One object per line, written as the run goes.
    Lines,
}

impl Shape {
    pub fn extension(self) -> &'static str {
        match self {
            Shape::Document => "json",
            Shape::Lines => "jsonl",
        }
    }
}

#[derive(Serialize)]
struct Source<'a> {
    source: &'a str,
    metrics: &'a [Metric<'a>],
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "lowercase")]
enum Line<'a> {
    Phase {
        phase: &'a PhaseInfo,
        sources: Vec<Source<'a>>,
    },

    Summary {
        summary: &'a Summary,
    },
}

#[derive(Default, Serialize)]
struct Document {
    phases: Vec<Owned>,

    #[serde(skip_serializing_if = "Option::is_none")]
    summary: Option<Summary>,
}

/// A phase kept until the end of the run, so it owns its names.
#[derive(Serialize)]
struct Owned {
    phase: PhaseInfo,
    sources: Vec<OwnedSource>,
}

#[derive(Serialize)]
struct OwnedSource {
    source: String,
    metrics: Vec<OwnedMetric>,
}

#[derive(Serialize)]
struct OwnedMetric {
    name: String,
    value: MetricValue,
    unit: MetricUnit,
}

pub struct JsonExporter {
    out: BufWriter<File>,

    path: PathBuf,

    shape: Shape,

    /// The phases so far, for [`Shape::Document`].
    document: Document,
}

impl JsonExporter {
    /// Writes to `data<timestamp>.<ext>`.
    pub fn new(shape: Shape) -> Result<Self> {
        Self::to_file(default_results_filename(shape.extension()), shape)
    }

    pub fn to_file(path: impl AsRef<Path>, shape: Shape) -> Result<Self> {
        let path = path.as_ref();
        let file = create_file(path).map_err(|e| JsonError::Create(path.into(), e))?;

        Ok(Self {
            out: BufWriter::new(file),
            path: path.into(),
            shape,
            document: Document::default(),
        })
    }

    fn write(&mut self, value: &impl Serialize) -> Result<()> {
        match self.shape {
            Shape::Lines => serde_json::to_writer(&mut self.out, value)?,
            Shape::Document => serde_json::to_writer_pretty(&mut self.out, value)?,
        }

        self.out.write_all(b"\n")?;
        self.out.flush()?;
        println!("JSON written to: {}", self.path.display());
        Ok(())
    }
}

fn named<'a>(sources: &'a [SourceMetrics<'_>]) -> Vec<Source<'a>> {
    sources
        .iter()
        .map(|source| Source {
            source: source.name,
            metrics: &source.metrics,
        })
        .collect()
}

impl Exporter for JsonExporter {
    type Error = JsonError;

    fn export(&mut self, phase: &PhaseInfo, sources: &[SourceMetrics<'_>]) -> Result<()> {
        if self.shape == Shape::Lines {
            return self.write(&Line::Phase {
                phase,
                sources: named(sources),
            });
        }

        self.document.phases.push(Owned {
            phase: phase.clone(),
            sources: sources
                .iter()
                .map(|source| OwnedSource {
                    source: source.name.to_owned(),
                    metrics: source
                        .metrics
                        .iter()
                        .map(|metric| OwnedMetric {
                            name: metric.name.to_owned(),
                            value: metric.value,
                            unit: metric.unit,
                        })
                        .collect(),
                })
                .collect(),
        });

        Ok(())
    }

    fn flush(&mut self) -> Result<()> {
        Ok(self.out.flush()?)
    }

    /// A document is written only here, so a failed run leaves the file empty.
    fn finish(&mut self, summary: &Summary) -> Result<()> {
        if self.shape == Shape::Lines {
            return self.write(&Line::Summary { summary });
        }

        self.document.summary = Some(*summary);

        let document = std::mem::take(&mut self.document);
        self.write(&document)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn phase(index: usize) -> PhaseInfo {
        PhaseInfo {
            index,
            start_token: "START".to_owned(),
            end_token: "END".to_owned(),
            start_line: None,
            end_line: Some(3),
            timestamp_us: 10,
            duration_ms: 2,
        }
    }

    fn run(shape: Shape) -> String {
        let directory = tempfile::tempdir().unwrap();
        let path = directory
            .path()
            .join(format!("results.{}", shape.extension()));
        let mut exporter = JsonExporter::to_file(&path, shape).unwrap();
        let sources = [SourceMetrics {
            name: "perf_event",
            metrics: vec![Metric {
                name: "INSTRUCTIONS",
                value: MetricValue::U64(42),
                unit: MetricUnit::COUNT,
            }],
        }];

        exporter.export(&phase(0), &sources).unwrap();
        exporter.export(&phase(1), &sources).unwrap();
        exporter.finish(&Summary::default()).unwrap();

        std::fs::read_to_string(path).unwrap()
    }

    #[test]
    fn lines_are_one_object_per_phase_then_the_summary() {
        let written = run(Shape::Lines);
        let lines: Vec<serde_json::Value> = written
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();

        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0]["type"], "phase");
        assert_eq!(lines[1]["sources"][0]["source"], "perf_event");
        assert_eq!(lines[1]["sources"][0]["metrics"][0]["value"], 42);
        assert_eq!(lines[2]["type"], "summary");
    }

    #[test]
    fn a_document_is_written_once_the_run_is_over() {
        let document: serde_json::Value = serde_json::from_str(&run(Shape::Document)).unwrap();

        assert_eq!(document["phases"].as_array().unwrap().len(), 2);
        assert_eq!(document["phases"][1]["phase"]["index"], 1);
        assert_eq!(
            document["phases"][0]["sources"][0]["metrics"][0]["unit"],
            "count"
        );
        assert!(document["summary"].is_object());
    }
}
