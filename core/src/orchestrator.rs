use futures::future::join_all;
use smallvec::SmallVec;
use thiserror::Error;
use tokio::sync;
use tokio::sync::mpsc;
use tokio::task::{self, JoinHandle};

use crate::exporter::{ExporterError, ExporterWrapper};
use crate::source::{MetricSourceError, MetricSourceWrapper};
use crate::types::{InitContext, Phase, PhaseInfo};
use crate::util::time::get_timestamp_micros;

const DEFAULT_CONTROL_CHANNEL_SIZE: usize = 16;
const DEFAULT_PHASES_CHANNEL_SIZE: usize = 1024;
const DEFAULT_SOURCES_LENGTH: usize = 4;

pub struct Orchestrator<State> {
    state: State,
}

type SourceHandle = JoinHandle<Result<Box<dyn MetricSourceWrapper>, MetricSourceError>>;
type Handles = SmallVec<[SourceHandle; DEFAULT_SOURCES_LENGTH]>;
type ExporterHandle = JoinHandle<Result<Box<dyn ExporterWrapper>, ExporterError>>;

struct ExporterTask {
    metrics_sender: mpsc::Sender<Phase>,
    phases_sender: mpsc::Sender<PhaseInfo>,
    handle: ExporterHandle,
}

pub struct OrchestratorIdleState {
    sources: Vec<Box<dyn MetricSourceWrapper>>,
    exporter: Box<dyn ExporterWrapper>,
}

pub struct OrchestratorReadyState {
    sources: Vec<Box<dyn MetricSourceWrapper>>,
    exporter_task: ExporterTask,
}

struct PendingPhase {
    phase_index: usize,
    start_token: String,
    start_timestamp: u128,
}

pub struct OrchestratorRunningState {
    handles: Handles,
    exporter_task: ExporterTask,
    current_phase_index: usize,
    pending_phase: Option<PendingPhase>,
    phases_tx: sync::broadcast::Sender<()>,
}

#[derive(Debug, Error)]
pub enum OrchestratorError {
    #[error("source error")]
    MetricSource(#[from] MetricSourceError),

    #[error("exporter error")]
    Exporter(#[from] ExporterError),

    #[error(transparent)]
    Send(#[from] sync::broadcast::error::SendError<()>),

    #[error(transparent)]
    Join(#[from] task::JoinError),
}

impl Orchestrator<OrchestratorIdleState> {
    pub fn new(
        exporter: Box<dyn ExporterWrapper>,
        sources: Vec<Box<dyn MetricSourceWrapper>>,
    ) -> Self {
        Self {
            state: OrchestratorIdleState { sources, exporter },
        }
    }

    pub async fn pre_init(
        mut self,
    ) -> Result<Orchestrator<OrchestratorReadyState>, OrchestratorError> {
        let nb_sources = self.state.sources.len();
        join_all(
            self.state
                .sources
                .iter_mut()
                .map(|source| source.pre_init()),
        )
        .await
        .into_iter()
        .collect::<Result<Vec<()>, MetricSourceError>>()?;

        let (metrics_sender, metrics_receiver) =
            mpsc::channel::<Phase>(DEFAULT_PHASES_CHANNEL_SIZE);
        let (phases_sender, phases_receiver) =
            mpsc::channel::<PhaseInfo>(DEFAULT_PHASES_CHANNEL_SIZE);
        let handle = self
            .state
            .exporter
            .run(nb_sources, metrics_receiver, phases_receiver);

        Ok(Orchestrator {
            state: OrchestratorReadyState {
                sources: self.state.sources,
                exporter_task: ExporterTask {
                    metrics_sender,
                    phases_sender,
                    handle,
                },
            },
        })
    }
}

impl Orchestrator<OrchestratorReadyState> {
    pub async fn run(
        mut self,
        ctx: InitContext,
    ) -> Result<Orchestrator<OrchestratorRunningState>, OrchestratorError> {
        join_all(self.state.sources.iter_mut().map(|source| source.init(ctx)))
            .await
            .into_iter()
            .collect::<Result<Vec<()>, MetricSourceError>>()?;

        let (tx, rx) = sync::broadcast::channel(DEFAULT_CONTROL_CHANNEL_SIZE);

        let handles = self
            .state
            .sources
            .into_iter()
            .map(|source| {
                let exporter_tx = self.state.exporter_task.metrics_sender.clone();
                source.run(rx.resubscribe(), exporter_tx)
            })
            .collect();

        Ok(Orchestrator {
            state: OrchestratorRunningState {
                handles,
                exporter_task: self.state.exporter_task,
                current_phase_index: 0,
                pending_phase: None,
                phases_tx: tx,
            },
        })
    }
}

impl Orchestrator<OrchestratorRunningState> {
    pub async fn join(
        self,
    ) -> Result<(Vec<Box<dyn MetricSourceWrapper>>, Box<dyn ExporterWrapper>), OrchestratorError>
    {
        drop(self.state.phases_tx);

        let mut sources = Vec::with_capacity(self.state.handles.len());
        for source_handle in self.state.handles {
            match source_handle.await {
                Ok(Ok(source)) => sources.push(source),
                Ok(Err(e)) => {
                    return Err(OrchestratorError::from(e));
                }
                Err(e) => return Err(e.into()),
            }
        }

        drop(self.state.exporter_task.metrics_sender);
        drop(self.state.exporter_task.phases_sender);

        let exporter = self.state.exporter_task.handle.await??;

        Ok((sources, exporter))
    }

    pub async fn measure(&mut self, token: String) -> Result<(), OrchestratorError> {
        let timestamp = get_timestamp_micros();
        self.state.phases_tx.send(())?;

        if let Some(pending_phase) = self.state.pending_phase.take() {
            let phase_info = PhaseInfo {
                phase_index: pending_phase.phase_index,
                start_token: pending_phase.start_token,
                start_timestamp: pending_phase.start_timestamp,
                end_token: token.clone(),
                end_timestamp: timestamp,
            };
            self.state
                .exporter_task
                .phases_sender
                .send(phase_info)
                .await
                .map_err(ExporterError::boxed)?;
        }

        self.state.pending_phase = Some(PendingPhase {
            phase_index: self.state.current_phase_index,
            start_token: token,
            start_timestamp: timestamp,
        });
        self.state.current_phase_index += 1;
        Ok(())
    }
}
