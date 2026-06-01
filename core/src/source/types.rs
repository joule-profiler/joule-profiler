use tokio::task::JoinHandle;

use crate::aggregate::sensor_result::SensorResult;
use crate::source::{MetricSource, MetricSourceError};
use std::fmt::Debug;

/// Trait for types returned by a [`MetricReader`](`super::MetricReader`).
///
/// Any type implementing this trait represents the result of a metric measurement.
/// It must implement `Send` to be safely.
pub trait MetricReaderTypeBound: Send {}

impl<T> MetricReaderTypeBound for T where T: Send {}

/// Trait for errors produced by a [`MetricReader`](`super::MetricReader`).
///
/// This trait marks the types of errors that can occur during metric collection.
/// Errors must implement `std::error::Error` for error handling
/// and be `Send + Sync` so they can be safely propagated across threads.
pub trait MetricReaderErrorBound: std::error::Error + Send + Sync {}

impl<E> MetricReaderErrorBound for E where E: std::error::Error + Send + Sync {}

/// The handle of a worker to gracefully join it.
/// It returns the collected results and also the metric source with erased type for API convenience.
/// If an error occured in the source during the measurements, then it is returned.  
pub type SourceWorkerHandle =
    JoinHandle<Result<(SensorResult, Box<dyn MetricSource>), MetricSourceError>>;

/// Events sent to a metric source worker.
#[derive(Debug, Clone, Copy)]
pub enum SourceEvent {
    /// Triggers measurement of metrics.
    Measure,

    /// Starts a new measurement phase.
    NewPhase,

    /// Signals the worker to finish and join.
    JoinWorker,
}

/// Raw phase containing metrics from a metric reader.
#[derive(Debug, Default, Clone)]
pub(crate) struct RawPhase<V> {
    /// Metrics collected during the phase.
    pub metrics: V,
}
