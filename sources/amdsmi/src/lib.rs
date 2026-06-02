use std::{collections::HashMap, sync::Arc, time::Duration};

use bitflags::bitflags;
use futures::StreamExt;
use joule_profiler_core::{
    sensor::Sensors,
    source::MetricReader,
    types::{Metric, Metrics},
    unit::{MetricUnit, Unit, UnitPrefix},
};
use log::{debug, trace};
use tokio::{sync::Mutex, task::JoinHandle};
use tokio_timerfd::Interval;
use tokio_util::sync::CancellationToken;

use crate::{
    config::AmdSmiConfig,
    counters::{Counter, EnergyCounter, PowerCounter, VramCounter},
    error::AmdSmiError::{self},
    hardware::{AmdSmi, Hardware},
};

pub mod config;
pub mod counters;
pub mod error;
mod hardware;

pub type UUID = String;

type Result<T> = std::result::Result<T, AmdSmiError>;
pub(crate) type WorkerHandle = (CancellationToken, JoinHandle<Result<()>>);

bitflags! {
    #[derive(Debug, Clone, Copy)]
    struct ProcessorSupport: u8 {
        const Energy = 1;
        const Power = 1 << 1;
        const Vram = 1 << 2;
    }
}

#[derive(Debug, Clone)]
pub struct Processor {
    uuid: UUID,
    support: ProcessorSupport,
}

pub struct AmdSmiSource<H: Hardware> {
    config: AmdSmiConfig,
    hardware: Arc<H>,
    handle: Option<WorkerHandle>,
    processors: Arc<HashMap<usize, Processor>>,
    energy_counters: HashMap<usize, EnergyCounter>,
    vram_counters: Arc<Mutex<HashMap<usize, VramCounter>>>,
    power_counters: Arc<Mutex<HashMap<usize, PowerCounter>>>,
}

impl AmdSmiSource<AmdSmi> {
    pub fn new(config: AmdSmiConfig) -> Result<Self> {
        let amdsmi = AmdSmi::new()?;
        let processors = amdsmi.get_processors()?.into_iter().enumerate().collect();

        Ok(Self {
            config,
            hardware: Arc::new(amdsmi),
            handle: None,
            processors: Arc::new(processors),
            energy_counters: HashMap::new(),
            vram_counters: Arc::default(),
            power_counters: Arc::default(),
        })
    }
}

impl<H: Hardware> AmdSmiSource<H> {
    pub fn create_worker(
        hardware: Arc<H>,
        processors: Arc<HashMap<usize, Processor>>,
        power_counters: Arc<Mutex<HashMap<usize, PowerCounter>>>,
        vram_counters: Arc<Mutex<HashMap<usize, VramCounter>>>,
        poll_interval: Duration,
    ) -> Result<WorkerHandle> {
        let mut ticker = Interval::new_interval(poll_interval)?;

        let cancellation_token = CancellationToken::new();
        let cancellation_token_clone = cancellation_token.clone();

        let handle = tokio::spawn(async move {
            debug!("Starting AMD SMI source polling.");

            loop {
                tokio::select! {
                    _ = ticker.next() => {
                        trace!("Polled AMD SMI source.");
                        Self::read_polled_counters(&hardware, &processors, &power_counters, &vram_counters).await?;
                    }

                    () = cancellation_token.cancelled() => {
                        debug!("AMD SMI worker stopped.");
                        break;
                    }
                }
            }

            Ok(())
        });

        Ok((cancellation_token_clone, handle))
    }

    async fn read_polled_counters(
        hardware: &Arc<H>,
        processors: &Arc<HashMap<usize, Processor>>,
        power_counters: &Arc<Mutex<HashMap<usize, PowerCounter>>>,
        vram_counters: &Arc<Mutex<HashMap<usize, VramCounter>>>,
    ) -> Result<()> {
        for (index, processor) in processors.iter() {
            if processor.support.contains(ProcessorSupport::Vram) {
                let mut lock = vram_counters.lock().await;
                let vram_usage = hardware.get_vram_usage(processor)?;
                lock.entry(*index).and_modify(|c| c.update(vram_usage));
            }

            if processor.support.contains(ProcessorSupport::Vram) {
                let mut lock = power_counters.lock().await;
                let power = hardware.get_power(processor)?;
                lock.entry(*index).and_modify(|c| c.update(power));
            }
        }
        Ok(())
    }
}

impl<H: Hardware> MetricReader for AmdSmiSource<H> {
    type Type = HashMap<usize, Counter>;

