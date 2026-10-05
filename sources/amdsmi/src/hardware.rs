use std::collections::{HashMap, HashSet};

use log::{debug, trace};

use crate::error::{AmdSmiError, Result};
use crate::sensor::{Device, DeviceSupport};

/// The AMD SMI calls the sensor makes, behind a trait so that tests can mock them.
#[cfg_attr(test, mockall::automock)]
#[allow(clippy::ref_option_ref)]
pub trait AmdSmiHardware: Send + Sync + 'static {
    fn new() -> Result<Self>
    where
        Self: Sized;

    /// The GPUs to measure and what each supports.
    // Automock needs lifetime and clippy wants it erased.
    #[allow(clippy::needless_lifetimes)]
    fn init_devices<'a>(&mut self, spec: Option<&'a HashSet<String>>) -> Result<Vec<Device>>;

    fn get_name(&self, device: &Device) -> Result<String>;

    /// The energy counter, in microjoules.
    fn get_energy(&self, device: &Device) -> Result<u64>;

    /// The power, in milliwatts.
    fn get_power(&self, device: &Device) -> Result<u32>;

    /// The VRAM used, in bytes.
    fn get_vram_usage(&self, device: &Device) -> Result<u64>;

    /// The utilization, in percent.
    fn get_utilization(&self, device: &Device) -> Result<u32>;
}

pub struct AmdSmiWrapperHardware {
    amdsmi: amdsmi::AmdSmi,

    /// By UUID.
    handles: HashMap<String, amdsmi::Processor>,
}

impl AmdSmiWrapperHardware {
    fn handle(&self, device: &Device) -> Result<&amdsmi::Processor> {
        self.handles
            .get(&device.uuid)
            .ok_or_else(|| AmdSmiError::NoSuchDevice(device.uuid.clone()))
    }
}

impl AmdSmiHardware for AmdSmiWrapperHardware {
    fn new() -> Result<Self> {
        debug!("Attempting to initialize AMD SMI reader");
        let amdsmi = amdsmi::AmdSmi::init().map_err(|err| match err {
            amdsmi::error::AmdSmiError::DriverNotLoaded => AmdSmiError::NoDriverLoaded,
            amdsmi::error::AmdSmiError::LibraryNotFound => AmdSmiError::LibraryNotFound,
            _ => err.into(),
        })?;

        let (major, minor, patch) = amdsmi.get_lib_version()?;
        debug!("AMD SMI driver detected, version v{major}.{minor}.{patch}");

        Ok(Self {
            amdsmi,
            handles: HashMap::new(),
        })
    }

    fn init_devices(&mut self, spec: Option<&HashSet<String>>) -> Result<Vec<Device>> {
        trace!("discovering AMD GPU devices");
        let mut devices = Vec::new();

        for socket in self.amdsmi.get_socket_handles()? {
            for processor in socket.get_processor_handles()? {
                let uuid = processor.get_uuid()?;
                trace!("discovered GPU device {uuid}");

                if spec.is_some_and(|spec| !spec.contains(&uuid)) {
                    trace!("ignoring device {uuid}");
                    continue;
                }

                let mut support = DeviceSupport::empty();

                if processor.get_energy_count().is_ok() {
                    support |= DeviceSupport::Energy;
                } else if processor.get_power().is_ok() {
                    support |= DeviceSupport::Power;
                }
                if processor.get_vram_usage().is_ok() {
                    support |= DeviceSupport::Vram;
                }
                if processor.get_gpu_activity().is_ok() {
                    support |= DeviceSupport::Utilization;
                }

                if support.is_empty() {
                    trace!("no support detected for device {uuid}, ignored");
                    continue;
                }

                trace!("GPU device {uuid} compatibility is {support:?}");
                self.handles.insert(uuid.clone(), processor);
                devices.push(Device { uuid, support });
            }
        }

        debug!("kept {} AMD GPU devices", devices.len());
        Ok(devices)
    }

    fn get_name(&self, device: &Device) -> Result<String> {
        Ok(self.handle(device)?.get_board_info()?)
    }

    #[allow(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "the accumulator times its resolution is a positive count of microjoules"
    )]
    fn get_energy(&self, device: &Device) -> Result<u64> {
        trace!("retrieving energy for device {}", device.uuid);
        let count = self.handle(device)?.get_energy_count()?;

        Ok((count.energy_accumulator as f64 * f64::from(count.counter_resolution)) as u64)
    }

    fn get_power(&self, device: &Device) -> Result<u32> {
        trace!("retrieving power for device {}", device.uuid);

        // Watts to milliwatts.
        Ok(self.handle(device)?.get_power()?.saturating_mul(1000))
    }

    fn get_vram_usage(&self, device: &Device) -> Result<u64> {
        trace!("retrieving VRAM usage for device {}", device.uuid);
        Ok(self.handle(device)?.get_vram_usage()?)
    }

    fn get_utilization(&self, device: &Device) -> Result<u32> {
        trace!("retrieving GPU utilization for device {}", device.uuid);
        Ok(self.handle(device)?.get_gpu_activity()?.gpu_usage)
    }
}
