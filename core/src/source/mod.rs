use std::pin::Pin;
use std::{error::Error, future::Future};

use thiserror::Error;
use tokio::sync;
use tokio::sync::broadcast::error::RecvError;
use tokio::{
    sync::mpsc::{self, channel},
    task::JoinHandle,
};

mod processor;
mod sensor;

pub use processor::Processor;
pub use sensor::Sensor;

use crate::types::{InitContext, Phase, Sensors};

#[derive(Debug, Error)]
#[error(transparent)]
pub struct MetricSourceError(#[from] Box<dyn std::error::Error + Send + Sync>);

impl MetricSourceError {
    pub(crate) fn boxed<E: Error + Send + Sync + 'static>(error: E) -> Self {
        Self(Box::new(error))
    }
}

pub trait MetricSource: Send + Sized + 'static {
    type Sensor: Sensor<Self>;
    type Processor: Processor<Self>;
    type Snapshot: Send + Sync;
    type Error: Error + Send + Sync;
    type Config;

    fn from_config(config: Self::Config) -> Result<(Self::Sensor, Self::Processor), Self::Error>;

    fn get_name() -> &'static str;
}

pub(crate) trait MetricSourceWrapper: Send {
    fn pre_init(
        &mut self,
    ) -> Pin<Box<dyn Future<Output = Result<(), MetricSourceError>> + Send + '_>>;

    fn init(
        &mut self,
        ctx: InitContext,
    ) -> Pin<Box<dyn Future<Output = Result<(), MetricSourceError>> + Send + '_>>;

    fn list_sensors(&self) -> Result<Sensors, MetricSourceError>;

    fn run(
        self: Box<Self>,
        rx: sync::broadcast::Receiver<()>,
        exporter_tx: mpsc::Sender<Phase>,
    ) -> JoinHandle<Result<Box<dyn MetricSourceWrapper>, MetricSourceError>>;
}

pub(crate) struct MetricSourceRuntime<S: MetricSource> {
    sensor: S::Sensor,
    processor: S::Processor,
}

impl<S: MetricSource> MetricSourceRuntime<S> {
    pub fn new(config: S::Config) -> Result<Self, S::Error> {
        let (sensor, processor) = S::from_config(config)?;
        Ok(Self { sensor, processor })
    }

    pub fn boxed(config: S::Config) -> Result<Box<dyn MetricSourceWrapper>, MetricSourceError> {
        Ok(Box::new(
            Self::new(config).map_err(MetricSourceError::boxed)?,
        ))
    }
}

impl<S> MetricSourceWrapper for MetricSourceRuntime<S>
where
    S: MetricSource,
{
    fn pre_init(
        &mut self,
    ) -> Pin<Box<dyn Future<Output = Result<(), MetricSourceError>> + Send + '_>> {
        Box::pin(async {
            self.sensor
                .pre_init()
                .await
                .map_err(MetricSourceError::boxed)
        })
    }

    fn init(
        &mut self,
        ctx: InitContext,
    ) -> Pin<Box<dyn Future<Output = Result<(), MetricSourceError>> + Send + '_>> {
        Box::pin(async move {
            self.sensor
                .init(ctx)
                .await
                .map_err(MetricSourceError::boxed)?;
            self.processor
                .init()
                .await
                .map_err(MetricSourceError::boxed)?;
            Ok(())
        })
    }

    fn run(
        self: Box<Self>,
        mut rx: sync::broadcast::Receiver<()>,
        exporter_tx: mpsc::Sender<Phase>,
    ) -> JoinHandle<Result<Box<dyn MetricSourceWrapper>, MetricSourceError>> {
        tokio::spawn(async move {
            let mut sensor = self.sensor;
            let mut processor = self.processor;
            let mut phase_index = 0;

            let (sensor_tx, mut processor_rx) = channel::<S::Snapshot>(128);

            let processor_task = tokio::spawn(async move {
                while let Some(snapshot) = processor_rx.recv().await {
                    match processor.consume(snapshot).await {
                        Ok(Some(mut metrics)) => {
                            for metric in &mut metrics {
                                metric.source = S::get_name();
                            }
                            let phase = Phase {
                                phase_index,
                                metrics,
                            };
                            exporter_tx
                                .send(phase)
                                .await
                                .map_err(MetricSourceError::boxed)?;
                            phase_index += 1;
                        }
                        Ok(None) => {}
                        Err(e) => return Err(MetricSourceError::boxed(e)),
                    }
                }
                processor.clean().await.map_err(MetricSourceError::boxed)?;
                Ok(processor)
            });

            loop {
                match rx.recv().await {
                    Ok(()) => {
                        let snapshot = sensor.measure().await.map_err(MetricSourceError::boxed)?;
                        sensor_tx
                            .send(snapshot)
                            .await
                            .map_err(MetricSourceError::boxed)?;
                    }
                    Err(e) => match e {
                        RecvError::Closed => break,
                        RecvError::Lagged(n) => println!("WARN: {n} lagged"),
                    },
                }
            }

            sensor.join().await.map_err(MetricSourceError::boxed)?;
            sensor.clean().await.map_err(MetricSourceError::boxed)?;
            drop(sensor_tx);

            let processor = processor_task.await.map_err(MetricSourceError::boxed)??;

            Ok(Box::new(MetricSourceRuntime::<S> { sensor, processor })
                as Box<dyn MetricSourceWrapper>)
        })
    }

    fn list_sensors(&self) -> Result<Sensors, MetricSourceError> {
        self.sensor.list_sensors().map_err(MetricSourceError::boxed)
    }
}
