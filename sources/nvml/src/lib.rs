//! NVML (NVIDIA Management Library) energy profiling integration.
//!
//! This module provides energy consumption monitoring for NVIDIA GPUs using the NVML library.
//! It implements the `MetricReader` trait to collect energy metrics from GPU devices and
//! track energy usage over time.

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
    config::NvmlConfig,
    counters::{Counter, EnergyCounter, PowerCounter, UtilizationCounter, VramCounter},
    error::NvmlError,
    hardware::{NvmlHardware, NvmlWrapperHardware},
};

pub mod config;
pub mod counters;
mod error;
mod hardware;

#[allow(clippy::upper_case_acronyms)]
type UUID = String;

const NVML_SOURCE_NAME: &str = "NVML";

const MILLI_JOULE_UNIT: MetricUnit = MetricUnit {
    prefix: UnitPrefix::Milli,
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
    struct DeviceSupport: u8 {
        const Energy = 1;
        const Power = 1 << 1;
        const Vram = 1 << 2;
        const Utilization = 1 << 3;
    }
}

/// Custom result type for NVML.
type Result<T> = std::result::Result<T, NvmlError>;

/// Polling task handle and its cancellation token.
type WorkerHandle = (CancellationToken, JoinHandle<Result<()>>);

#[derive(Debug, Clone)]
pub struct Device {
    /// The index of the device.
    index: u32,

    /// The UUID of the device.
    uuid: UUID,

    /// The supports of the device (e.g., Energy, Power, VRAM).
    support: DeviceSupport,
}

/// NVML-based energy profiler for NVIDIA GPUs.
///
/// This struct provides an interface to monitor energy consumption of NVIDIA GPUs using
/// the NVML library.
/// The NVML hardware can be changed for testing purposes, but the default adapter is the NVML one.
#[allow(private_interfaces, private_bounds)]
pub struct Nvml<H: NvmlHardware = NvmlWrapperHardware> {
    /// Source configuration.
    config: NvmlConfig,

    /// The hardware instance for interacting with the NVIDIA driver.
    hardware: Arc<H>,

    /// Map of GPU devices.
    devices: Arc<Vec<Device>>,

    /// The handle to the polling task and its cancellation token.
    handle: Option<WorkerHandle>,

    /// The current energy counters.
    energy_counters: HashMap<u32, EnergyCounter>,

    /// The current vram counters.
    vram_counters: Arc<Mutex<HashMap<u32, VramCounter>>>,

    utilization_counters: Arc<Mutex<HashMap<u32, UtilizationCounter>>>,

    /// The current power counters.
    power_counters: Arc<Mutex<HashMap<u32, PowerCounter>>>,
}

impl Nvml {
    /// Creates a new NVML source instance.
    /// This initializes the NVML hardware with the specified devices specification.
    ///
    /// This function will return an error if:
    /// - The NVML library cannot be initialized (driver not installed, incompatible version, etc.)
    /// - Device information cannot be queried.
    /// - No GPU devices are detected.
    /// - The permissions are insufficient to be able to query the NVML driver.
    pub fn new(config: NvmlConfig) -> Result<Self> {
        let mut hardware = NvmlWrapperHardware::new()?;
        let devices = hardware.init_devices(config.gpus_spec.as_ref())?;

        Ok(Self {
            config,
            hardware: Arc::new(hardware),
            devices: Arc::new(devices),
            handle: None,
            energy_counters: HashMap::default(),
            power_counters: Arc::default(),
            utilization_counters: Arc::default(),
            vram_counters: Arc::default(),
        })
    }
}

