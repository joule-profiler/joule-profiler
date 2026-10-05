use std::collections::HashSet;

use log::{debug, trace};

use crate::sensor::{Device, DeviceSupport};
use crate::{Result, error::NvmlError};

/// The NVML calls the sensor makes, behind a trait so that tests can mock them.
#[cfg_attr(test, mockall::automock)]
#[allow(clippy::ref_option_ref)]
pub trait NvmlHardware: Send + Sync + 'static {
    fn new() -> Result<Self>
    where
        Self: Sized;

    /// The GPUs to measure and what each supports.
    // Automock needs lifetime and clippy wants it erased.
    #[allow(clippy::needless_lifetimes)]
    fn init_devices<'a>(&mut self, spec: Option<&'a HashSet<u32>>) -> Result<Vec<Device>>;

    fn get_name(&self, device: Device) -> Result<String>;

    /// The energy counter.
    fn get_energy(&self, device: Device) -> Result<u64>;

    /// The power, in milliwatts.
    fn get_power(&self, device: Device) -> Result<u32>;

    /// The VRAM used, in bytes.
    fn get_vram_usage(&self, device: Device) -> Result<u64>;

    /// The utilization, in percent.
    fn get_utilization(&self, device: Device) -> Result<u32>;
}

pub struct NvmlWrapperHardware {
    nvml: nvml_wrapper::Nvml,
}

impl NvmlHardware for NvmlWrapperHardware {
    fn new() -> Result<Self> {
        debug!("Attempting to initialize NVML reader");
        let nvml = nvml_wrapper::Nvml::init().map_err(|err| match err {
            nvml_wrapper::error::NvmlError::DriverNotLoaded => NvmlError::NoDriverLoaded,
            nvml_wrapper::error::NvmlError::NoPermission => NvmlError::NoPermission,
            _ => err.into(),
        })?;

        Ok(Self { nvml })
    }

    fn init_devices(&mut self, spec: Option<&HashSet<u32>>) -> Result<Vec<Device>> {
        trace!("discovering GPU devices");
        let device_count = self.nvml.device_count()?;

        let devices: Vec<_> = (0..device_count)
            .flat_map(|i| {
                let device = self.nvml.device_by_index(i)?;
                let uuid = device.uuid()?;
                trace!("discovered GPU device {i} with UUID {uuid}");

                if let Some(spec) = &spec
                    && !spec.contains(&i)
                {
                    trace!("ignoring device {i}");
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

                if support.is_empty() {
                    trace!("no support detected for device {uuid}, ignored");
                    Ok::<Option<Device>, NvmlError>(None)
                } else {
                    trace!("GPU device {i} compatibility is {support:?}");
                    Ok(Some(Device { index: i, support }))
                }
            })
            .flatten()
            .collect();

        debug!(
            "detected {device_count} GPU devices and kept {}",
            devices.len()
        );
        Ok(devices)
    }

    fn get_name(&self, device: Device) -> Result<String> {
        Ok(self.nvml.device_by_index(device.index)?.name()?)
    }

    fn get_energy(&self, device: Device) -> Result<u64> {
        trace!("retrieving energy for device {}", device.index);
        Ok(self
            .nvml
            .device_by_index(device.index)?
            .total_energy_consumption()?)
    }

    fn get_power(&self, device: Device) -> Result<u32> {
        trace!("retrieving power for device {}", device.index);
        Ok(self.nvml.device_by_index(device.index)?.power_usage()?)
    }

    fn get_vram_usage(&self, device: Device) -> Result<u64> {
        trace!("retrieving VRAM usage for device {}", device.index);
        Ok(self.nvml.device_by_index(device.index)?.memory_info()?.used)
    }

    fn get_utilization(&self, device: Device) -> Result<u32> {
        trace!("retrieving GPU utilization for device {}", device.index);
        Ok(self
            .nvml
            .device_by_index(device.index)?
            .utilization_rates()?
            .gpu)
    }
}
