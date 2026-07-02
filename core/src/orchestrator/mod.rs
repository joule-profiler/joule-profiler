//! Core orchestration module for `JouleProfiler`.
//!
//! This module defines the core logic for metric sources orchestration through [`SourceOrchestrator`] structure.

use std::time::Duration;

use crate::aggregate::sensor_result::SensorResult;
use crate::orchestrator::error::OrchestratorError;
use crate::source::types::SourceEvent;
use crate::source::{MetricSource, MetricSourceError};
use crate::util::time::get_timestamp_micros;
use futures::future::try_join_all;
use log::{debug, trace};
use tokio::time::timeout;
use tokio::{sync::mpsc, task::JoinHandle};

pub mod error;

/// The size of the control channel to send event.
/// It should not have a huge size because if the buffer is full,
/// it means that the sources are slower to measure than the phases durations.
pub const CONTROL_CHANNEL_SIZE: usize = 16;

/// The handle describing the return type of a source worker.
type TaskHandle = JoinHandle<Result<(SensorResult, Box<dyn MetricSource>), MetricSourceError>>;

struct SourceHandle {
    /// The event channel sender used to manage the metric sources.
    control_sender: mpsc::Sender<SourceEvent>,

    /// The handle of the worker task, used for joining sources gracefully.
    handle: TaskHandle,
}

/// Orchestrates the metric sources and send them the profiler's messages through asynchronous channels.
/// It is a proxy between the profiler and the sources and is responsible of their lifecycle.
pub struct Orchestrator {
    /// Sources initialized through [`SourceOrchestrator::init`], not yet started by [`SourceOrchestrator::run`].
    sources: Vec<Box<dyn MetricSource>>,

    handles: Vec<SourceHandle>,
}

impl Orchestrator {
    pub fn new(sources: Vec<Box<dyn MetricSource>>) -> Self {
        Self {
            sources,
            handles: Vec::new(),
        }
    }

    /// Initializes each metric source with the profiled program's pid, bounded by `init_timeout`.
    ///
    /// Used by some sources for per-process profiling (e.g. `perf_event`).
    /// Stores the initialized sources, ready to be started with [`SourceOrchestrator::run`].
    ///
    /// Each source is initialized in its own task: some [`MetricReader::init`](`crate::source::MetricReader::init`)
    /// implementations perform blocking work without yielding, which would otherwise prevent
    /// `init_timeout` from being enforced if awaited directly on the calling task.
    pub async fn init(
        &mut self,
        pid: i32,
        init_timeout: Duration,
    ) -> Result<(), OrchestratorError> {
        trace!(
            "Initializing {} source(s) with pid {pid}",
            self.sources.len()
        );
        let sources = std::mem::take(&mut self.sources);
        let tasks = sources.into_iter().map(|mut source| {
            tokio::spawn(async move {
                let result = source.init(pid).await;
                (source, result)
            })
        });

        let begin_init_timestamp = get_timestamp_micros();
        let initialized = timeout(init_timeout, try_join_all(tasks))
            .await
            .map_err(|_| OrchestratorError::InitializationError("init timeout reached"))??;
        let end_init_timestamp = get_timestamp_micros();
        debug!(
            "Sources initialized in {}μs.",
            end_init_timestamp - begin_init_timestamp
        );

        self.sources = initialized
            .into_iter()
            .map(|(source, result)| result.map(|()| source))
            .collect::<Result<Vec<_>, MetricSourceError>>()?;

        Ok(())
    }

    /// Starts all the metric sources, already initialized through [`SourceOrchestrator::init`].
    ///
    /// Stores the sources handles and the channels senders to be able to gracefully join the sources and send events.
    #[inline]
    pub fn run(&mut self) -> Result<(), OrchestratorError> {
        trace!(
            "Starting orchestrator with {} source(s)",
            self.sources.len()
        );
        let sources = std::mem::take(&mut self.sources);

        if sources.is_empty() {
            return Err(OrchestratorError::NoSourceConfigured);
        }

        self.handles = sources
            .into_iter()
            .map(|source| {
                let (control_sender, control_receiver) = mpsc::channel(CONTROL_CHANNEL_SIZE);
                let handle = source.run(control_receiver);
                SourceHandle {
                    control_sender,
                    handle,
                }
            })
            .collect();

        Ok(())
    }

