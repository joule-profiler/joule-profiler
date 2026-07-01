//! NVML (NVIDIA Management Library) energy profiling integration.
//!
//! This module provides energy consumption monitoring for NVIDIA GPUs using the NVML library.

use std::collections::HashMap;

use joule_profiler_core::source::MetricSource;
use joule_profiler_core::source::Processor;
use joule_profiler_core::source::Sensor;
use joule_profiler_core::time::get_timestamp_micros;
use joule_profiler_core::types::AvailableSensor;
use joule_profiler_core::types::Sensors;
use joule_profiler_core::types::{Metric, Metrics};
use joule_profiler_core::unit::MetricUnit;
use joule_profiler_core::unit::Unit;
use joule_profiler_core::unit::UnitPrefix;
use nvml_wrapper::Nvml;
use thiserror::Error;
use tokio::task::JoinError;

#[derive(Debug, Error)]
pub enum NvmlError {
    /// NVML could not find or load the NVIDIA driver.
    #[error(
        "No driver found or loaded to access NVML, check whether you have an Nvidia GPU or not"
    )]
    NoDriverLoaded,

    /// The process lacks the required permissions to access NVML.
    #[error("Insufficient permissions to access NVML. Try running with sudo")]
    NoPermission,

    /// Error propagated from the underlying `nvml_wrapper` library.
    #[error("NVML error: {0}")]
    Nvml(
        #[from]
        #[source]
        nvml_wrapper::error::NvmlError,
    ),

    #[error("failed to join tokio task")]
    JoinError(
        #[from]
        #[source]
        JoinError,
    ),
}

const MILLI_JOULE_UNIT: MetricUnit = MetricUnit {
    prefix: UnitPrefix::Milli,
    unit: Unit::Joule,
};

fn init_nvml() -> Result<(Nvml, u32), NvmlError> {
    let nvml = Nvml::init().map_err(|err| match err {
        nvml_wrapper::error::NvmlError::DriverNotLoaded => NvmlError::NoDriverLoaded,
        nvml_wrapper::error::NvmlError::NoPermission => NvmlError::NoPermission,
        err => err.into(),
    })?;
    let device_count = nvml.device_count()?;
    Ok((nvml, device_count))
}

pub struct Snapshot {
    pub timestamp: u128,
    pub energy: HashMap<u32, u64>,
}

pub struct NvmlSensor {
    nvml: Nvml,
    device_count: u32,
}

impl Sensor<NvmlSource> for NvmlSensor {
    async fn measure(&mut self) -> Result<Snapshot, NvmlError> {
        let timestamp = get_timestamp_micros();
        let mut energy = HashMap::with_capacity(self.device_count as usize);
        for i in 0..self.device_count {
            let device = self.nvml.device_by_index(i)?;
            energy.insert(i, device.total_energy_consumption()?);
        }
        Ok(Snapshot { timestamp, energy })
    }

    fn list_sensors(&self) -> Result<Sensors, NvmlError> {
        (0..self.device_count)
            .map(|i| {
                Ok(AvailableSensor::new(
                    format!("GPU-{i}"),
                    MILLI_JOULE_UNIT,
                    NvmlSource::get_name(),
                ))
            })
            .collect()
    }
}

#[derive(Default)]
pub struct NvmlProcessor {
    baseline: Option<Snapshot>,
}

impl Processor<NvmlSource> for NvmlProcessor {
    async fn consume(&mut self, snapshot: Snapshot) -> Result<Option<Metrics>, NvmlError> {
        let metrics = self.baseline.as_ref().map(|baseline| {
            snapshot
                .energy
                .iter()
                .filter_map(|(device_index, &value)| {
                    let previous = baseline.energy.get(device_index)?;
                    let delta = value.wrapping_sub(*previous);
                    Some(Metric::new(
                        format!("GPU-{device_index}"),
                        delta,
                        NvmlSource::get_name(),
                        baseline.timestamp,
                    ))
                })
                .collect()
        });

        self.baseline = Some(snapshot);
        Ok(metrics)
    }
}

pub struct NvmlSource;

impl MetricSource for NvmlSource {
    type Sensor = NvmlSensor;
    type Processor = NvmlProcessor;
    type Snapshot = Snapshot;
    type Error = NvmlError;
    type Config = ();

    fn from_config(_config: ()) -> Result<(NvmlSensor, NvmlProcessor), NvmlError> {
        let (nvml, device_count) = init_nvml()?;
        Ok((NvmlSensor { nvml, device_count }, NvmlProcessor::default()))
    }

    fn get_name() -> &'static str {
        "NVML"
    }
}
