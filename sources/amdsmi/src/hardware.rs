use std::collections::{HashMap, HashSet};

use amdsmi::types::{EnergyCount, GpuUsageInfo};
use joule_profiler_core::time::get_timestamp_micros;
use log::{debug, trace};

use crate::{
    Processor, ProcessorSupport, Result, UUID, counters::PowerMeasurement, error::AmdSmiError,
};

/// Trait for abstracting the backend of AMD SMI library. Used for testing.
#[cfg_attr(test, mockall::automock)]
#[allow(clippy::ref_option_ref)]
pub trait AmdSmiHardware: Send + Sync + 'static {
    /// Creates an hardware instance.
    fn new() -> Result<Self>
    where
        Self: Sized;

    /// Init all GPU devices specicied by the provided specification.
    // Automock needs lifetime and clippy wants it erased.
    #[allow(clippy::needless_lifetimes)]
    fn init_processors<'a>(&mut self, spec: Option<&'a HashSet<UUID>>) -> Result<Vec<Processor>>;

    /// Retrieve the energy count of a device.
    fn get_energy_count(&self, processor: &Processor) -> Result<EnergyCount>;

    /// Retrieve the instantaneous power of a device.
    fn get_power(&self, processor: &Processor) -> Result<PowerMeasurement>;

    /// Retrieve the current vram usage of a device.
    fn get_vram_usage(&self, processor: &Processor) -> Result<u64>;

    /// Retrieve the current GPU utilization info.
    fn get_gpu_activity(&self, processor: &Processor) -> Result<GpuUsageInfo>;
}

/// Backend for interacting with AMD SMI library.
pub struct AmdSmiWrapperHardware {
    /// Handle to the AMD SMI wrapper library.
    amdsmi: amdsmi::AmdSmi,

    /// The handles to the GPU devices.
    processor_handles: HashMap<UUID, amdsmi::Processor>,
}

impl AmdSmiWrapperHardware {
    fn get_device_handle(&self, processor: &Processor) -> Result<&amdsmi::Processor> {
        self.processor_handles
            .get(&processor.uuid)
            .ok_or(AmdSmiError::NoSuchDevice(processor.clone()))
    }
}

impl AmdSmiHardware for AmdSmiWrapperHardware {
    /// Creates a new AMD SMI hardware instance.
    ///
    /// This function will return an error if:
    /// - The AMD SMI library cannot be initialized (driver not installed, incompatible version, etc.)
    /// - The permissions are insufficient to be able to query the AMD SMI driver.
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
            processor_handles: HashMap::new(),
        })
    }

    /// Initializes devices with the specified devices specification.
    /// Check the compatibility of each device and determine which metrics can be queried.
    fn init_processors(&mut self, spec: Option<&HashSet<UUID>>) -> Result<Vec<Processor>> {
        trace!("Discovering AMD GPU devices.");
        let sockets = self.amdsmi.get_socket_handles()?;

        let processors: Vec<_> = sockets
            .into_iter()
            .flat_map(|s| {
                trace!("Socket {} detected.", s.get_socket_info()?);
                s.get_processor_handles()
            })
            .flatten()
            .flat_map(|p| {
                let uuid = p.get_uuid()?;
                trace!("Discovered GPU device {uuid}.");

                if let Some(spec) = &spec
                    && !spec.contains(&uuid)
                {
                    trace!("Ignoring device {uuid}.");
                    return Ok::<Option<Processor>, AmdSmiError>(None);
                }

                let mut support = ProcessorSupport::empty();

                if p.get_energy_count().is_ok() {
                    support |= ProcessorSupport::Energy;
                } else if p.get_power().is_ok() {
                    support |= ProcessorSupport::Power;
                }
                if p.get_vram_usage().is_ok() {
                    support |= ProcessorSupport::Vram;
                }
                if p.get_gpu_activity().is_ok() {
                    support |= ProcessorSupport::Utilization;
                }

                debug!("Device {uuid} compatibility: {support:?}");

                if support.is_empty() {
                    trace!("No support detected for device {uuid}, ignored.");
                    Ok(None)
                } else {
                    self.processor_handles.insert(uuid.clone(), p);
                    Ok(Some(Processor {
                        uuid: uuid.clone(),
                        support,
                    }))
                }
            })
            .flatten()
            .collect();

        debug!("Discovered {} gpus.", processors.len());

        Ok(processors)
    }

    fn get_energy_count(&self, processor: &Processor) -> Result<EnergyCount> {
        trace!("Retrieving energy for GPU device {}.", processor.uuid);
        Ok(self.get_device_handle(processor)?.get_energy_count()?)
    }

    fn get_power(&self, processor: &Processor) -> Result<PowerMeasurement> {
        trace!("Retrieving power for GPU device {}.", processor.uuid);
        Ok(PowerMeasurement {
            timestamp: get_timestamp_micros(),
            power: self.get_device_handle(processor)?.get_power()?,
        })
    }

    fn get_vram_usage(&self, processor: &Processor) -> Result<u64> {
        trace!("Retrieving VRAM usage for GPU device {}.", processor.uuid);
        Ok(self.get_device_handle(processor)?.get_vram_usage()?)
    }

    fn get_gpu_activity(&self, processor: &Processor) -> Result<GpuUsageInfo> {
        trace!(
            "Retrieving GPU utilization for GPU device {}.",
            processor.uuid
        );
        Ok(self.get_device_handle(processor)?.get_gpu_activity()?)
    }
}