    /// Send a measure event to each metrics source.
    ///
    /// This function ensure event submission, but is does not ensure
    /// measurement completion and that the measure will be taken directly after it.
    /// At high concurrency and with many running sources, the measurement can be made lately.
    #[inline]
    pub async fn measure(&mut self) -> Result<(), OrchestratorError> {
        self.send_event(SourceEvent::Measure).await
    }

    /// Initializes a new phase for each metric source.
    #[inline]
    pub async fn new_phase(&mut self) -> Result<(), OrchestratorError> {
        self.send_event(SourceEvent::NewPhase).await
    }

    /// Retrieves and merge results from all sources.
    ///
    /// Returns a tuple containing the aggregated results and the list of the metric sources in order to reuse them.
    ///
    /// # Errors
    ///
    /// If not enough snapshots have been made, a [`NotEnoughSnapshots`](`OrchestratorError::NotEnoughSnapshots`) error is returned.
    /// Also if an error has occured in one of the sources, it will be returned.
    pub async fn finalize(
        &mut self,
    ) -> Result<(SensorResult, Vec<Box<dyn MetricSource>>), OrchestratorError> {
        let (results, sources) = self.join_all().await?;
        let merged = SensorResult::merge(results)?;
        Ok((merged, sources))
    }

    /// Stop the worker thread of each metrics sources to join threads gracefully.
    #[inline]
    async fn join(&mut self) -> Result<(), OrchestratorError> {
        self.send_event(SourceEvent::JoinWorker).await
    }

    /// Sends the provided event to all the metrics sources.
    ///
    /// If an error is encountered in a source, then the worker is aborted and the error is returned.
    async fn send_event(&mut self, event: SourceEvent) -> Result<(), OrchestratorError> {
        if let Err((failed_index, send_err)) = try_join_all(
            self.handles
                .iter_mut()
                .enumerate()
                .map(
                    |(i, h)| async move { h.control_sender.send(event).await.map_err(|e| (i, e)) },
                ),
        )
        .await
        {
            Err(self.handle_event_error(failed_index, send_err.into()).await)
        } else {
            Ok(())
        }
    }

    /// Handles the error from a disconnected source (failed) and return it.
    async fn handle_event_error(
        &mut self,
        failed_index: usize,
        err: OrchestratorError,
    ) -> OrchestratorError {
        if self.handles.get(failed_index).is_none() {
            return err;
        }
        let source_handle = self.handles.remove(failed_index);

        match source_handle.handle.await {
            Ok(Ok((_, _))) => err,
            Ok(Err(metric_err)) => metric_err.into(),
            Err(join_err) => join_err.into(),
        }
    }

    /// Joins all workers and collect results.
    /// Waits until workers termination.
    /// If an error has occured in one of the sources, it will be returned.
    async fn join_all(
        &mut self,
    ) -> Result<(Vec<SensorResult>, Vec<Box<dyn MetricSource>>), OrchestratorError> {
        self.join().await?;

        let handles = std::mem::take(&mut self.handles);

        let results = try_join_all(handles.into_iter().map(|h| h.handle)).await?;

        let (results, sources) = results
            .into_iter()
            .map(|r| r.map_err(OrchestratorError::from))
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .unzip();

        Ok((results, sources))
    }
}

#[cfg(test)]
mod tests {
    use mockall::mock;

    use crate::{sensor::Sensors, source::MetricReader, types::Metrics};

    use super::*;
    use std::sync::{Arc, Mutex};

    #[derive(Debug)]
    pub struct MockError;

