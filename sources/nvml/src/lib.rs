//! `nvml` source for Joule Profiler.
//!
//! Reads the energy of NVIDIA GPUs through NVML, the library behind `nvidia-smi`, and optionally
//! their VRAM and utilization. A GPU without an energy counter reports the integral of its power
//! instead.
//!
//! # Example
//!
//! ```no_run
//! use joule_profiler_source_nvml::Nvml;
//!
//! let source = Nvml::new().gpus([0]).minimal(false).build().unwrap();
//! ```

mod error;
mod hardware;
mod processor;
mod sensor;

use std::{collections::HashSet, sync::Arc, time::Duration};

use joule_profiler_core::{Info, InfoValue, MemoryUnit, MetricUnit, Source};
use serde::Deserialize;

pub use error::NvmlError;

use crate::{
    error::Result,
    hardware::{NvmlHardware, NvmlWrapperHardware},
    processor::NvmlProcessor,
    sensor::{Device, DeviceSupport, Devices, NvmlSensor, needs_polling},
};

const DEFAULT_POLL_INTERVAL: Duration = Duration::from_millis(20);

/// Configuration of the nvml source.
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Nvml {
    gpus: Option<HashSet<u32>>,

    minimal: bool,

    #[serde(with = "humantime_serde")]
    poll_interval: Duration,

    memory_unit: MemoryUnit,
}

impl Default for Nvml {
    fn default() -> Self {
        Self {
            gpus: None,
            minimal: true,
            poll_interval: DEFAULT_POLL_INTERVAL,
            memory_unit: MemoryUnit::default(),
        }
    }
}

impl Nvml {
    /// Initializes a configuration that measures the energy of every GPU.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the GPUs to measure, by NVML index. Defaults to every GPU.
    pub fn gpus(mut self, gpus: impl IntoIterator<Item = u32>) -> Self {
        self.gpus = Some(gpus.into_iter().collect());
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
    /// Returns [`NvmlError::NoDriverLoaded`] or [`NvmlError::NoPermission`] if NVML cannot be
    /// initialized, and [`NvmlError::NoDeviceDetected`] if no GPU reports what is asked.
    pub fn build(self) -> Result<Source> {
        let mut hardware = NvmlWrapperHardware::new()?;
        let devices = self.filter_devices(&mut hardware)?;

        let info = self.info(&hardware, &devices);

        Ok(Source::new(
            NvmlSensor::new(Arc::new(hardware), Arc::clone(&devices), self.poll_interval),
            NvmlProcessor::new(devices, self.memory_unit),
        )
        .with_info(info))
    }

    /// Returns the GPUs to measure, each limited to what the mode asks for.
    fn filter_devices<H: NvmlHardware>(&self, hardware: &mut H) -> Result<Devices> {
        let mut devices = hardware.discover_devices(self.gpus.as_ref())?;

        if self.minimal {
            for device in &mut devices {
                device.support &= DeviceSupport::ENERGY;
            }
        }

        devices.retain(|device: &Device| !device.support.is_empty());

        if devices.is_empty() {
            return Err(NvmlError::NoDeviceDetected);
        }

        log::debug!(
            "nvml: {} GPUs measured, {} mode",
            devices.len(),
            if self.minimal { "minimal" } else { "normal" }
        );

        Ok(devices.into())
    }

    /// Returns the info of the source: the configuration, then the model of each GPU, or why it
    /// cannot be read.
    fn info<H: NvmlHardware>(&self, hardware: &H, devices: &[Device]) -> Info {
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
            |info, device| {
                info.with(
                    format!("GPU-{}", device.index),
                    hardware.read_model(*device),
                )
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hardware::MockNvmlHardware;

    fn mock_hardware(support: Vec<DeviceSupport>) -> MockNvmlHardware {
        let mut hardware = MockNvmlHardware::default();

        hardware.expect_discover_devices().returning(move |_| {
            Ok(support
                .iter()
                .enumerate()
                .map(|(index, support)| Device {
                    index: u32::try_from(index).unwrap(),
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

        let devices = Nvml::new()
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

        let devices = Nvml::new()
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
            Nvml::new().minimal(true).filter_devices(&mut hardware),
            Err(NvmlError::NoDeviceDetected)
        ));
    }

    #[test]
    fn unreadable_model() {
        let mut hardware = mock_hardware(vec![DeviceSupport::Energy, DeviceSupport::Energy]);
        hardware.expect_read_model().returning(|device| {
            if device.index == 0 {
                Ok("Some GPU".to_owned())
            } else {
                Err(NvmlError::NoDeviceDetected)
            }
        });
        let devices = hardware.discover_devices(None).unwrap();

        let info = Nvml::new().info(&hardware, &devices);

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

            let info = Nvml::new()
                .poll_interval(Duration::from_millis(10))
                .info(&hardware, &devices);

            assert_eq!(info.get("poll_interval"), Some(&poll_interval.into()));
        }
    }
}
