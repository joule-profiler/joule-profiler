//! AMD SMI (AMD System Management Interface) energy profiling integration for Joule Profiler.
//!
//! This module provides energy consumption, VRAM usage and GPU utilization metrics for AMD GPUs using the AMD SMI library.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Duration,
};

use bitflags::bitflags;
use futures::{StreamExt, future::try_join_all};
use joule_profiler_core::{
    sensor::{Sensor, Sensors},
    source::MetricReader,
    types::{Metric, Metrics},
    unit::{MetricUnit, Unit, UnitPrefix},
};
use log::{debug, trace};
use tokio::task::{JoinHandle, spawn_blocking};
use tokio_timerfd::Interval;
use tokio_util::sync::CancellationToken;

use crate::{
    config::AmdSmiConfig,
    counters::{Counter, EnergyCounter, PowerCounter, UtilizationCounter, VramCounter},
    error::AmdSmiError::{self},
    hardware::{AmdSmiHardware, AmdSmiWrapperHardware},
};

pub mod config;
pub mod counters;
pub mod error;
mod hardware;

pub type UUID = String;

type Result<T> = std::result::Result<T, AmdSmiError>;

/// Polling task handle and its cancellation token.
type WorkerHandle = (CancellationToken, JoinHandle<Result<()>>);

const AMDSMI_SOURCE_NAME: &str = "amdsmi";

const MICRO_JOULE_UNIT: MetricUnit = MetricUnit {
    prefix: UnitPrefix::Micro,
    unit: Unit::Joule,
};

const BYTE_UNIT: MetricUnit = MetricUnit {
    prefix: UnitPrefix::None,
    unit: Unit::Byte,
};

const PERCENT_UNIT: MetricUnit = MetricUnit {
    prefix: UnitPrefix::None,
    unit: Unit::Percent,
};

bitflags! {
    /// The supports of a device.
    #[derive(Debug, Clone, Copy)]
    struct ProcessorSupport: u8 {
        const Energy = 1;
        const Power = 1 << 1;
        const Vram = 1 << 2;
        const Utilization = 1 << 3;
    }
}

#[derive(Debug, Clone)]
pub struct Processor {
    /// The UUID of the device.
    uuid: UUID,

    /// The supports of the device (e.g., Energy, Power, VRAM).
    support: ProcessorSupport,
}

pub struct AmdSmi<H: AmdSmiHardware = AmdSmiWrapperHardware> {
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

    /// The current GPU utilization counters.
    utilization_counters: Arc<Mutex<HashMap<usize, UtilizationCounter>>>,
}

