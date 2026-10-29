use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use joule_profiler_core::info::{Info, InfoValue};
use joule_profiler_core::source::Source;
use serde::Deserialize;

pub mod error;
pub mod processor;
pub mod sensor;

mod hardware;

pub use error::{NvmlError, Result};
pub use processor::NvmlProcessor;
pub use sensor::{Device, DeviceSupport, NvmlSensor, Snapshot};

use crate::hardware::{NvmlHardware, NvmlWrapperHardware};
use crate::sensor::{Devices, polls};

const POLL: Duration = Duration::from_millis(20);

/// GPU energy, and in normal mode VRAM and utilization too. Minimal mode needs no polling
/// thread for a GPU with its own energy counter.
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Nvml {
    #[serde(rename = "gpus")]
    gpus_spec: Option<HashSet<u32>>,

    /// Energy only.
    minimal: bool,

    #[serde(with = "humantime_serde")]
    poll_interval: Duration,
}

impl Default for Nvml {
    fn default() -> Self {
        Self {
            gpus_spec: None,
            minimal: true,
            poll_interval: POLL,
        }
    }
}

impl Nvml {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn gpus(mut self, gpus: impl IntoIterator<Item = u32>) -> Self {
        self.gpus_spec = Some(gpus.into_iter().collect());
        self
    }

    pub fn minimal(mut self, minimal: bool) -> Self {
        self.minimal = minimal;
        self
    }

    /// Sets how often the power, VRAM and utilization of the GPUs are read. Defaults to 20ms.
    pub fn poll(mut self, interval: Duration) -> Self {
        self.poll_interval = interval;
        self
    }

    pub fn build(self) -> Result<Source> {
        let mut hardware = NvmlWrapperHardware::new()?;
        let devices = self.resolve(&mut hardware)?;

        let info = self.info(&hardware, &devices);

        Ok(Source::new(
            NvmlSensor::new(Arc::new(hardware), Arc::clone(&devices), self.poll_interval),
            NvmlProcessor::new(devices),
        )
        .with_info(info))
    }

    /// The GPUs to measure, each limited to what the mode asks for.
    fn resolve<H: NvmlHardware>(&self, hardware: &mut H) -> Result<Devices> {
        let mut devices = hardware.init_devices(self.gpus_spec.as_ref())?;

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

    /// The poll interval, then the model of each GPU, or why it cannot be read.
    fn info<H: NvmlHardware>(&self, hardware: &H, devices: &[Device]) -> Info {
        let polling = if polls(devices) {
            InfoValue::from(self.poll_interval)
        } else {
            InfoValue::from("none")
        };

        devices.iter().fold(
            Info::new().with("poll_interval", polling),
            |info, device| info.with(format!("GPU-{}", device.index), hardware.get_name(*device)),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hardware::MockNvmlHardware;

    fn found(support: Vec<DeviceSupport>) -> MockNvmlHardware {
        let mut hardware = MockNvmlHardware::default();

        hardware.expect_init_devices().returning(move |_| {
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
    fn minimal_keeps_both_ways_of_telling_the_energy() {
        let mut hardware = found(vec![
            DeviceSupport::Energy | DeviceSupport::Vram,
            DeviceSupport::Power | DeviceSupport::Utilization,
        ]);

        let devices = Nvml::new()
            .minimal(true)
            .resolve(&mut hardware)
            .expect("resolves");

        assert_eq!(devices[0].support, DeviceSupport::Energy);
        assert_eq!(devices[1].support, DeviceSupport::Power);
    }

    #[test]
    fn normal_keeps_the_vram_and_the_load_as_well() {
        let mut hardware = found(vec![
            DeviceSupport::Energy | DeviceSupport::Vram | DeviceSupport::Utilization,
        ]);

        let devices = Nvml::new()
            .minimal(false)
            .resolve(&mut hardware)
            .expect("resolves");

        assert_eq!(
            devices[0].support,
            DeviceSupport::Energy | DeviceSupport::Vram | DeviceSupport::Utilization
        );
    }

    #[test]
    fn minimal_leaves_out_a_gpu_that_reports_no_energy_at_all() {
        let mut hardware = found(vec![DeviceSupport::Vram | DeviceSupport::Utilization]);

        assert!(matches!(
            Nvml::new().minimal(true).resolve(&mut hardware),
            Err(NvmlError::NoDeviceDetected)
        ));
    }

    #[test]
    fn a_model_that_cannot_be_read_says_why() {
        let mut hardware = found(vec![DeviceSupport::Energy, DeviceSupport::Energy]);
        let mut first = true;
        hardware.expect_get_name().returning(move |_| {
            if std::mem::take(&mut first) {
                Ok("Some GPU".to_owned())
            } else {
                Err(NvmlError::NoDeviceDetected)
            }
        });
        let devices = hardware.init_devices(None).unwrap();

        let info = Nvml::new().info(&hardware, &devices);

        let models: Vec<&InfoValue> = info
            .entries()
            .iter()
            .skip(1)
            .map(|(_, model)| model)
            .collect();
        assert_eq!(
            models,
            [
                &InfoValue::from("Some GPU"),
                &InfoValue::from("unavailable (no GPU device detected)")
            ]
        );
    }

    #[test]
    fn the_poll_interval_is_none_when_no_gpu_is_polled() {
        let mut hardware = found(vec![DeviceSupport::Energy]);
        hardware
            .expect_get_name()
            .returning(|_| Ok("Some GPU".to_owned()));
        let devices = hardware.init_devices(None).unwrap();

        let info = Nvml::new()
            .poll(Duration::from_millis(10))
            .info(&hardware, &devices);

        assert_eq!(info.get("poll_interval"), Some(&"none".into()));
    }

    #[test]
    fn the_poll_interval_is_given_when_a_gpu_is_polled() {
        let mut hardware = found(vec![DeviceSupport::Power | DeviceSupport::Vram]);
        hardware
            .expect_get_name()
            .returning(|_| Ok("Some GPU".to_owned()));
        let devices = hardware.init_devices(None).unwrap();

        let info = Nvml::new()
            .poll(Duration::from_millis(10))
            .info(&hardware, &devices);

        assert_eq!(info.get("poll_interval"), Some(&"10ms".into()));
    }
}