    impl std::fmt::Display for MockError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "mock error")
        }
    }

    impl std::error::Error for MockError {}

    mock! {
        pub MetricReader {}

        impl MetricReader for MetricReader {
            type Type = ();
            type Error = MockError;

            async fn init(&mut self, pid: i32) -> Result<(), MockError>;
            async fn join(&mut self) -> Result<(), MockError>;
            async fn measure(&mut self) -> Result<(), MockError>;
            async fn retrieve(&mut self) -> Result<(), MockError>;
            fn get_sensors(&self) -> Result<Sensors, MockError>;
            fn to_metrics(&self, v: ()) -> Result<Metrics, MockError>;
            fn get_name() -> &'static str;
        }
    }

    #[derive(Debug, Default)]
    struct State {
        pid: i32,
        init: usize,
        join: usize,
        measure: usize,
    }

    fn mock_reader() -> (MockMetricReader, Arc<Mutex<State>>) {
        let state_arc = Arc::new(Mutex::new(State::default()));
        let mut mock = MockMetricReader::new();

        let state = state_arc.clone();
        mock.expect_init().returning(move |pid| {
            let mut lock = state.lock().unwrap();
            lock.init += 1;
            lock.pid = pid;
            Ok(())
        });

        let state = state_arc.clone();
        mock.expect_join().returning(move || {
            state.lock().unwrap().join += 1;
            Ok(())
        });

        let state = state_arc.clone();
        mock.expect_measure().returning(move || {
            state.lock().unwrap().measure += 1;
            Ok(())
        });

        mock.expect_get_sensors().returning(|| Ok(vec![]));
        mock.expect_to_metrics()
            .returning(|_| Ok(Metrics::default()));

        (mock, state_arc)
    }

    fn mock_source() -> (Box<dyn MetricSource>, Arc<Mutex<State>>) {
        let (r, state) = mock_reader();
        (r.into(), state)
    }

    #[tokio::test]
    async fn finalize_without_measurements_returns_not_enough_snapshots() {
        let (source, _) = mock_source();
        let mut orchestrator = Orchestrator::new(vec![source]);
        orchestrator.init(0, Duration::from_secs(1)).await.unwrap();
        orchestrator.run().unwrap();

        assert!(matches!(
            orchestrator.finalize().await,
            Err(OrchestratorError::AllSourcesEmpty)
        ));
    }

    #[tokio::test]
    async fn run_orchestrator_with_no_source_returns_error() {
        let mut orchestrator = Orchestrator::new(Vec::new());

        assert!(matches!(
            orchestrator.run(),
            Err(OrchestratorError::NoSourceConfigured)
        ));
    }

    #[tokio::test]
    async fn event_reaches_worker() {
        let (source, state) = mock_source();
        let mut orchestrator = Orchestrator::new(vec![source]);
        orchestrator.init(0, Duration::from_secs(1)).await.unwrap();
        orchestrator.run().unwrap();

        let _ = orchestrator.measure().await;
        let _ = orchestrator.join().await;

        tokio::task::yield_now().await;

        let lock = state.lock().unwrap();

        assert_eq!(lock.measure, 1);
        assert_eq!(lock.init, 1);
        assert_eq!(lock.join, 1);
    }

    #[tokio::test]
    async fn init_initializes_source_with_right_pid() {
        let (source, state) = mock_source();
        let mut orchestrator = Orchestrator::new(vec![source]);

        orchestrator.init(42, Duration::from_secs(1)).await.unwrap();

        assert_eq!(state.lock().unwrap().pid, 42);
    }

    // Requires a multi-threaded runtime: a blocking `init` occupies its own worker thread,
    // relying on another worker thread being free to enforce `init_timeout`.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn init_times_out_on_blocking_source() {
        let mut reader = MockMetricReader::new();
        reader.expect_init().returning(|_| {
            // Simulates a `MetricReader::init` implementation performing blocking
            // work without ever yielding (e.g. a blocking syscall or file read),
            // which must not prevent `init_timeout` from being enforced.
            std::thread::sleep(Duration::from_millis(200));
            Ok(())
        });
        let source: Box<dyn MetricSource> = reader.into();
        let mut orchestrator = Orchestrator::new(vec![source]);

        let result = orchestrator.init(0, Duration::from_millis(20)).await;

        assert!(matches!(
            result,
            Err(OrchestratorError::InitializationError(_))
        ));
    }

    #[tokio::test]
    async fn measure_error_in_worker_propagates_to_orchestrator() {
        let mut reader = MockMetricReader::new();
        reader.expect_init().returning(|_| Ok(()));
        reader.expect_measure().returning(|| Err(MockError));
        let source: Box<dyn MetricSource> = reader.into();
        let mut orchestrator = Orchestrator::new(vec![source]);

        orchestrator.init(0, Duration::from_secs(1)).await.unwrap();
        orchestrator.run().unwrap();
        orchestrator.measure().await.unwrap();
        let result = orchestrator.finalize().await;

        assert!(result.is_err());
        assert!(matches!(
            result,
            Err(OrchestratorError::MetricSourceError(_))
        ));
    }
}
