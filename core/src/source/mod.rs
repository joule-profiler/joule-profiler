//! Internal abstractions for metric sources.
//!
//! This module defines the private traits used by the
//! profiler to manage metric readers. It is not part of the public API.
//! Implementations are boxed for flexibility, while internally resolving
//! concrete types to minimize the profiler overhead.

use std::time::Duration;

use tokio::sync::{mpsc, oneshot};

pub(crate) mod accumulator;
pub mod error;
pub mod reader;
pub(crate) mod runtime;
pub(crate) mod types;

use crate::sensor::Sensors;
use crate::source::runtime::MetricSourceRuntime;
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
    /// Spawn the source worker and return its handle, control channel and initialization channel.
    fn run(
        self: Box<Self>,
        init_timeout: Duration,
    ) -> (
        SourceWorkerHandle,
        mpsc::Sender<SourceEvent>,
        oneshot::Sender<i32>,
    );

    /// List sensors exposed by this source.
    fn list_sensors(&self) -> Result<Sensors, MetricSourceError>;
}

impl<R> MetricSource for MetricSourceRuntime<R>
where
    R: MetricReader,
{
    /// Runs the worker task and returns its handle and the sender, used to send events to manage the metric source.
    ///
    /// The metric source is consumed and transformed into a [`MetricSourceRuntime`] with the metric source as a reader.
    /// This transformation allows to monomorphize the metric source and discover its type after its launch.
    fn run(
        self: Box<Self>,
        init_timeout: Duration,
    ) -> (
        SourceWorkerHandle,
        mpsc::Sender<SourceEvent>,
        oneshot::Sender<i32>,
    ) {
        let (control_sender, control_receiver) = mpsc::channel(4);
        let (init_sender, init_receiver) = oneshot::channel();
        let handle = tokio::spawn(async move {
            self.run_worker(control_receiver, init_receiver, init_timeout)
                .await
        });
        (handle, control_sender, init_sender)
    }

    /// List the sensors of the metric source.
    fn list_sensors(&self) -> Result<Sensors, MetricSourceError> {
        self.get_source_sensors()
    }
}

/// Converts a [`MetricReader`] into a boxed [`MetricSource`].
impl<R> From<R> for Box<dyn MetricSource>
where
    R: MetricReader,
{
    fn from(reader: R) -> Self {
        let source = MetricSourceRuntime::new(reader);
        Box::new(source)
    }
}
