//! Lets a program measure itself: it spawns a profiler and sends it its phases.
//!
//! ```text
//!  program (Session)                       profiler (IpcInjector + ResultsWriter)
//!    spawn                       ────────▶  detaches
//!    {"Configure": "…"}          ────────▶  builds its sources and attaches them
//!                                ◀── 'S' ─  resume(): measuring
//!    {"Phase": "load"}           ────────▶  next_phase(): a boundary
//!                                ◀── 'D' ─  the sensors are woken
//!    {"End": 0}                  ────────▶  next_phase(): no more phases
//!    results, from the pipe      ◀────────  schema, phases, summary (or an error)
//! ```

mod profiler;
mod program;

use joule_profiler_core::metric::MetricValue;
use joule_profiler_core::phase::{PhaseInfo, SourceMetrics, Summary};
use joule_profiler_core::schema::Schema;
use serde::{Deserialize, Serialize, Serializer};
use thiserror::Error;

pub use crate::profiler::{IpcInjector, ResultsWriter, serve_spawned};
pub use crate::program::{Results, Session};

#[derive(Debug, Error)]
pub enum IpcError {
    #[error("the profiler failed: {0}")]
    Profiler(String),

    #[error("the profiler stopped measuring")]
    Closed,

    #[error("the other end does not follow the protocol")]
    Protocol,

    #[error("the profiler process failed: {0}")]
    Spawned(std::process::ExitStatus),

    #[error("this process was not started by `Session::spawn`: {0} is not set")]
    NotSpawned(&'static str),

    #[error(transparent)]
    Io(#[from] std::io::Error),
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
enum Request {
    /// Always first.
    Configure(String),
    Phase(String),
    End(i32),
}

/// The control socket and the results pipe of a spawned profiler, as `<socket>,<pipe>`.
const DESCRIPTORS: &str = "JOULE_PROFILER_IPC_DESCRIPTORS";

const STARTED: u8 = b'S';
const DECLARED: u8 = b'D';

#[derive(Serialize)]
enum Written<'a> {
    Schema(&'a Schema),
    Phase {
        info: &'a PhaseInfo,
        values: Values<'a>,
    },
    Summary(&'a Summary),
    Error(&'a str),
}

struct Values<'a>(&'a [SourceMetrics<'a>]);

impl Serialize for Values<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        struct Of<'a>(&'a SourceMetrics<'a>);

        impl Serialize for Of<'_> {
            fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                serializer.collect_seq(self.0.metrics.iter().map(|metric| metric.value))
            }
        }

        serializer.collect_seq(self.0.iter().map(Of))
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct PhaseValues {
    pub info: PhaseInfo,
    pub values: Vec<Vec<MetricValue>>,
}

impl PhaseValues {
    pub fn new(info: &PhaseInfo, sources: &[SourceMetrics<'_>]) -> Self {
        Self {
            info: info.clone(),
            values: sources
                .iter()
                .map(|source| source.metrics.iter().map(|metric| metric.value).collect())
                .collect(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Received {
    Schema(Schema),
    Phase(PhaseValues),
    Summary(Summary),
}

#[derive(Deserialize)]
enum Read {
    Schema(Schema),
    Phase(PhaseValues),
    Summary(Summary),
    Error(String),
}
