use std::collections::HashSet;

use joule_profiler_core::time::get_timestamp_micros;
use log::{debug, trace};

use crate::{Device, DeviceSupport, Result, counters::PowerMeasurement, error::NvmlError};

/// Trait for abstracting the backend of NVML library. Used for testing.
#[cfg_attr(test, mockall::automock)]
#[allow(clippy::ref_option_ref)]
pub trait NvmlHardware: Send + Sync + 'static {
    /// Creates an hardware instance.
    fn new() -> Result<Self>
    where
        Self: Sized;

    /// Init all GPU devices specicied by the provided specification.
    // Automock needs lifetime and clippy wants it erased.
    #[allow(clippy::needless_lifetimes)]
    fn init_devices<'a>(&mut self, spec: Option<&'a HashSet<u32>>) -> Result<Vec<Device>>;

    /// Retrieve the energy count of a device.
    fn get_energy(&self, device: &Device) -> Result<u64>;

    /// Retrieve the instantaneous power of a device.
    fn get_power(&self, device: &Device) -> Result<PowerMeasurement>;

    /// Retrieve the current vram usage of a device.
    fn get_vram_usage(&self, device: &Device) -> Result<u64>;

    /// Retrieve the current GPU utilization info.
    fn get_utilization(&self, device: &Device) -> Result<u32>;
}

/// Hardware adapter for NVML library.
pub struct NvmlWrapperHardware {
    /// The NVML wrapper instance for interacting with the NVIDIA driver.
    pub nvml: nvml_wrapper::Nvml,
}

impl NvmlHardware for NvmlWrapperHardware {
    /// Creates a new NVML hardware instance.
    ///
    /// This function will return an error if:
    /// - The NVML library cannot be initialized (driver not installed, incompatible version, etc.)
    /// - The permissions are insufficient to be able to query the NVML driver.
    fn new() -> Result<Self> {
        debug!("Attempting to initialize NVML reader");
        let nvml = nvml_wrapper::Nvml::init().map_err(|err| match err {
            nvml_wrapper::error::NvmlError::DriverNotLoaded => NvmlError::NoDriverLoaded,
            nvml_wrapper::error::NvmlError::NoPermission => NvmlError::NoPermission,
            _ => err.into(),
        })?;

        Ok(Self { nvml })
    }

    /// Initializes devices with the specified devices specification.
    /// Check the compatibility of each device and determine which metrics can be queried.
    fn init_devices(&mut self, spec: Option<&HashSet<u32>>) -> Result<Vec<Device>> {
        trace!("Discovering GPU devices.");
        let device_count = self.nvml.device_count()?;

        let devices: Vec<_> = (0..device_count)
            .flat_map(|i| {
                let device = self.nvml.device_by_index(i)?;
                let uuid = device.uuid()?;
                trace!("Discovered GPU device, UUID: {uuid}, index: {i}.");

                if let Some(spec) = &spec
                    && !spec.contains(&i)
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
                    Ok(Some(Device { index: i, support }))
                }
            })
            .flatten()
            .collect();

        Ok(devices)
    }

    fn get_energy(&self, device: &Device) -> Result<u64> {
        trace!("Retrieving energy for GPU device {}.", device.index);
        Ok(self
            .nvml
            .device_by_index(device.index)?
            .total_energy_consumption()?)
    }

    fn get_power(&self, device: &Device) -> Result<PowerMeasurement> {
        trace!("Retrieving power for GPU device {}.", device.index);
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
        trace!("Retrieving VRAM usage for GPU device {}.", device.index);
        Ok(self.nvml.device_by_index(device.index)?.memory_info()?.used)
    }

    fn get_utilization(&self, device: &Device) -> Result<u32> {
        trace!(
            "Retrieving GPU utilization for GPU device {}.",
            device.index
        );
        Ok(self
            .nvml
            .device_by_index(device.index)?
            .utilization_rates()?
            .gpu)
    }
}
