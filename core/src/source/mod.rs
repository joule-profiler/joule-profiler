//! Internal abstractions for metric sources.
//!
//! This module defines the private traits used by the
//! profiler to manage metric readers. It is not part of the public API.
//! Implementations are boxed for flexibility, while internally resolving
//! concrete types to minimize the profiler overhead.

use std::future::Future;
use std::pin::Pin;

use tokio::sync::mpsc;

pub(crate) mod accumulator;
pub mod error;
pub mod reader;
pub(crate) mod runtime;
pub(crate) mod types;

use crate::sensor::Sensors;
use crate::source::types::{SourceEvent, SourceWorkerHandle};
pub use error::MetricSourceError;
pub use reader::MetricReader;
pub use types::{MetricReaderErrorBound, MetricReaderTypeBound};

/// Internal trait representing a runnable metric source.
///
/// Implemented by the runtime wrapper around a [`MetricReader`].
/// This trait is used to erase the type of the metric source, to be able to have a
/// convenient API for users while maintaining performance with monomorphization during hot paths.
pub(crate) trait MetricSource: Send {
    /// Initialize the source with the profiled program's pid.
    ///
    /// Must be awaited (and completed) before [`MetricSource::run`] is called.
    fn init(
        &mut self,
        pid: i32,
    ) -> Pin<Box<dyn Future<Output = Result<(), MetricSourceError>> + Send + '_>>;

    /// Spawn the source worker and return its handle.
    fn run(self: Box<Self>, control_receiver: mpsc::Receiver<SourceEvent>) -> SourceWorkerHandle;

    /// List sensors exposed by this source.
    fn list_sensors(&self) -> Result<Sensors, MetricSourceError>;
}
