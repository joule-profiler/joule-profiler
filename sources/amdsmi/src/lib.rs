//! `amdsmi` source for Joule Profiler.
//!
//! Reads the energy of AMD GPUs through AMD SMI, the library behind `amd-smi`, and optionally
//! their VRAM and utilization. A GPU without an energy counter reports the integral of its power
//! instead.
//!
//! # Example
//!
//! ```no_run
//! use joule_profiler_source_amdsmi::AmdSmi;
//!
//! let source = AmdSmi::new().minimal(false).build().unwrap();
//! ```

mod error;
mod hardware;
mod processor;
mod sensor;

use std::{collections::HashSet, sync::Arc, time::Duration};

use joule_profiler_core::{Info, InfoValue, MemoryUnit, MetricUnit, Source};
use serde::Deserialize;

pub use error::AmdSmiError;

use crate::{
    error::Result,
    hardware::{AmdSmiHardware, AmdSmiWrapperHardware},
    processor::AmdSmiProcessor,
    sensor::{AmdSmiSensor, Device, DeviceSupport, Devices, needs_polling},
};

const DEFAULT_POLL_INTERVAL: Duration = Duration::from_millis(20);

/// Configuration of the amdsmi source.
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AmdSmi {
    gpus: Option<HashSet<String>>,

    minimal: bool,

    #[serde(with = "humantime_serde")]
    poll_interval: Duration,

    memory_unit: MemoryUnit,
}

impl Default for AmdSmi {
    fn default() -> Self {
        Self {
            gpus: None,
            minimal: true,
            poll_interval: DEFAULT_POLL_INTERVAL,
            memory_unit: MemoryUnit::default(),
        }
    }
}

impl AmdSmi {
    /// Initializes a configuration that measures the energy of every GPU.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the GPUs to measure, by UUID. Defaults to every GPU.
    pub fn gpus(mut self, gpus: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.gpus = Some(gpus.into_iter().map(Into::into).collect());
        self
    }

    /// Sets whether only the energy is reported, without the VRAM and the utilization. Defaults
    /// to `true`.
    pub fn minimal(mut self, minimal: bool) -> Self {
        self.minimal = minimal;
        self
    }

    /// Sets how often the power, VRAM and utilization of the GPUs are read. Defaults to 20ms.
    pub fn poll_interval(mut self, interval: Duration) -> Self {
        self.poll_interval = interval;
        self
    }

    /// Sets the unit of the VRAM. Defaults to bytes.
    pub fn memory_unit(mut self, unit: MemoryUnit) -> Self {
        self.memory_unit = unit;
        self
    }

    /// Builds the source.
    ///
    /// # Errors
    ///
    /// Returns [`AmdSmiError::NoDriverLoaded`] or [`AmdSmiError::LibraryNotFound`] if AMD SMI
    /// cannot be initialized, and [`AmdSmiError::NoDeviceDetected`] if no GPU reports what is
    /// asked.
    pub fn build(self) -> Result<Source> {
        let mut hardware = AmdSmiWrapperHardware::new()?;
        let devices = self.filter_devices(&mut hardware)?;

        let info = self.info(&hardware, &devices);

        Ok(Source::new(
            AmdSmiSensor::new(Arc::new(hardware), Arc::clone(&devices), self.poll_interval),
            AmdSmiProcessor::new(devices, self.memory_unit),
        )
        .with_info(info))
    }

    /// Returns the GPUs to measure, each limited to what the mode asks for.
    fn filter_devices<H: AmdSmiHardware>(&self, hardware: &mut H) -> Result<Devices> {
        let mut devices = hardware.discover_devices(self.gpus.as_ref())?;

        if self.minimal {
            for device in &mut devices {
                device.support &= DeviceSupport::ENERGY;
            }
        }

        devices.retain(|device: &Device| !device.support.is_empty());

        if devices.is_empty() {
            return Err(AmdSmiError::NoDeviceDetected);
        }

        log::debug!(
            "amdsmi: {} GPUs measured, {} mode",
            devices.len(),
            if self.minimal { "minimal" } else { "normal" }
        );

        Ok(devices.into())
    }

