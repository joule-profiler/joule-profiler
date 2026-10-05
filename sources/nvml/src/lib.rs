use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use joule_profiler_core::info::InfoValue;
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
use crate::sensor::Devices;

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

    pub fn minimal(mut self) -> Self {
        self.minimal = true;
        self
    }

    pub fn normal(mut self) -> Self {
        self.minimal = false;
        self
    }

    pub fn poll(mut self, interval: Duration) -> Self {
        self.poll_interval = interval;
        self
    }

    pub fn build(self) -> Result<Source> {
        let mut hardware = NvmlWrapperHardware::new()?;
        let devices = self.resolve(&mut hardware)?;

        let models = models(&hardware, &devices);

        Ok(Source::new(
            NvmlSensor::new(Arc::new(hardware), Arc::clone(&devices), self.poll_interval),
            NvmlProcessor::new(devices, self.poll_interval, models),
        ))
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
}

/// The model of each GPU, or why it cannot be read.
fn models<H: NvmlHardware>(hardware: &H, devices: &[Device]) -> Vec<InfoValue> {
    devices
        .iter()
        .map(|device| InfoValue::from(hardware.get_name(*device)))
        .collect()
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
            .minimal()
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
            .normal()
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
            Nvml::new().minimal().resolve(&mut hardware),
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

        assert_eq!(
            models(&hardware, &devices),
            [
                InfoValue::from("Some GPU"),
                InfoValue::from("unavailable (no GPU device detected)")
            ]
        );
    }
}
