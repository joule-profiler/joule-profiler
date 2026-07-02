//! Core orchestration module for `JouleProfiler`.
//!
//! This module defines the core logic for metric sources orchestration through [`SourceOrchestrator`] structure.

use std::time::Duration;

use crate::aggregate::sensor_result::SensorResult;
use crate::orchestrator::error::OrchestratorError;
use crate::source::error::IntoMetricSourceError;
use crate::source::types::SourceEvent;
use crate::source::{MetricSource, MetricSourceError};
use futures::future::try_join_all;
use tokio::time::timeout;
use tokio::{
    sync::{mpsc, oneshot},
    task::JoinHandle,
};

pub mod error;

/// The size of the control channel to send event.
/// It should not have a huge size because if the buffer is full,
/// it means that the sources are slower to measure than the phases durations.
pub const CONTROL_CHANNEL_SIZE: usize = 16;

/// The handle describing the return type of a source worker.
type TaskHandle = JoinHandle<Result<(SensorResult, Box<dyn MetricSource>), MetricSourceError>>;

struct InitChannel {
    sender: oneshot::Sender<i32>,
    validation: oneshot::Receiver<Result<(), MetricSourceError>>,
}

impl InitChannel {
    async fn initialize(self, pid: i32) -> Result<(), OrchestratorError> {
        self.sender
            .send(pid)
            .map_err(|_| OrchestratorError::InitializationError("Failed to send PID"))?;

        self.validation
            .await
            .map_err(|e| OrchestratorError::MetricSourceError(e.into_metric_source_error()))??;

        Ok(())
    }
}

struct SourceHandle {
    /// The event channel sender used to manage the metric sources.
    control_sender: mpsc::Sender<SourceEvent>,

    /// The channels used for sources initialization.
    init_channel: Option<InitChannel>,

    /// The handle of the worker task, used for joining sources gracefully.
    handle: TaskHandle,
}

/// Orchestrates the metric sources and send them the profiler's messages through asynchronous channels.
/// It is a proxy between the profiler and the sources and is responsible of their lifecycle.
#[derive(Default)]
pub struct SourceOrchestrator {
    handles: Vec<SourceHandle>,
    init_timeout: Duration,
}

impl SourceOrchestrator {
    /// Starts all the metric sources.
    ///
    /// The function uses a one shot sender to forward the profiled program's pid to the sources, used by some for per-process profiling (e.g. `perf_event`).
    /// Stores the sources handles and the channels senders to be able to gracefully join the sources and send events.
    #[inline]
    pub fn run(
        &mut self,
        sources: Vec<Box<dyn MetricSource>>,
        init_timeout: Duration,
    ) -> Result<(), OrchestratorError> {
        if sources.is_empty() {
            return Err(OrchestratorError::NoSourceConfigured);
        }
        let mut handles = Vec::with_capacity(sources.len());

        for source in sources {
            let (control_sender, control_receiver) = mpsc::channel(CONTROL_CHANNEL_SIZE);
            let (init_sender, init_receiver) = oneshot::channel();
            let (init_validation_sender, init_validation_receiver) = oneshot::channel();

            let handle = source.run(
                control_receiver,
                init_receiver,
                init_validation_sender,
                init_timeout,
            );

            let init_channel = InitChannel {
                sender: init_sender,
                validation: init_validation_receiver,
            };

            handles.push(SourceHandle {
                handle,
                control_sender,
                init_channel: Some(init_channel),
            });
        }

        self.init_timeout = init_timeout;
        self.handles = handles;

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

    /// Initializes each metric source.
    /// Called when the program execution is stopped to inizialize sources requiring pid filtering (e.g. `perf_event`).
    /// Wait for all sources to be initialized and to receive their validation.
    pub async fn init(&mut self, pid: i32) -> Result<(), OrchestratorError> {
        let futures = self.handles.iter_mut().map(|handle| async move {
            if let Some(init_channel) = handle.init_channel.take() {
                init_channel.initialize(pid).await
            } else {
                Ok(())
            }
        });

        timeout(self.init_timeout, try_join_all(futures))
            .await
            .map_err(|_| OrchestratorError::InitializationError("init timeout reached"))??;

        Ok(())
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
        let mut orchestrator = SourceOrchestrator::default();
        let (source, _) = mock_source();
        orchestrator
            .run(vec![source], Duration::from_secs(1))
            .unwrap();
        orchestrator.init(0).await.unwrap();

        assert!(matches!(
            orchestrator.finalize().await,
            Err(OrchestratorError::AllSourcesEmpty)
        ));
    }

    #[tokio::test]
    async fn run_orchestrator_with_no_source_returns_error() {
        let mut orchestrator = SourceOrchestrator::default();

        assert!(matches!(
            orchestrator.run(vec![], Duration::from_secs(1)),
            Err(OrchestratorError::NoSourceConfigured)
        ));
    }

    #[tokio::test]
    async fn event_reaches_worker() {
        let (source, state) = mock_source();
        let mut orchestrator = SourceOrchestrator::default();
        orchestrator
            .run(vec![source], Duration::from_secs(1))
            .unwrap();

        let _ = orchestrator.measure().await;
        let _ = orchestrator.init(0).await;
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

        let mut orchestrator = SourceOrchestrator::default();
        orchestrator
            .run(vec![source], Duration::from_secs(1))
            .unwrap();

        orchestrator.init(42).await.unwrap();

        // wait for initialization
        tokio::task::yield_now().await;

        assert_eq!(state.lock().unwrap().pid, 42);
    }

    #[tokio::test]
    async fn measure_error_in_worker_propagates_to_orchestrator() {
        let mut reader = MockMetricReader::new();
        reader.expect_init().returning(|_| Ok(()));
        reader.expect_measure().returning(|| Err(MockError));
        let source: Box<dyn MetricSource> = reader.into();
        let mut orchestrator = SourceOrchestrator::default();

        orchestrator
            .run(vec![source], Duration::from_secs(1))
            .unwrap();
        orchestrator.init(0).await.unwrap();
        orchestrator.measure().await.unwrap();
        let result = orchestrator.finalize().await;

        assert!(result.is_err());
        assert!(matches!(
            result,
            Err(OrchestratorError::MetricSourceError(_))
        ));
    }
}
