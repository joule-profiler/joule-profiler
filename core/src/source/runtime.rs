use std::pin::Pin;

use log::debug;
use tokio::sync::mpsc;

use crate::{
    aggregate::{phase::SensorPhase, sensor_result::SensorResult},
    sensor::Sensors,
    source::{
        MetricReader, MetricSource, MetricSourceError,
        accumulator::MetricAccumulator,
        error::IntoMetricSourceError,
        types::{SourceEvent, SourceWorkerHandle},
    },
};

/// Orchestrate a metric source and handle the conversion between raw source results to metrics.
pub struct MetricSourceRuntime<R: MetricReader> {
    accumulator: MetricAccumulator<R>,
    source: R,
}

impl<R: MetricReader> MetricSourceRuntime<R> {
    /// Initialize a [`MetricSourceRuntime`] with the given [`MetricReader`] generic type.
    pub fn new(reader: R) -> Self {
        debug!("Creating MetricAccumulator for reader: {}", R::get_name());

        Self {
            accumulator: MetricAccumulator::new(),
            source: reader,
        }
    }

    /// Runs the worker responsible for source and accumulator management.
    ///
    /// It listens for events through a channel and execute them.
    /// The source must already be initialized (see [`MetricSource::init`]).
    pub(crate) async fn run_worker(
        mut self,
        mut receiver: mpsc::Receiver<SourceEvent>,
    ) -> Result<(SensorResult, Box<dyn MetricSource>), MetricSourceError> {
        loop {
            if let Some(event) = receiver.recv().await {
                match event {
                    SourceEvent::Measure => self.measure_source().await?,
                    SourceEvent::NewPhase => self.init_new_phase().await?,
                    SourceEvent::JoinWorker => break,
                }
            }
        }

        self.source
            .join()
            .await
            .map_err(IntoMetricSourceError::into_metric_source_error)?;

        let result = self.retrieve()?;
        Ok((result, self.source.into()))
    }

    /// Make a measurement.
    #[inline]
    async fn measure_source(&mut self) -> Result<(), MetricSourceError> {
        self.source
            .measure()
            .await
            .map_err(IntoMetricSourceError::into_metric_source_error)
    }

    /// Initialize a new phase.
    #[inline]
    async fn init_new_phase(&mut self) -> Result<(), MetricSourceError> {
        let result = self
            .source
            .retrieve()
            .await
            .map_err(IntoMetricSourceError::into_metric_source_error)?;
        self.accumulator.new_phase(result);
        Ok(())
    }

    /// Retrieve the results from the accumulator and convert them into metrics.
    #[inline]
    fn retrieve(&mut self) -> Result<SensorResult, MetricSourceError> {
        let result = self
            .accumulator
            .retrieve()
            .into_iter()
            .map(|phase| {
                Ok(SensorPhase {
                    metrics: self
                        .source
                        .to_metrics(phase.metrics)
                        .map_err(IntoMetricSourceError::into_metric_source_error)?,
                })
            })
            .collect::<Result<Vec<_>, MetricSourceError>>()?;
        Ok(SensorResult { phases: result })
    }
}