impl<H: AmdSmiHardware> AmdSmi<H> {
    /// Creates the worker task for power and vram polling at the specified polling interval.
    pub fn create_worker(
        hardware: Arc<H>,
        processors: Arc<HashMap<usize, Processor>>,
        power_counters: Arc<Mutex<HashMap<usize, PowerCounter>>>,
        vram_counters: Arc<Mutex<HashMap<usize, VramCounter>>>,
        usage_counters: Arc<Mutex<HashMap<usize, UtilizationCounter>>>,
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
                        Self::read_polled_counters(&hardware, &processors, &power_counters, &vram_counters, &usage_counters).await?;
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
    ///
    /// Spawn every blocking read upfront so they run concurrently on the
    /// blocking thread pool, instead of awaiting them one by one.
    async fn read_polled_counters(
        hardware: &Arc<H>,
        processors: &Arc<HashMap<usize, Processor>>,
        power_counters: &Arc<Mutex<HashMap<usize, PowerCounter>>>,
        vram_counters: &Arc<Mutex<HashMap<usize, VramCounter>>>,
        utilization_counters: &Arc<Mutex<HashMap<usize, UtilizationCounter>>>,
    ) -> Result<()> {
        let mut vram_tasks = Vec::new();
        let mut power_tasks = Vec::new();
        let mut utilization_tasks = Vec::new();

        for (index, processor) in processors.iter() {
            let index = *index;

            if processor.support.contains(ProcessorSupport::Vram) {
                let hardware = hardware.clone();
                let processor = processor.clone();
                vram_tasks.push(spawn_blocking(move || {
                    hardware
                        .get_vram_usage(&processor)
                        .map(|vram| (index, vram))
                }));
            }
            if processor.support.contains(ProcessorSupport::Power) {
                let hardware = hardware.clone();
                let processor = processor.clone();
                power_tasks.push(spawn_blocking(move || {
                    hardware.get_power(&processor).map(|power| (index, power))
                }));
            }
            if processor.support.contains(ProcessorSupport::Utilization) {
                let hardware = hardware.clone();
                let processor = processor.clone();
                utilization_tasks.push(spawn_blocking(move || {
                    hardware
                        .get_gpu_activity(&processor)
                        .map(|usage| (index, usage.gpu_usage))
                }));
            }
        }

        let vram_updates = try_join_all(vram_tasks).await?;
        let power_updates = try_join_all(power_tasks).await?;
        let utilization_updates = try_join_all(utilization_tasks).await?;

        {
            let mut lock = vram_counters
                .lock()
                .map_err(|_| AmdSmiError::MutexPoisoned)?;
            for (index, vram_usage) in vram_updates.into_iter().collect::<Result<Vec<_>>>()? {
                lock.entry(index).or_default().update(vram_usage);
            }
        }

        {
            let mut lock = power_counters
                .lock()
                .map_err(|_| AmdSmiError::MutexPoisoned)?;
            for (index, power) in power_updates.into_iter().collect::<Result<Vec<_>>>()? {
                lock.entry(index).or_default().push(power);
            }
        }

        {
            let mut lock = utilization_counters
                .lock()
                .map_err(|_| AmdSmiError::MutexPoisoned)?;
            for (index, utilization) in utilization_updates
                .into_iter()
                .collect::<Result<Vec<_>>>()?
            {
                lock.entry(index).or_default().update(utilization);
            }
        }

        Ok(())
    }
}

impl<H: AmdSmiHardware> MetricReader for AmdSmi<H> {
    type Type = HashMap<usize, Counter>;
    type Error = AmdSmiError;
    type Config = AmdSmiConfig;