    /// Returns the info of the source: the configuration, then the model of each GPU, or why it
    /// cannot be read.
    fn info<H: AmdSmiHardware>(&self, hardware: &H, devices: &[Device]) -> Info {
        let poll_interval = if needs_polling(devices) {
            InfoValue::from(self.poll_interval)
        } else {
            InfoValue::from("none")
        };

        devices.iter().fold(
            Info::new().with("poll_interval", poll_interval).with(
                "memory_unit",
                MetricUnit::from(self.memory_unit).to_string(),
            ),
            |info, device| info.with(&device.uuid, hardware.read_model(device)),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hardware::MockAmdSmiHardware;

    fn mock_hardware(support: Vec<DeviceSupport>) -> MockAmdSmiHardware {
        let mut hardware = MockAmdSmiHardware::default();

        hardware.expect_discover_devices().returning(move |_| {
            Ok(support
                .iter()
                .enumerate()
                .map(|(index, support)| Device {
                    uuid: format!("GPU-{index}"),
                    support: *support,
                })
                .collect())
        });

        hardware
    }

    #[test]
    fn minimal_keeps_energy() {
        let mut hardware = mock_hardware(vec![
            DeviceSupport::Energy | DeviceSupport::Vram,
            DeviceSupport::Power | DeviceSupport::Utilization,
        ]);

        let devices = AmdSmi::new()
            .minimal(true)
            .filter_devices(&mut hardware)
            .unwrap();

        assert_eq!(devices[0].support, DeviceSupport::Energy);
        assert_eq!(devices[1].support, DeviceSupport::Power);
    }

    #[test]
    fn normal_keeps_everything() {
        let mut hardware = mock_hardware(vec![
            DeviceSupport::Energy | DeviceSupport::Vram | DeviceSupport::Utilization,
        ]);

        let devices = AmdSmi::new()
            .minimal(false)
            .filter_devices(&mut hardware)
            .unwrap();

        assert_eq!(
            devices[0].support,
            DeviceSupport::Energy | DeviceSupport::Vram | DeviceSupport::Utilization
        );
    }

    #[test]
    fn minimal_drops_gpu_without_energy() {
        let mut hardware = mock_hardware(vec![DeviceSupport::Vram | DeviceSupport::Utilization]);

        assert!(matches!(
            AmdSmi::new().minimal(true).filter_devices(&mut hardware),
            Err(AmdSmiError::NoDeviceDetected)
        ));
    }

    #[test]
    fn unreadable_model() {
        let mut hardware = mock_hardware(vec![DeviceSupport::Energy, DeviceSupport::Energy]);
        hardware.expect_read_model().returning(|device| {
            if device.uuid == "GPU-0" {
                Ok("Some GPU".to_owned())
            } else {
                Err(AmdSmiError::NoDeviceDetected)
            }
        });
        let devices = hardware.discover_devices(None).unwrap();

        let info = AmdSmi::new().info(&hardware, &devices);

        assert_eq!(info.get("GPU-0"), Some(&"Some GPU".into()));
        assert_eq!(
            info.get("GPU-1"),
            Some(&"unavailable (no GPU device detected)".into())
        );
    }

    #[test]
    fn poll_interval_only_when_polled() {
        for (support, poll_interval) in [
            (DeviceSupport::Energy, "none"),
            (DeviceSupport::Power | DeviceSupport::Vram, "10ms"),
        ] {
            let mut hardware = mock_hardware(vec![support]);
            hardware
                .expect_read_model()
                .returning(|_| Ok("Some GPU".to_owned()));
            let devices = hardware.discover_devices(None).unwrap();

            let info = AmdSmi::new()
                .poll_interval(Duration::from_millis(10))
                .info(&hardware, &devices);

            assert_eq!(info.get("poll_interval"), Some(&poll_interval.into()));
        }
    }
}