impl<R> MetricSource for MetricSourceRuntime<R>
where
    R: MetricReader,
{
    /// Initializes the source with the profiled program's pid.
    fn init(
        &mut self,
        pid: i32,
    ) -> Pin<Box<dyn Future<Output = Result<(), MetricSourceError>> + Send + '_>> {
        Box::pin(async move {
            self.source
                .init(pid)
                .await
                .map_err(IntoMetricSourceError::into_metric_source_error)
        })
    }

    /// Spawns the worker task and returns its handle, used to send events to manage the metric source.
    ///
    /// The metric source is consumed and transformed into a [`MetricSourceRuntime`] with the metric source as a reader.
    /// This transformation allows to monomorphize the metric source and discover its type after its launch.
    fn run(self: Box<Self>, control_receiver: mpsc::Receiver<SourceEvent>) -> SourceWorkerHandle {
        tokio::spawn(async move { self.run_worker(control_receiver).await })
    }

    /// List the sensors of the metric source.
    fn list_sensors(&self) -> Result<Sensors, MetricSourceError> {
        self.source
            .get_sensors()
            .map_err(IntoMetricSourceError::into_metric_source_error)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::aggregate::Metrics;
    use crate::sensor::Sensors;
    use crate::source::MetricReader;
    use mockall::mock;
    use std::sync::{Arc, Mutex};
    use tokio::sync::mpsc;

    #[derive(Debug)]
    pub struct MockError(String);

    impl std::fmt::Display for MockError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "{}", self.0)
        }
    }
    impl std::error::Error for MockError {}

    mock! {
        pub MetricReader {}
        impl MetricReader for MetricReader {
            type Type  = ();
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
    struct Counts {
        init: usize,
        join: usize,
        measure: usize,
        retrieve: usize,
    }

    fn mock_reader_counted() -> (MockMetricReader, Arc<Mutex<Counts>>) {
        let counts = Arc::new(Mutex::new(Counts::default()));
        let mut m = MockMetricReader::new();

        let c = counts.clone();
        m.expect_init().returning(move |_| {
            c.lock().unwrap().init += 1;
            Ok(())
        });

        let c = counts.clone();
        m.expect_join().returning(move || {
            c.lock().unwrap().join += 1;
            Ok(())
        });

        let c = counts.clone();
        m.expect_measure().returning(move || {
            c.lock().unwrap().measure += 1;
            Ok(())
        });

        let c = counts.clone();
        m.expect_retrieve().returning(move || {
            c.lock().unwrap().retrieve += 1;
            Ok(())
        });

        m.expect_get_sensors().returning(|| Ok(vec![]));
        m.expect_to_metrics().returning(|_| Ok(Metrics::default()));

        (m, counts)
    }

    #[tokio::test]
    async fn run_worker_measure_event_calls_measure() {
        let (reader, counts) = mock_reader_counted();
        let mut rt = MetricSourceRuntime::new(reader);
        rt.init(0).await.unwrap();
        let (tx, rx) = mpsc::channel(16);

        tx.send(SourceEvent::Measure).await.unwrap();
        tx.send(SourceEvent::Measure).await.unwrap();
        tx.send(SourceEvent::JoinWorker).await.unwrap();

        rt.run_worker(rx).await.unwrap();

        assert_eq!(counts.lock().unwrap().measure, 2);
    }

    #[tokio::test]
    async fn init_source_passes_pid() {
        let (reader, counts) = mock_reader_counted();
        let mut rt = MetricSourceRuntime::new(reader);

        rt.init(42).await.unwrap();

        assert_eq!(counts.lock().unwrap().init, 1);
    }

    #[tokio::test]
    async fn run_worker_measure_error_propagates() {
        let mut reader = MockMetricReader::new();
        reader.expect_init().returning(|_| Ok(()));
        reader
            .expect_measure()
            .returning(|| Err(MockError("injected".into())));
        let mut rt = MetricSourceRuntime::new(reader);
        rt.init(0).await.unwrap();
        let (tx, rx) = mpsc::channel(16);

        tx.send(SourceEvent::Measure).await.unwrap();
        tx.send(SourceEvent::JoinWorker).await.unwrap();

        assert!(rt.run_worker(rx).await.is_err());
    }

    #[tokio::test]
    async fn run_worker_new_phase_calls_retrieve() {
        let (reader, counts) = mock_reader_counted();
        let mut rt = MetricSourceRuntime::new(reader);
        rt.init(0).await.unwrap();
        let (tx, rx) = mpsc::channel(16);

        tx.send(SourceEvent::NewPhase).await.unwrap();
        tx.send(SourceEvent::JoinWorker).await.unwrap();

        rt.run_worker(rx).await.unwrap();

        assert_eq!(counts.lock().unwrap().retrieve, 1);
    }

    #[tokio::test]
    async fn run_worker_join_calls_join_on_source() {
        let (reader, counts) = mock_reader_counted();
        let mut rt = MetricSourceRuntime::new(reader);
        rt.init(0).await.unwrap();
        let (tx, rx) = mpsc::channel(16);

        tx.send(SourceEvent::JoinWorker).await.unwrap();
        rt.run_worker(rx).await.unwrap();

        assert_eq!(counts.lock().unwrap().join, 1);
    }

    #[tokio::test]
    async fn run_worker_full_lifecycle() {
        let (reader, counts) = mock_reader_counted();
        let mut rt = MetricSourceRuntime::new(reader);
        rt.init(0).await.unwrap();
        let (tx, rx) = mpsc::channel(16);

        tx.send(SourceEvent::Measure).await.unwrap();
        tx.send(SourceEvent::Measure).await.unwrap();
        tx.send(SourceEvent::NewPhase).await.unwrap();
        tx.send(SourceEvent::JoinWorker).await.unwrap();

        assert!(rt.run_worker(rx).await.is_ok());

        let c = counts.lock().unwrap();
        assert_eq!(c.measure, 2);
        assert_eq!(c.retrieve, 1);
        assert_eq!(c.join, 1);
    }
}
