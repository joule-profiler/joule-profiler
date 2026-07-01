use std::{collections::HashMap, error::Error};

use smallvec::SmallVec;
use thiserror::Error;
use tokio::sync::mpsc;
use tokio::{select, task::JoinHandle};

use crate::types::{Metrics, Phase, PhaseInfo};

#[derive(Debug, Error)]
#[error("exporter error")]
pub struct ExporterError(#[from] Box<dyn std::error::Error + Sync + Send>);

impl ExporterError {
    pub fn boxed<E: Error + Send + Sync + 'static>(error: E) -> Self {
        Self(Box::new(error))
    }
}

pub trait Exporter: Send + 'static {
    type Error: Error + Send + Sync;

    fn export(
        &mut self,
        phases_info: PhaseInfo,
        metrics: Metrics,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send;

    fn finish(&mut self) -> impl Future<Output = Result<(), Self::Error>> + Send {
        async { Ok(()) }
    }

    fn clean(&mut self) -> impl Future<Output = Result<(), Self::Error>> + Send {
        async { Ok(()) }
    }
}

pub(crate) trait ExporterWrapper: Send {
    fn run(
        self: Box<Self>,
        nb_sources: usize,
        metrics_receiver: mpsc::Receiver<Phase>,
        phases_receiver: mpsc::Receiver<PhaseInfo>,
    ) -> JoinHandle<Result<Box<dyn ExporterWrapper>, ExporterError>>;
}

impl<E: Exporter> ExporterWrapper for ExporterRuntime<E> {
    fn run(
        self: Box<Self>,
        nb_sources: usize,
        mut metrics_receiver: mpsc::Receiver<Phase>,
        mut phases_receiver: mpsc::Receiver<PhaseInfo>,
    ) -> JoinHandle<Result<Box<dyn ExporterWrapper>, ExporterError>> {
        let mut exporter = self.exporter;
        tokio::spawn(async move {
            let mut pending_metrics: HashMap<usize, (Metrics, usize)> = HashMap::new();
            let mut pending_info: HashMap<usize, PhaseInfo> = HashMap::new();

            loop {
                select! {
                    Some(phase) = metrics_receiver.recv() => {
                        let entry = pending_metrics
                            .entry(phase.phase_index)
                            .or_insert_with(|| (SmallVec::new(), 0));
                        entry.0.extend(phase.metrics);
                        entry.1 += 1;

                        if entry.1 == nb_sources && let Some(phase_info) = pending_info.remove(&phase.phase_index) && let Some((metrics, _)) = pending_metrics.remove(&phase.phase_index) {
                            exporter.export(phase_info, metrics).await.map_err(ExporterError::boxed)?;
                        }
                    },
                    Some(phase_info) = phases_receiver.recv() => {
                        let ready = pending_metrics
                            .get(&phase_info.phase_index)
                            .is_some_and(|(_, count)| *count == nb_sources);

                        if ready && let Some((metrics, _)) = pending_metrics.remove(&phase_info.phase_index) {
                            exporter.export(phase_info, metrics).await.map_err(ExporterError::boxed)?;
                        } else {
                            pending_info.insert(phase_info.phase_index, phase_info);
                        }
                    },
                    else => break
                }
            }
            exporter.finish().await.map_err(ExporterError::boxed)?;
            exporter.clean().await.map_err(ExporterError::boxed)?;
            Ok(exporter.into())
        })
    }
}

pub(crate) struct ExporterRuntime<E: Exporter> {
    exporter: E,
}

impl<E: Exporter> From<E> for Box<dyn ExporterWrapper> {
    fn from(exporter: E) -> Self {
        Box::new(ExporterRuntime { exporter })
    }
}
