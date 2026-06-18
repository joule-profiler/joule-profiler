use std::collections::HashSet;

use joule_profiler_core::time::get_timestamp_micros;
use log::{debug, trace};

use crate::{Device, DeviceSupport, Result, UUID, counters::PowerMeasurement, error::NvmlError};

/// Trait abstracting NVML hardware access for testability.
#[cfg_attr(test, mockall::automock)]
pub trait NvmlHardware: Send + Sync + 'static {
    // Automock needs lifetime and clippy wants it erased.
    #[allow(clippy::needless_lifetimes)]
    fn init_devices<'a>(&mut self, spec: Option<&'a HashSet<UUID>>) -> Result<Vec<Device>>;
    fn get_energy(&self, device: &Device) -> Result<u64>;
    fn get_power(&self, device: &Device) -> Result<PowerMeasurement>;
    fn get_vram_usage(&self, device: &Device) -> Result<u64>;
    fn get_utilization(&self, device: &Device) -> Result<u32>;
}

/// Hardware adapter for NVML library.
pub struct NvmlWrapperHardware {
    /// The NVML wrapper instance for interacting with the NVIDIA driver.
    pub nvml: nvml_wrapper::Nvml,
}

impl NvmlWrapperHardware {
    pub fn new() -> Result<Self> {
        debug!("Attempting to initialize NVML reader");
        let nvml = nvml_wrapper::Nvml::init().map_err(|err| match err {
            nvml_wrapper::error::NvmlError::DriverNotLoaded => NvmlError::NoDriverLoaded,
            nvml_wrapper::error::NvmlError::NoPermission => NvmlError::NoPermission,
            _ => err.into(),
        })?;

        Ok(Self { nvml })
    }
}

impl NvmlHardware for NvmlWrapperHardware {
    fn init_devices(&mut self, spec: Option<&HashSet<UUID>>) -> Result<Vec<Device>> {
        trace!("Discovering NVIDIA GPU devices.");
        let device_count = self.nvml.device_count()?;

        let devices: Vec<_> = (0..device_count)
            .flat_map(|i| {
                let device = self.nvml.device_by_index(i)?;
                let uuid = device.uuid()?;
                trace!("Discovered GPU device {uuid}.");

                if let Some(spec) = &spec
                    && !spec.contains(&uuid)
                {
                    trace!("Ignoring device {uuid}.");
                    return Ok::<Option<Device>, NvmlError>(None);
                }

                let mut support = DeviceSupport::empty();

                if device.total_energy_consumption().is_ok() {
                    support |= DeviceSupport::Energy;
                } else if device.power_usage().is_ok() {
                    support |= DeviceSupport::Power;
                }
                if device.memory_info().is_ok() {
                    support |= DeviceSupport::Vram;
                }
                if device.utilization_rates().is_ok() {
                    support |= DeviceSupport::Utilization;
                }

                debug!("Device {uuid}, compatibility: {support:?}");

                if support.is_empty() {
                    trace!("No support detected for device {uuid}, ignored.");
                    Ok::<Option<Device>, NvmlError>(None)
                } else {
                    Ok(Some(Device {
                        index: i,
                        uuid: uuid.clone(),
                        support,
                    }))
                }
            })
            .flatten()
            .collect();

        Ok(devices)
    }

    fn get_energy(&self, device: &Device) -> Result<u64> {
        trace!("Retrieving energy for NVIDIA GPU device {}.", device.index);
        Ok(self
            .nvml
            .device_by_index(device.index)?
            .total_energy_consumption()?)
    }

    fn get_power(&self, device: &Device) -> Result<PowerMeasurement> {
        trace!("Retrieving power for NVIDIA  GPU device {}.", device.index);
        Ok(self
            .nvml
            .device_by_index(device.index)?
            .power_usage()
            .map(|power| PowerMeasurement {
                timestamp: get_timestamp_micros(),
                power,
            })?)
    }

    fn get_vram_usage(&self, device: &Device) -> Result<u64> {
        trace!(
            "Retrieving VRAM usage NVIDIA for GPU device {}.",
            device.index
        );
        Ok(self.nvml.device_by_index(device.index)?.memory_info()?.used)
    }

    fn get_utilization(&self, device: &Device) -> Result<u32> {
        trace!(
            "Retrieving GPU utilization for NVIDIA GPU device {}.",
            device.index
        );
        Ok(self
            .nvml
            .device_by_index(device.index)?
            .utilization_rates()?
            .gpu)
    }
}