impl<H: NvmlHardware> Nvml<H> {
    /// Creates the worker task for power and vram polling at the specified polling interval.
    pub fn create_worker(
        hardware: Arc<H>,
        devices: Arc<Vec<Device>>,
        power_counters: Arc<Mutex<HashMap<u32, PowerCounter>>>,
        vram_counters: Arc<Mutex<HashMap<u32, VramCounter>>>,
        utilization_counters: Arc<Mutex<HashMap<u32, UtilizationCounter>>>,
        poll_interval: Duration,
    ) -> Result<WorkerHandle> {
        let mut ticker = Interval::new_interval(poll_interval)?;

        let cancellation_token = CancellationToken::new();
        let cancellation_token_clone = cancellation_token.clone();

        let handle = tokio::spawn(async move {
            debug!("Starting NVML source polling.");

            loop {
                tokio::select! {
                    _ = ticker.next() => {
                        trace!("Polled NVML source.");
                        Self::read_polled_counters(&hardware, &devices, &power_counters, &vram_counters, &utilization_counters).await?;
                    }

                    () = cancellation_token.cancelled() => {
                        debug!("NVML worker stopped.");
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
        processors: &Arc<Vec<Device>>,
        power_counters: &Arc<Mutex<HashMap<u32, PowerCounter>>>,
        vram_counters: &Arc<Mutex<HashMap<u32, VramCounter>>>,
        utilization_counters: &Arc<Mutex<HashMap<u32, UtilizationCounter>>>,
    ) -> Result<()> {
        let mut vram_updates = Vec::new();
        let mut utilization_updates = Vec::new();
        let mut power_updates = Vec::new();

        for device in processors.iter() {
            if device.support.contains(DeviceSupport::Vram) {
                vram_updates.push((device.index, hardware.get_vram_usage(device)?));
            }

            if device.support.contains(DeviceSupport::Utilization) {
                utilization_updates.push((device.index, hardware.get_utilization(device)?));
            }

            if device.support.contains(DeviceSupport::Power) {
                power_updates.push((device.index, hardware.get_power(device)?));
            }
        }

        {
            let mut lock = vram_counters.lock().await;
            for (index, vram_usage) in vram_updates {
                lock.entry(index).or_default().update(vram_usage);
            }
        }

        {
            let mut lock = utilization_counters.lock().await;
            for (index, utilization) in utilization_updates {
                lock.entry(index).or_default().update(utilization);
            }
        }

        {
            let mut lock = power_counters.lock().await;
            for (index, power) in power_updates {
                lock.entry(index).or_default().push(power);
            }
        }

        Ok(())
    }
}

impl<H: NvmlHardware> MetricReader for Nvml<H> {
    type Type = HashMap<u32, Counter>;

    type Error = NvmlError;

    async fn measure(&mut self) -> Result<()> {
        debug!("NVML measure triggered.");
        for device in self.devices.iter() {
            if device.support.contains(DeviceSupport::Energy) {
                let energy = self.hardware.get_energy(device)?;
                self.energy_counters
                    .entry(device.index)
                    .or_default()
                    .update(energy);
            }
        }
        Self::read_polled_counters(
            &self.hardware,
            &self.devices,
            &self.power_counters,
            &self.vram_counters,
            &self.utilization_counters,
        )
        .await?;
        Ok(())
    }

    /// Retrieve the current counters and reset them for the next phase.
    async fn retrieve(&mut self) -> Result<Self::Type> {
        debug!("Retrieving NVML counters.");
        let mut energy_counters = self.energy_counters.clone();
        for counter in self.energy_counters.values_mut() {
            counter.reset();
        }

        let mut lock = self.vram_counters.lock().await;
        let mut vram_counters = lock.clone();
        for counter in lock.values_mut() {
            counter.reset();
        }

        let mut lock = self.utilization_counters.lock().await;
        let mut utilization_counters = lock.clone();
        for counter in lock.values_mut() {
            counter.reset();
        }

        let mut lock = self.power_counters.lock().await;
        let mut power_counters = lock.clone();
        for counter in lock.values_mut() {
            counter.reset();
        }

        let map = self
            .devices
            .iter()
            .map(|device| {
                let energy = energy_counters.remove(&device.index);
                let vram = vram_counters.remove(&device.index);
                let utilization = utilization_counters.remove(&device.index);
                let power = power_counters.remove(&device.index);
                let counter = Counter {
                    energy,
                    vram,
                    utilization,
                    power,
                };
                (device.index, counter)
            })
            .collect();

        Ok(map)
    }

    fn get_sensors(&self) -> Result<Sensors> {
        self.devices
            .iter()
            .map(|device| {
                Ok(Sensor::new(
                    device.uuid.clone(),
                    MILLI_JOULE_UNIT,
                    NVML_SOURCE_NAME,
                ))
            })
            .collect::<Result<_>>()
    }

    fn to_metrics(&self, result: Self::Type) -> Result<Metrics> {
        let metrics = result
            .into_iter()
            .flat_map(|(index, counter)| {
                let mut processor_metrics = Vec::new();

                let energy = counter
                    .energy
                    .map_or_else(|| counter.power.map(|c| c.compute_energy()), |c| c.diff());

                if let Some(energy) = energy {
                    processor_metrics.push(Metric::new(
                        format!("GPU-{index}-energy"),
                        energy,
                        MILLI_JOULE_UNIT,
                        Self::get_name(),
                    ));
                }

                if let Some(vram) = counter.vram
                    && let Some(min) = vram.min
                    && let Some(max) = vram.max
                {
                    processor_metrics.push(Metric::new(
                        format!("GPU-{index}-vram_min"),
                        min,
                        BYTE_UNIT,
                        Self::get_name(),
                    ));

                    processor_metrics.push(Metric::new(
                        format!("GPU-{index}-vram_max"),
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
                        format!("GPU-{index}-utilization_min"),
                        u64::from(min),
                        PERCENT_UNIT,
                        Self::get_name(),
                    ));

                    processor_metrics.push(Metric::new(
                        format!("GPU-{index}-utilization_max"),
                        u64::from(max),
                        PERCENT_UNIT,
                        Self::get_name(),
                    ));
                }

                Ok::<Metrics, NvmlError>(processor_metrics)
            })
            .flatten()
            .collect();
        Ok(metrics)
    }

    /// Creates the polling task if a polling interval has been configured.
    async fn init(&mut self, _pid: i32) -> Result<()> {
        if let Some(poll_interval) = self.config.poll_interval {
            self.handle = Some(Self::create_worker(
                self.hardware.clone(),
                self.devices.clone(),
                self.power_counters.clone(),
                self.vram_counters.clone(),
                self.utilization_counters.clone(),
                poll_interval,
            )?);
        }
        debug!("NVML source initialized.");
        Ok(())
    }

    /// Joins the polling task if it exists.
    async fn join(&mut self) -> Result<()> {
        if let Some((cancellation_token, handle)) = self.handle.take() {
            debug!("Joining NVML source polling task.");
            cancellation_token.cancel();
            handle.await??;
        }
        Ok(())
    }

    fn get_name() -> &'static str {
        NVML_SOURCE_NAME
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, sync::Arc, time::Duration};

    use joule_profiler_core::source::MetricReader;
    use mockall::predicate;
    use tokio::time::sleep;

    use crate::{
        Device, DeviceSupport, Nvml,
        config::NvmlConfig,
        counters::{Counter, PowerMeasurement},
        error::NvmlError,
        hardware::MockNvmlHardware,
    };

    fn make_device(index: u32, support: DeviceSupport) -> Device {
        Device {
            index,
            uuid: format!("GPU-mock-{index}"),
            support,
        }
    }

    fn build_nvml(
        hardware: MockNvmlHardware,
        devices: Vec<Device>,
        poll_interval: Option<Duration>,
    ) -> Nvml<MockNvmlHardware> {
        Nvml {
            config: NvmlConfig {
                poll_interval,
                gpus_spec: None,
            },
            hardware: Arc::new(hardware),
            devices: Arc::new(devices),
            handle: None,
            energy_counters: HashMap::default(),
            power_counters: Arc::default(),
            utilization_counters: Arc::default(),
            vram_counters: Arc::default(),
        }
    }

    fn power_measurement(power: u32) -> PowerMeasurement {
        PowerMeasurement {
            timestamp: 0,
            power,
        }
    }

    #[tokio::test]
    async fn measure_reads_energy_for_energy_capable_device() {
        let device = make_device(0, DeviceSupport::Energy);

        let mut hw = MockNvmlHardware::new();
        hw.expect_get_energy()
            .with(predicate::function(move |d: &Device| {
                d.index == device.index
            }))
            .once()
            .returning(|_| Ok(42_000));

        let mut nvml = build_nvml(hw, vec![device], None);
        nvml.measure().await.unwrap();

        assert!(nvml.energy_counters.contains_key(&0));
    }

    #[tokio::test]
    async fn measure_reads_power_for_power_only_device() {
        let device = make_device(0, DeviceSupport::Power);

        let mut hw = MockNvmlHardware::new();
        hw.expect_get_power()
            .with(predicate::function(move |d: &Device| {
                d.index == device.index
            }))
            .once()
            .returning(|_| Ok(power_measurement(150_000)));

        let mut nvml = build_nvml(hw, vec![device], None);
        nvml.measure().await.unwrap();

        assert!(nvml.power_counters.lock().await.contains_key(&0));
    }

    #[tokio::test]
    async fn measure_reads_vram_for_vram_capable_device() {
        let device = make_device(0, DeviceSupport::Vram);

        let mut hw = MockNvmlHardware::new();
        hw.expect_get_vram_usage()
            .with(predicate::function(move |d: &Device| {
                d.index == device.index
            }))
            .once()
            .returning(|_| Ok(8_000_000_000));

        let mut nvml = build_nvml(hw, vec![device], None);
        nvml.measure().await.unwrap();

        assert!(nvml.vram_counters.lock().await.contains_key(&0));
    }

    #[tokio::test]
    async fn measure_reads_utilization_for_utilization_capable_device() {
        let device = make_device(0, DeviceSupport::Utilization);

        let mut hw = MockNvmlHardware::new();
        hw.expect_get_utilization()
            .with(predicate::function(|d: &Device| d.index == 0))
            .once()
            .returning(|_| Ok(42));

        let mut nvml = build_nvml(hw, vec![device], None);
        nvml.measure().await.unwrap();

        assert!(nvml.utilization_counters.lock().await.contains_key(&0));
    }

    #[tokio::test]
    async fn measure_skips_energy_and_power_for_vram_only_device() {
        let device = make_device(0, DeviceSupport::Vram);

        let mut hw = MockNvmlHardware::new();
        hw.expect_get_energy().never();
        hw.expect_get_power().never();
        hw.expect_get_vram_usage()
            .once()
            .returning(|_| Ok(1_000_000));

        let mut nvml = build_nvml(hw, vec![device], None);
        nvml.measure().await.unwrap();

        assert!(nvml.energy_counters.is_empty());
        assert!(nvml.power_counters.lock().await.is_empty());
    }

    #[tokio::test]
    async fn measure_propagates_energy_error() {
        let device = make_device(0, DeviceSupport::Energy);

        let mut hw = MockNvmlHardware::new();
        hw.expect_get_energy()
            .once()
            .returning(|_| Err(NvmlError::NoPermission));

        let mut nvml = build_nvml(hw, vec![device], None);
        assert!(nvml.measure().await.is_err());
    }

    #[tokio::test]
    async fn measure_propagates_power_error() {
        let device = make_device(0, DeviceSupport::Power);

        let mut hw = MockNvmlHardware::new();
        hw.expect_get_power()
            .once()
            .returning(|_| Err(NvmlError::NoPermission));

        let mut nvml = build_nvml(hw, vec![device], None);
        assert!(nvml.measure().await.is_err());
    }

    #[tokio::test]
    async fn retrieve_returns_counters_and_resets_them() {
        let device = make_device(0, DeviceSupport::Energy | DeviceSupport::Vram);

        let mut hw = MockNvmlHardware::new();
        hw.expect_get_energy().returning(|_| Ok(10_000));
        hw.expect_get_vram_usage().returning(|_| Ok(1_000_000));

        let mut nvml = build_nvml(hw, vec![device], None);
        nvml.measure().await.unwrap();
        nvml.measure().await.unwrap();

        let result = nvml.retrieve().await.unwrap();
        assert!(result.contains_key(&0));

        let result2 = nvml.retrieve().await.unwrap();
        let counter: &Counter = result2.get(&0).unwrap();
        assert!(
            counter
                .energy
                .as_ref()
                .and_then(|e| e.diff())
                .is_none_or(|v| v == 0)
        );
    }

    #[tokio::test]
    async fn retrieve_includes_entry_for_every_device() {
        let devices = vec![
            make_device(0, DeviceSupport::Energy),
            make_device(1, DeviceSupport::Power),
            make_device(2, DeviceSupport::Vram),
        ];

        let mut hw = MockNvmlHardware::new();
        hw.expect_get_energy().returning(|_| Ok(1_000));
        hw.expect_get_power()
            .returning(|_| Ok(power_measurement(5_000)));
        hw.expect_get_vram_usage().returning(|_| Ok(1_000_000));

        let mut nvml = build_nvml(hw, devices, None);
        nvml.measure().await.unwrap();

        let result = nvml.retrieve().await.unwrap();
        assert_eq!(result.len(), 3);
        for index in [0, 1, 2] {
            assert!(result.contains_key(&index));
        }
    }

    #[tokio::test]
    async fn init_does_not_spawn_worker_without_poll_interval() {
        let mut nvml = build_nvml(MockNvmlHardware::new(), vec![], None);
        nvml.init(1234).await.unwrap();
        assert!(nvml.handle.is_none());
    }

    #[tokio::test]
    async fn init_spawns_worker_with_poll_interval() {
        let mut nvml = build_nvml(
            MockNvmlHardware::new(),
            vec![],
            Some(Duration::from_millis(50)),
        );
        nvml.init(1234).await.unwrap();
        assert!(nvml.handle.is_some());
        nvml.join().await.unwrap();
    }

    #[tokio::test]
    async fn worker_polls_counters_and_can_be_cancelled() {
        let device = make_device(
            0,
            DeviceSupport::Power | DeviceSupport::Vram | DeviceSupport::Utilization,
        );

        let mut hw = MockNvmlHardware::new();
        hw.expect_get_power()
            .returning(|_| Ok(power_measurement(5_000)));
        hw.expect_get_vram_usage().returning(|_| Ok(1_000_000));
        hw.expect_get_utilization().returning(|_| Ok(50));

        let mut nvml = build_nvml(hw, vec![device], Some(Duration::from_millis(20)));
        nvml.init(0).await.unwrap();

        sleep(Duration::from_millis(80)).await;

        nvml.join().await.unwrap();

        assert!(nvml.power_counters.lock().await.contains_key(&0));
    }
}
