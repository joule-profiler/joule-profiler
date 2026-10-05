use std::time::Duration;

use joule_profiler_core::info::{Info, InfoValue};
use joule_profiler_core::metric::{MetricInfo, MetricValue};
use joule_profiler_core::processor::Processor;
use joule_profiler_core::unit::MetricUnit;

use crate::error::Result;
use crate::hardware::AmdSmiHardware;
use crate::sensor::{AmdSmiSensor, DeviceSample, DeviceSupport, Devices, Snapshot, polls};

pub struct AmdSmiProcessor {
    devices: Devices,
    poll_interval: Duration,

    /// The model of each GPU, read when the source is built.
    models: Vec<InfoValue>,
}

impl AmdSmiProcessor {
    pub(crate) fn new(devices: Devices, poll_interval: Duration, models: Vec<InfoValue>) -> Self {
        Self {
            devices,
            poll_interval,
            models,
        }
    }
}

impl<H: AmdSmiHardware> Processor<AmdSmiSensor<H>> for AmdSmiProcessor {
    fn metrics(&self) -> Vec<MetricInfo> {
        let mut metrics = Vec::new();

        for device in self.devices.iter() {
            let uuid = &device.uuid;

            if device.support.intersects(DeviceSupport::ENERGY) {
                metrics.push(MetricInfo::new(
                    format!("GPU-{uuid}-energy"),
                    MetricUnit::MICROJOULE,
                ));
            }

            if device.support.contains(DeviceSupport::Vram) {
                for end in ["min", "max", "avg"] {
                    metrics.push(MetricInfo::new(
                        format!("GPU-{uuid}-vram_{end}"),
                        MetricUnit::BYTE,
                    ));
                }
            }

            if device.support.contains(DeviceSupport::Utilization) {
                for end in ["min", "max", "avg"] {
                    metrics.push(MetricInfo::new(
                        format!("GPU-{uuid}-utilization_{end}"),
                        MetricUnit::PERCENT,
                    ));
                }
            }
        }

        metrics
    }

    /// The poll interval, then the model of each GPU.
    fn info(&self) -> Info {
        let polling = if polls(&self.devices) {
            InfoValue::from(self.poll_interval)
        } else {
            InfoValue::from("none")
        };

        self.devices.iter().zip(&self.models).fold(
            Info::new().with("poll_interval", polling),
            |info, (device, model)| info.with(&device.uuid, model.clone()),
        )
    }

    fn process(
        &mut self,
        previous: &Snapshot,
        current: &Snapshot,
        values: &mut Vec<MetricValue>,
    ) -> Result<()> {
        let mut next = |value: u64| values.push(MetricValue::U64(value));

        let samples = self
            .devices
            .iter()
            .zip(&current.devices)
            .zip(&previous.devices);

        for ((device, current), previous) in samples {
            if device.support.intersects(DeviceSupport::ENERGY) {
                next(energy(previous, current));
            }

            if device.support.contains(DeviceSupport::Vram) {
                let (low, high, mean) = current.vram.min_max_mean();

                next(low);
                next(high);
                next(mean);
            }

            if device.support.contains(DeviceSupport::Utilization) {
                let (low, high, mean) = current.utilization.min_max_mean();

                next(low);
                next(high);
                next(mean);
            }
        }

        Ok(())
    }
}

/// The energy of the phase in microjoules, from the GPU counter or else from the integrated power.
fn energy(previous: &DeviceSample, current: &DeviceSample) -> u64 {
    if let Some((current, previous)) = current.energy_uj.zip(previous.energy_uj) {
        return current.saturating_sub(previous);
    }

    current.drawn_nj / 1_000
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hardware::MockAmdSmiHardware;
    use crate::sensor::Device;

    const UUID: &str = "5bff74bf-0000-1000-80ab-6aa8d0e43a2c";

    fn processor(support: DeviceSupport) -> AmdSmiProcessor {
        AmdSmiProcessor::new(
            vec![Device {
                uuid: UUID.to_owned(),
                support,
            }]
            .into(),
            Duration::from_millis(10),
            vec![InfoValue::from("Some GPU")],
        )
    }

    fn info(processor: &AmdSmiProcessor) -> Info {
        Processor::<AmdSmiSensor<MockAmdSmiHardware>>::info(processor)
    }

    #[test]
    fn the_poll_interval_and_the_models_are_in_the_info() {
        let info = info(&processor(DeviceSupport::Power | DeviceSupport::Vram));

        assert_eq!(info.get("poll_interval"), Some(&"10ms".into()));
        assert_eq!(info.get(UUID), Some(&"Some GPU".into()));
    }

    #[test]
    fn the_poll_interval_is_none_when_no_gpu_is_polled() {
        let info = info(&processor(DeviceSupport::Energy));

        assert_eq!(info.get("poll_interval"), Some(&"none".into()));
    }
}
