use std::collections::{HashMap, HashSet};

use amdsmi::types::{EnergyCount, GpuUsageInfo};
use joule_profiler_core::time::get_timestamp_micros;
use log::{debug, trace};

use crate::{
    Processor, ProcessorSupport, Result, UUID, counters::PowerMeasurement, error::AmdSmiError,
};

/// Trait for abstracting the backend of AMD SMI library. Used for testing.
pub trait Hardware: Send + Sync + 'static {
    /// Init all GPU devices specicied by the provided specification.
    fn init_processors(&mut self, spec: Option<&HashSet<UUID>>) -> Result<Vec<Processor>>;

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
pub struct AmdSmi {
    /// Handle to the AMD SMI wrapper library.
    amdsmi: amdsmi::AmdSmi,

    /// The handles to the GPU devices.
    processor_handles: HashMap<UUID, amdsmi::Processor>,
}

impl AmdSmi {
    pub fn new() -> Result<Self> {
        let amdsmi = amdsmi::AmdSmi::init()?;
        let (major, minor, patch) = amdsmi.get_lib_version()?;

        debug!("AMD SMI driver detected, version v{major}.{minor}.{patch}");

        Ok(Self {
            amdsmi,
            processor_handles: HashMap::new(),
        })
    }

    fn get_device_handle(&self, processor: &Processor) -> Result<&amdsmi::Processor> {
        self.processor_handles
            .get(&processor.uuid)
            .ok_or(AmdSmiError::NoSuchDevice(processor.clone()))
    }
}

impl Hardware for AmdSmi {
    fn init_processors(&mut self, spec: Option<&HashSet<UUID>>) -> Result<Vec<Processor>> {
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
        Ok(self.get_device_handle(processor)?.get_energy_count()?)
    }

    fn get_power(&self, processor: &Processor) -> Result<PowerMeasurement> {
        Ok(PowerMeasurement {
            timestamp: get_timestamp_micros(),
            power: self.get_device_handle(processor)?.get_power()?,
        })
    }

    fn get_vram_usage(&self, processor: &Processor) -> Result<u64> {
        Ok(self.get_device_handle(processor)?.get_vram_usage()?)
    }

    fn get_gpu_activity(&self, processor: &Processor) -> Result<GpuUsageInfo> {
        Ok(self.get_device_handle(processor)?.get_gpu_activity()?)
    }
}
