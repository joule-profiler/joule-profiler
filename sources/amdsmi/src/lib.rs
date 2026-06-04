use std::{collections::HashMap, sync::Arc, time::Duration};

use bitflags::bitflags;
use futures::StreamExt;
use joule_profiler_core::{
    sensor::{Sensor, Sensors},
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

/// Polling task handle and its cancellation token.
type WorkerHandle = (CancellationToken, JoinHandle<Result<()>>);

const MICRO_JOULE_UNIT: MetricUnit = MetricUnit {
    prefix: UnitPrefix::Micro,
    unit: Unit::Joule,
};

const BYTE_UNIT: MetricUnit = MetricUnit {
    prefix: UnitPrefix::None,
    unit: Unit::Byte,
};

bitflags! {
    /// The supports of a device.
    #[derive(Debug, Clone, Copy)]
    struct ProcessorSupport: u8 {
        const Energy = 1;
        const Power = 1 << 1;
        const Vram = 1 << 2;
    }
}

#[derive(Debug, Clone)]
pub struct Processor {
    /// The UUID of the device.
    uuid: UUID,

    /// The supports of the device (e.g., Energy, Power, VRAM).
    support: ProcessorSupport,
}

pub struct AmdSmiSource<H: Hardware> {
    /// Source configuration.
    config: AmdSmiConfig,

    /// The hardware used for querying AMD SMI lib. Used for testing.
    hardware: Arc<H>,

    /// The handle to the polling task and its cancellation token.
    handle: Option<WorkerHandle>,

    /// Map of GPU devices, the key is an index to avoid cloning the devices UUID.
    processors: Arc<HashMap<usize, Processor>>,

    /// The current energy counters.
    energy_counters: HashMap<usize, EnergyCounter>,

    /// The current vram counters.
    vram_counters: Arc<Mutex<HashMap<usize, VramCounter>>>,

    /// The current power counters.
    power_counters: Arc<Mutex<HashMap<usize, PowerCounter>>>,
}

impl AmdSmiSource<AmdSmi> {
    /// Initializes the AMD SMI source and retrieve the GPU devices.
    pub fn new(config: AmdSmiConfig) -> Result<Self> {
        let mut amdsmi = AmdSmi::new()?;
        let processors = amdsmi
            .init_processors(config.gpus_spec.as_ref())?
            .into_iter()
            .enumerate()
            .collect();

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
    /// Creates the worker task for power and vram polling at the specified polling interval.
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

    /// Reads the power and vram counters for each processors and updates the current counters.
    async fn read_polled_counters(
        hardware: &Arc<H>,
        processors: &Arc<HashMap<usize, Processor>>,
        power_counters: &Arc<Mutex<HashMap<usize, PowerCounter>>>,
        vram_counters: &Arc<Mutex<HashMap<usize, VramCounter>>>,
    ) -> Result<()> {
        let mut vram_updates = Vec::new();
        let mut power_updates = Vec::new();

        for (index, processor) in processors.iter() {
            if processor.support.contains(ProcessorSupport::Vram) {
                vram_updates.push((*index, hardware.get_vram_usage(processor)?));
            }
            if processor.support.contains(ProcessorSupport::Power) {
                power_updates.push((*index, hardware.get_power(processor)?));
            }
        }

        {
            let mut lock = vram_counters.lock().await;
            for (index, vram_usage) in vram_updates {
                lock.entry(index).and_modify(|c| c.update(vram_usage));
            }
        }

        {
            let mut lock = power_counters.lock().await;
            for (index, power) in power_updates {
                lock.entry(index).and_modify(|c| c.push(power));
            }
        }

        Ok(())
    }
}

impl<H: Hardware> MetricReader for AmdSmiSource<H> {
    type Type = HashMap<usize, Counter>;

    type Error = AmdSmiError;

    /// Makes a measurement for every devices.
    async fn measure(&mut self) -> Result<()> {
        for (index, processor) in self.processors.iter() {
            if processor.support.contains(ProcessorSupport::Energy) {
                let energy = self.hardware.get_energy_count(processor)?;
                self.energy_counters
                    .entry(*index)
                    .or_default()
                    .update(energy);
            }
        }
        Self::read_polled_counters(
            &self.hardware,
            &self.processors,
            &self.power_counters,
            &self.vram_counters,
        )
        .await?;
        Ok(())
    }

    /// Retrieve the current counters and reset them for the next phase.
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

    /// Creates the polling task if a polling interval has been configured.
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

    /// Joins the polling task if it exists.
    async fn join(&mut self) -> Result<()> {
        if let Some((cancellation_token, handle)) = self.handle.take() {
            debug!("Joining AMD SMI source polling task.");
            cancellation_token.cancel();
            handle.await??;
        }
        Ok(())
    }

    fn get_sensors(&self) -> Result<Sensors> {
        let sensors = self
            .processors
            .values()
            .flat_map(|p| {
                let mut processor_sensors = Vec::new();
                let uuid = &p.uuid;

                if p.support.contains(ProcessorSupport::Energy)
                    || p.support.contains(ProcessorSupport::Power)
                {
                    processor_sensors.push(Sensor::new(
                        format!("GPU-{uuid}-energy"),
                        MICRO_JOULE_UNIT,
                        Self::get_name(),
                    ));
                }

                if p.support.contains(ProcessorSupport::Vram) {
                    processor_sensors.push(Sensor::new(
                        format!("GPU-{uuid}-vram_min"),
                        BYTE_UNIT,
                        Self::get_name(),
                    ));
                    processor_sensors.push(Sensor::new(
                        format!("GPU-{uuid}-vram_max"),
                        BYTE_UNIT,
                        Self::get_name(),
                    ));
                }

                processor_sensors
            })
            .collect();
        Ok(sensors)
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

                let energy = counter
                    .energy
                    .map_or_else(|| counter.power.map(|c| c.compute_energy()), |c| c.diff());

                if let Some(energy) = energy {
                    processor_metrics.push(Metric::new(
                        format!("GPU-{uuid}-energy"),
                        energy,
                        MICRO_JOULE_UNIT,
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
                        BYTE_UNIT,
                        Self::get_name(),
                    ));

                    processor_metrics.push(Metric::new(
                        format!("GPU-{uuid}-vram_max"),
                        max,
                        BYTE_UNIT,
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