    /// Initializes the AMD SMI source and retrieve the GPU devices.
    fn from_config(config: AmdSmiConfig) -> Result<Self> {
        let mut amdsmi = H::new()?;
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
            utilization_counters: Arc::default(),
        })
    }

    /// Makes a measurement for every devices.
    ///
    /// Spawns the different measures in the blocking thread pool to execute
    /// them in parallel without blocking the async pool.
    async fn measure(&mut self) -> Result<()> {
        let mut energy_tasks = Vec::new();
        for (index, processor) in self.processors.iter() {
            if processor.support.contains(ProcessorSupport::Energy) {
                let hardware = self.hardware.clone();
                let processor = processor.clone();
                let index = *index;
                energy_tasks.push(spawn_blocking(move || {
                    hardware
                        .get_energy_count(&processor)
                        .map(|energy| (index, energy))
                }));
            }
        }

        Self::read_polled_counters(
            &self.hardware,
            &self.processors,
            &self.power_counters,
            &self.vram_counters,
            &self.utilization_counters,
        )
        .await?;

        for (index, energy) in try_join_all(energy_tasks)
            .await?
            .into_iter()
            .collect::<Result<Vec<_>>>()?
        {
            self.energy_counters
                .entry(index)
                .or_default()
                .update(energy);
        }

        Ok(())
    }

    /// Retrieve the current counters and reset them for the next phase.
    async fn retrieve(&mut self) -> Result<Self::Type> {
        let mut energy_counters = self.energy_counters.clone();
        for counter in self.energy_counters.values_mut() {
            counter.reset();
        }

        let mut lock = self
            .vram_counters
            .lock()
            .map_err(|_| AmdSmiError::MutexPoisoned)?;
        let mut vram_counters = lock.clone();
        for counter in lock.values_mut() {
            counter.reset();
        }

        let mut lock = self
            .power_counters
            .lock()
            .map_err(|_| AmdSmiError::MutexPoisoned)?;
        let mut power_counters = lock.clone();
        for counter in lock.values_mut() {
            counter.reset();
        }

        let mut lock = self
            .utilization_counters
            .lock()
            .map_err(|_| AmdSmiError::MutexPoisoned)?;
        let mut utilization_counters = lock.clone();
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
                let utilization = utilization_counters.remove(index);
                let counter = Counter {
                    energy,
                    vram,
                    power,
                    utilization,
                };
                (*index, counter)
            })
            .collect();

        Ok(map)
    }

    /// Creates the polling task if a polling interval has been configured.
    async fn init(&mut self, _pid: i32) -> Result<()> {
        self.handle = Some(Self::create_worker(
            self.hardware.clone(),
            self.processors.clone(),
            self.power_counters.clone(),
            self.vram_counters.clone(),
            self.utilization_counters.clone(),
            self.config.poll_interval,
        )?);

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

                if p.support.contains(ProcessorSupport::Utilization) {
                    processor_sensors.push(Sensor::new(
                        format!("GPU-{uuid}-utilization_min"),
                        PERCENT_UNIT,
                        Self::get_name(),
                    ));
                    processor_sensors.push(Sensor::new(
                        format!("GPU-{uuid}-utilization_max"),
                        PERCENT_UNIT,
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

                if let Some(utilization) = counter.utilization
                    && let Some(min) = utilization.min
                    && let Some(max) = utilization.max
                {
                    processor_metrics.push(Metric::new(
                        format!("GPU-{uuid}-utilization_min"),
                        u64::from(min),
                        PERCENT_UNIT,
                        Self::get_name(),
                    ));

                    processor_metrics.push(Metric::new(
                        format!("GPU-{uuid}-utilization_max"),
                        u64::from(max),
                        PERCENT_UNIT,
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
        AMDSMI_SOURCE_NAME
    }

    fn get_id() -> &'static str {
        AMDSMI_SOURCE_NAME
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, sync::Arc, time::Duration};

    use amdsmi::types::{EnergyCount, GpuUsageInfo};
    use joule_profiler_core::source::MetricReader;
    use mockall::predicate::function;
    use tokio::time::sleep;

    use crate::{
        AmdSmi, Processor, ProcessorSupport,
        config::AmdSmiConfig,
        counters::{Counter, PowerMeasurement},
        error::AmdSmiError,
        hardware::MockAmdSmiHardware,
    };

    fn make_processor(uuid: &str, support: ProcessorSupport) -> Processor {
        Processor {
            uuid: uuid.to_owned(),
            support,
        }
    }

    fn processors_map(list: Vec<Processor>) -> Arc<HashMap<usize, Processor>> {
        Arc::new(list.into_iter().enumerate().collect())
    }

    fn build_amdsmi(
        hardware: MockAmdSmiHardware,
        processors: Arc<HashMap<usize, Processor>>,
        poll_interval: Duration,
    ) -> AmdSmi<MockAmdSmiHardware> {
        AmdSmi {
            config: AmdSmiConfig {
                poll_interval,
                gpus_spec: None,
            },
            hardware: Arc::new(hardware),
            handle: None,
            processors,
            energy_counters: HashMap::default(),
            vram_counters: Arc::default(),
            power_counters: Arc::default(),
            utilization_counters: Arc::default(),
        }
    }

    fn energy_count(value: u64) -> EnergyCount {
        EnergyCount {
            energy_accumulator: value,
            counter_resolution: 1.0,
            timestamp: 0,
        }
    }

    fn gpu_usage(gpu_usage: u32) -> GpuUsageInfo {
        GpuUsageInfo {
            gpu_usage,
            mem_usage: 0,
        }
    }

    fn power_measurement(power: u32) -> PowerMeasurement {
        PowerMeasurement {
            timestamp: 0,
            power,
        }
    }

    fn by_uuid(uuid: &'static str) -> impl mockall::Predicate<Processor> {
        function(move |p: &Processor| p.uuid == uuid)
    }

    #[test]
    fn get_sensors_energy_support_emits_energy_sensor() {
        let processors = processors_map(vec![make_processor("UUID_0", ProcessorSupport::Energy)]);
        let amdsmi = build_amdsmi(
            MockAmdSmiHardware::default(),
            processors,
            Duration::from_secs(1),
        );

        let sensors = amdsmi.get_sensors().unwrap();
        assert!(sensors.iter().any(|s| s.name == "GPU-UUID_0-energy"));
    }

    #[test]
    fn get_sensors_power_support_emits_energy_sensor() {
        let processors = processors_map(vec![make_processor("UUID_0", ProcessorSupport::Power)]);
        let amdsmi = build_amdsmi(
            MockAmdSmiHardware::default(),
            processors,
            Duration::from_secs(1),
        );

        let sensors = amdsmi.get_sensors().unwrap();
        assert!(sensors.iter().any(|s| s.name == "GPU-UUID_0-energy"));
    }

    #[test]
    fn get_sensors_vram_support_emits_vram_sensors() {
        let processors = processors_map(vec![make_processor("UUID_0", ProcessorSupport::Vram)]);
        let amdsmi = build_amdsmi(
            MockAmdSmiHardware::default(),
            processors,
            Duration::from_secs(1),
        );

        let sensors = amdsmi.get_sensors().unwrap();
        assert!(sensors.iter().any(|s| s.name == "GPU-UUID_0-vram_min"));
        assert!(sensors.iter().any(|s| s.name == "GPU-UUID_0-vram_max"));
    }

    #[test]
    fn get_sensors_utilization_support_emits_utilization_sensors() {
        let processors = processors_map(vec![make_processor(
            "UUID_0",
            ProcessorSupport::Utilization,
        )]);
        let amdsmi = build_amdsmi(
            MockAmdSmiHardware::default(),
            processors,
            Duration::from_secs(1),
        );

        let sensors = amdsmi.get_sensors().unwrap();
        assert!(
            sensors
                .iter()
                .any(|s| s.name == "GPU-UUID_0-utilization_min")
        );
        assert!(
            sensors
                .iter()
                .any(|s| s.name == "GPU-UUID_0-utilization_max")
        );
    }

    #[test]
    fn get_sensors_empty_when_no_processors() {
        let amdsmi = build_amdsmi(
            MockAmdSmiHardware::default(),
            Arc::new(HashMap::new()),
            Duration::from_secs(1),
        );
        assert!(amdsmi.get_sensors().unwrap().is_empty());
    }

    #[test]
    fn get_sensors_counts_all_sensors_for_full_support() {
        let processors = processors_map(vec![make_processor(
            "UUID_0",
            ProcessorSupport::Energy | ProcessorSupport::Vram | ProcessorSupport::Utilization,
        )]);
        let amdsmi = build_amdsmi(
            MockAmdSmiHardware::default(),
            processors,
            Duration::from_secs(1),
        );
        assert_eq!(amdsmi.get_sensors().unwrap().len(), 5);
    }

    #[tokio::test]
    async fn measure_reads_energy_for_energy_capable_processor() {
        let processors = processors_map(vec![make_processor("UUID_0", ProcessorSupport::Energy)]);

        let mut hw = MockAmdSmiHardware::default();
        hw.expect_get_energy_count()
            .with(by_uuid("UUID_0"))
            .once()
            .returning(|_| Ok(energy_count(1_000_000)));

        let mut amdsmi = build_amdsmi(hw, processors, Duration::from_secs(1));
        amdsmi.measure().await.unwrap();

        assert!(amdsmi.energy_counters.contains_key(&0));
    }

    #[tokio::test]
    async fn measure_reads_power_for_power_capable_processor() {
        let processors = processors_map(vec![make_processor("UUID_0", ProcessorSupport::Power)]);

        let mut hw = MockAmdSmiHardware::default();
        hw.expect_get_power()
            .with(by_uuid("UUID_0"))
            .once()
            .returning(|_| Ok(power_measurement(150_000)));

        let mut amdsmi = build_amdsmi(hw, processors, Duration::from_secs(1));
        amdsmi.measure().await.unwrap();

        assert!(amdsmi.power_counters.lock().unwrap().contains_key(&0));
    }

    #[tokio::test]
    async fn measure_reads_vram_for_vram_capable_processor() {
        let processors = processors_map(vec![make_processor("UUID_0", ProcessorSupport::Vram)]);

        let mut hw = MockAmdSmiHardware::default();
        hw.expect_get_vram_usage()
            .with(by_uuid("UUID_0"))
            .once()
            .returning(|_| Ok(8_000_000_000));

        let mut amdsmi = build_amdsmi(hw, processors, Duration::from_secs(1));
        amdsmi.measure().await.unwrap();

        assert!(amdsmi.vram_counters.lock().unwrap().contains_key(&0));
    }

    #[tokio::test]
    async fn measure_reads_utilization_for_utilization_capable_processor() {
        let processors = processors_map(vec![make_processor(
            "UUID_0",
            ProcessorSupport::Utilization,
        )]);

        let mut hw = MockAmdSmiHardware::default();
        hw.expect_get_gpu_activity()
            .with(by_uuid("UUID_0"))
            .once()
            .returning(|_| Ok(gpu_usage(75)));

        let mut amdsmi = build_amdsmi(hw, processors, Duration::from_secs(1));
        amdsmi.measure().await.unwrap();

        assert!(amdsmi.utilization_counters.lock().unwrap().contains_key(&0));
    }

    #[tokio::test]
    async fn measure_skips_energy_and_power_for_vram_only_processor() {
        let processors = processors_map(vec![make_processor("UUID_0", ProcessorSupport::Vram)]);

        let mut hw = MockAmdSmiHardware::default();
        hw.expect_get_energy_count().never();
        hw.expect_get_power().never();
        hw.expect_get_vram_usage()
            .once()
            .returning(|_| Ok(1_000_000));

        let mut amdsmi = build_amdsmi(hw, processors, Duration::from_secs(1));
        amdsmi.measure().await.unwrap();

        assert!(amdsmi.energy_counters.is_empty());
        assert!(amdsmi.power_counters.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn measure_propagates_energy_error() {
        let processors = processors_map(vec![make_processor("UUID_0", ProcessorSupport::Energy)]);

        let mut hw = MockAmdSmiHardware::default();
        hw.expect_get_energy_count().once().returning(|_| {
            Err(AmdSmiError::AmdSmiError(
                amdsmi::error::AmdSmiError::NotSupported,
            ))
        });

        let mut amdsmi = build_amdsmi(hw, processors, Duration::from_secs(1));
        assert!(amdsmi.measure().await.is_err());
    }

    #[tokio::test]
    async fn measure_propagates_power_error() {
        let processors = processors_map(vec![make_processor("UUID_0", ProcessorSupport::Power)]);

        let mut hw = MockAmdSmiHardware::default();
        hw.expect_get_power().once().returning(|_| {
            Err(AmdSmiError::AmdSmiError(
                amdsmi::error::AmdSmiError::NotSupported,
            ))
        });

        let mut amdsmi = build_amdsmi(hw, processors, Duration::from_secs(1));
        assert!(amdsmi.measure().await.is_err());
    }

    #[tokio::test]
    async fn measure_propagates_vram_error() {
        let processors = processors_map(vec![make_processor("UUID_0", ProcessorSupport::Vram)]);

        let mut hw = MockAmdSmiHardware::default();
        hw.expect_get_vram_usage().once().returning(|_| {
            Err(AmdSmiError::AmdSmiError(
                amdsmi::error::AmdSmiError::NotSupported,
            ))
        });

        let mut amdsmi = build_amdsmi(hw, processors, Duration::from_secs(1));
        assert!(amdsmi.measure().await.is_err());
    }

    #[tokio::test]
    async fn measure_propagates_utilization_error() {
        let processors = processors_map(vec![make_processor(
            "UUID_0",
            ProcessorSupport::Utilization,
        )]);

        let mut hw = MockAmdSmiHardware::default();
        hw.expect_get_gpu_activity().once().returning(|_| {
            Err(AmdSmiError::AmdSmiError(
                amdsmi::error::AmdSmiError::NotSupported,
            ))
        });

        let mut amdsmi = build_amdsmi(hw, processors, Duration::from_secs(1));
        assert!(amdsmi.measure().await.is_err());
    }

    #[tokio::test]
    async fn retrieve_returns_counters_and_resets_them() {
        let processors = processors_map(vec![make_processor(
            "UUID_0",
            ProcessorSupport::Energy | ProcessorSupport::Vram,
        )]);

        let mut hw = MockAmdSmiHardware::default();
        hw.expect_get_energy_count()
            .returning(|_| Ok(energy_count(10_000)));
        hw.expect_get_vram_usage().returning(|_| Ok(1_000_000));

        let mut amdsmi = build_amdsmi(hw, processors, Duration::from_secs(1));
        amdsmi.measure().await.unwrap();
        amdsmi.measure().await.unwrap();

        let result = amdsmi.retrieve().await.unwrap();
        assert!(result.contains_key(&0));

        let result2 = amdsmi.retrieve().await.unwrap();
        let counter: &Counter = result2.get(&0).unwrap();
        assert!(
            counter
                .energy
                .as_ref()
                .and_then(super::counters::EnergyCounter::diff)
                .is_none_or(|v| v == 0)
        );
    }

    #[tokio::test]
    async fn retrieve_includes_entry_for_every_processor() {
        let processors = processors_map(vec![
            make_processor("UUID_0", ProcessorSupport::Energy),
            make_processor("UUID_1", ProcessorSupport::Power),
            make_processor("UUID_2", ProcessorSupport::Vram),
        ]);

        let mut hw = MockAmdSmiHardware::default();
        hw.expect_get_energy_count()
            .returning(|_| Ok(energy_count(1_000)));
        hw.expect_get_power()
            .returning(|_| Ok(power_measurement(5_000)));
        hw.expect_get_vram_usage().returning(|_| Ok(1_000_000));

        let mut amdsmi = build_amdsmi(hw, processors, Duration::from_secs(1));
        amdsmi.measure().await.unwrap();

        let result = amdsmi.retrieve().await.unwrap();
        assert_eq!(result.len(), 3);
        for index in [0, 1, 2] {
            assert!(result.contains_key(&index));
        }
    }

    #[tokio::test]
    async fn worker_polls_counters_and_can_be_cancelled() {
        let processors = processors_map(vec![make_processor(
            "UUID_0",
            ProcessorSupport::Power | ProcessorSupport::Vram | ProcessorSupport::Utilization,
        )]);

        let mut hw = MockAmdSmiHardware::default();
        hw.expect_get_power()
            .returning(|_| Ok(power_measurement(5_000)));
        hw.expect_get_vram_usage().returning(|_| Ok(1_000_000));
        hw.expect_get_gpu_activity()
            .returning(|_| Ok(gpu_usage(42)));

        let mut amdsmi = build_amdsmi(hw, processors, Duration::from_millis(20));
        amdsmi.init(0).await.unwrap();

        sleep(Duration::from_millis(80)).await;

        amdsmi.join().await.unwrap();

        assert!(amdsmi.power_counters.lock().unwrap().contains_key(&0));
    }
}