    type Error = AmdSmiError;

    async fn measure(&mut self) -> Result<()> {
        for (index, processor) in self.processors.iter() {
            if processor.support.contains(ProcessorSupport::Energy) {
                let energy = self.hardware.get_energy_count(processor)?;
                self.energy_counters
                    .entry(*index)
                    .or_default()
                    .update(energy);
            } else if processor.support.contains(ProcessorSupport::Power) {
                let mut lock = self.power_counters.lock().await;
                let power = self.hardware.get_power(processor)?;
                lock.entry(*index).or_default().update(power);
            }

            if processor.support.contains(ProcessorSupport::Vram) {
                let mut lock = self.vram_counters.lock().await;
                let vram_usage = self.hardware.get_vram_usage(processor)?;
                lock.entry(*index).or_default().update(vram_usage);
            }
        }

        Ok(())
    }

    async fn retrieve(&mut self) -> Result<Self::Type> {
        let mut energy_counters = self.energy_counters.clone();
        for counter in self.energy_counters.values_mut() {
            counter.reset();
        }

        let mut lock = self.vram_counters.lock().await;
        let mut vram_counters = lock.clone();
        for counter in lock.values_mut() {
            counter.reset();
        }

        let mut lock = self.power_counters.lock().await;
        let mut power_counters = lock.clone();
        for counter in lock.values_mut() {
            counter.reset();
        }

        let map = self
            .processors
            .keys()
            .map(|index| {
                let energy = energy_counters.remove(index);
                let vram = vram_counters.remove(index);
                let power = power_counters.remove(index);
                let counter = Counter {
                    energy,
                    vram,
                    power,
                };
                (*index, counter)
            })
            .collect();

        Ok(map)
    }

    async fn init(&mut self, _pid: i32) -> Result<()> {
        if let Some(poll_interval) = self.config.poll_interval {
            self.handle = Some(Self::create_worker(
                self.hardware.clone(),
                self.processors.clone(),
                self.power_counters.clone(),
                self.vram_counters.clone(),
                poll_interval,
            )?);
        }

        debug!("AMD SMI source initialized.");
        Ok(())
    }

    async fn join(&mut self) -> Result<()> {
        if let Some((cancellation_token, handle)) = self.handle.take() {
            debug!("Joining AMD SMI source polling task.");
            cancellation_token.cancel();
            handle.await??;
        }
        Ok(())
    }

    fn get_sensors(&self) -> Result<Sensors> {
        todo!()
    }

    fn to_metrics(&self, result: Self::Type) -> Result<Metrics> {
        let metrics = result
            .into_iter()
            .flat_map(|(index, counter)| {
                let uuid = &self
                    .processors
                    .get(&index)
                    .ok_or(AmdSmiError::NoSuchDeviceFromIndex(index))?
                    .uuid;

                let mut processor_metrics = Vec::new();
                if let Some(energy) = counter.energy
                    && let Some(energy) = energy.diff()
                {
                    processor_metrics.push(Metric::new(
                        format!("GPU-{uuid}-energy"),
                        energy,
                        MetricUnit {
                            prefix: UnitPrefix::Micro,
                            unit: Unit::Joule,
                        },
                        Self::get_name(),
                    ));
                } else if let Some(power) = counter.power {
                    let energy = power.compute_energy();
                    processor_metrics.push(Metric::new(
                        format!("GPU-{uuid}-energy"),
                        energy,
                        MetricUnit {
                            prefix: UnitPrefix::Micro,
                            unit: Unit::Joule,
                        },
                        Self::get_name(),
                    ));
                }

                if let Some(vram) = counter.vram
                    && let Some(min) = vram.min
                    && let Some(max) = vram.max
                {
                    processor_metrics.push(Metric::new(
                        format!("GPU-{uuid}-vram_min"),
                        min,
                        MetricUnit {
                            prefix: UnitPrefix::None,
                            unit: Unit::Byte,
                        },
                        Self::get_name(),
                    ));

                    processor_metrics.push(Metric::new(
                        format!("GPU-{uuid}-vram_max"),
                        max,
                        MetricUnit {
                            prefix: UnitPrefix::None,
                            unit: Unit::Byte,
                        },
                        Self::get_name(),
                    ));
                }

                Ok::<Metrics, AmdSmiError>(processor_metrics)
            })
            .flatten()
            .collect();
        Ok(metrics)
    }

    fn get_name() -> &'static str {
        "amdsmi"
    }
}
