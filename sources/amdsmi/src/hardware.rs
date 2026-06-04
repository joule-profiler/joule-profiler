use std::collections::{HashMap, HashSet};

use amdsmi::types::EnergyCount;
use joule_profiler_core::time::get_timestamp_micros;
use log::{debug, trace};

use crate::{
    Processor, ProcessorSupport, Result, UUID, counters::PowerMeasurement, error::AmdSmiError,
};

pub trait Hardware: Send + Sync + 'static {
    fn get_processors(&self, spec: Option<&HashSet<UUID>>) -> Result<Vec<Processor>>;
    fn get_energy_count(&self, processor: &Processor) -> Result<EnergyCount>;
    fn get_power(&self, processor: &Processor) -> Result<PowerMeasurement>;
    fn get_vram_usage(&self, processor: &Processor) -> Result<u64>;
}

pub struct AmdSmi {
    processor_handles: HashMap<UUID, amdsmi::Processor>,
}

impl AmdSmi {
    pub fn new() -> Result<Self> {
        let amdsmi = amdsmi::AmdSmi::init()?;
        let (major, minor, patch) = amdsmi.get_lib_version()?;

        debug!("AMD SMI driver detected, version v{major}.{minor}.{patch}");

        let sockets = amdsmi.get_socket_handles()?;

        let processor_handles: HashMap<_, _> = sockets
            .into_iter()
            .flat_map(|s| {
                trace!("Socket {} detected.", s.get_socket_info()?);
                s.get_processor_handles()
            })
            .flatten()
            .flat_map(|p| {
                let board_info = p.get_board_info()?;
                let uuid = p.get_uuid()?;
                trace!("Discovered GPU device {board_info}, UUID: {uuid}.");
                Ok::<(UUID, amdsmi::Processor), AmdSmiError>((uuid, p))
            })
            .collect();

        debug!("Discovered {} gpus.", processor_handles.len());

        Ok(Self { processor_handles })
    }

    fn get_device_handle(&self, processor: &Processor) -> Result<&amdsmi::Processor> {
        self.processor_handles
            .get(&processor.uuid)
            .ok_or(AmdSmiError::NoSuchDevice(processor.clone()))
    }
}

impl Hardware for AmdSmi {
    fn get_processors(&self, spec: Option<&HashSet<UUID>>) -> Result<Vec<Processor>> {
        Ok(self
            .processor_handles
            .iter()
            .filter_map(|(uuid, handle)| {
                if let Some(spec) = &spec
                    && !spec.contains(uuid)
                {
                    trace!("Ignoring device {uuid}.");
                    return None;
                }

                let mut support = ProcessorSupport::empty();

                if handle.get_energy_count().is_ok() {
                    support |= ProcessorSupport::Energy;
                } else if handle.get_power().is_ok() {
                    support |= ProcessorSupport::Power;
                }
                if handle.get_vram_usage().is_ok() {
                    support |= ProcessorSupport::Vram;
                }

                debug!("Device {uuid} compatibility: {support:?}");

                if support.is_empty() {
                    trace!("No support detected for device {uuid}, ignored.");
                    None
                } else {
                    Some(Processor {
                        uuid: uuid.clone(),
                        support,
                    })
                }
            })
            .collect())
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
}
