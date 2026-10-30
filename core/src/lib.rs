//! Core of Joule Profiler.
//!
//! Joule Profiler measures a program phase by phase. An [`Injector`] runs the program and
//! reports the boundaries of its phases, each [`Source`] measures at every boundary, and an
//! [`Exporter`] receives the metrics of each phase. [`JouleProfiler`] orchestrates them together.
//!
//! A source is a [`Sensor`], which takes the measurements on its own thread, and a
//! [`Processor`], which turns two snapshots into the metrics of a phase, on the
//! exporter's thread.
//!
//! See [`JouleProfiler`] for an example.

mod error;
mod exporter;
mod info;
mod injector;
mod metric;
mod phase;
mod processor;
mod profiler;
mod schema;
mod sensor;
mod source;
mod trigger;
mod unit;
pub mod util;

pub use error::Error;
pub use exporter::Exporter;
pub use info::{Info, InfoValue};
pub use injector::{Injector, PhaseToken, StopHandle, Target};
pub use metric::{Metric, MetricInfo, MetricValue};
pub use phase::{PhaseInfo, SourceMetrics, Summary};
pub use processor::Processor;
pub use profiler::JouleProfiler;
pub use schema::{Schema, SourceInfo};
pub use sensor::Sensor;
pub use source::Source;
pub use unit::{MemoryUnit, MetricUnit, Unit, UnitPrefix};
