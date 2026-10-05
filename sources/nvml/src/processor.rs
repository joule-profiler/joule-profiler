use std::time::Duration;

use joule_profiler_core::info::{Info, InfoValue};
use joule_profiler_core::metric::{MetricInfo, MetricValue};
use joule_profiler_core::processor::Processor;
use joule_profiler_core::unit::MetricUnit;

use crate::error::Result;
use crate::hardware::NvmlHardware;
use crate::sensor::{DeviceSample, DeviceSupport, Devices, NvmlSensor, Snapshot, polls};

pub struct NvmlProcessor {
    devices: Devices,
    poll_interval: Duration,

    /// The model of each GPU, read when the source is built.
    models: Vec<InfoValue>,
}

impl NvmlProcessor {
    pub(crate) fn new(devices: Devices, poll_interval: Duration, models: Vec<InfoValue>) -> Self {
        Self {
            devices,
            poll_interval,
            models,
        }
    }
}

impl<H: NvmlHardware> Processor<NvmlSensor<H>> for NvmlProcessor {
    fn metrics(&self) -> Vec<MetricInfo> {
        let mut metrics = Vec::new();

        for device in self.devices.iter() {
            let index = device.index;

            if device.support.intersects(DeviceSupport::ENERGY) {
                metrics.push(MetricInfo::new(
                    format!("GPU-{index}-energy"),
                    MetricUnit::MILLIJOULE,
                ));
            }

            if device.support.contains(DeviceSupport::Vram) {
                for end in ["min", "max", "avg"] {
                    metrics.push(MetricInfo::new(
                        format!("GPU-{index}-vram_{end}"),
                        MetricUnit::BYTE,
                    ));
                }
            }

            if device.support.contains(DeviceSupport::Utilization) {
                for end in ["min", "max", "avg"] {
                    metrics.push(MetricInfo::new(
                        format!("GPU-{index}-utilization_{end}"),
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
            |info, (device, model)| info.with(format!("GPU-{}", device.index), model.clone()),
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

/// The energy of the phase in millijoules, from the GPU counter or else from the integrated power.
fn energy(previous: &DeviceSample, current: &DeviceSample) -> u64 {
    if let Some((current, previous)) = current.energy_mj.zip(previous.energy_mj) {
        return current.saturating_sub(previous);
    }

    current.drawn_nj / 1_000_000
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hardware::MockNvmlHardware;
    use crate::sensor::Device;

    fn processor(support: DeviceSupport) -> NvmlProcessor {
        NvmlProcessor::new(
            vec![Device { index: 0, support }].into(),
            Duration::from_millis(10),
            vec![InfoValue::from("Some GPU")],
        )
    }

    fn info(processor: &NvmlProcessor) -> Info {
        Processor::<NvmlSensor<MockNvmlHardware>>::info(processor)
    }

    #[test]
    fn the_poll_interval_and_the_models_are_in_the_info() {
        let info = info(&processor(DeviceSupport::Power | DeviceSupport::Vram));

        assert_eq!(info.get("poll_interval"), Some(&"10ms".into()));
        assert_eq!(info.get("GPU-0"), Some(&"Some GPU".into()));
    }

    #[test]
    fn the_poll_interval_is_none_when_no_gpu_is_polled() {
        let info = info(&processor(DeviceSupport::Energy));

        assert_eq!(info.get("poll_interval"), Some(&"none".into()));
    }
}
