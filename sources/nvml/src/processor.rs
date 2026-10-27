use joule_profiler_core::metric::{MetricInfo, MetricValue};
use joule_profiler_core::processor::Processor;
use joule_profiler_core::unit::MetricUnit;

use crate::error::Result;
use crate::hardware::NvmlHardware;
use crate::sensor::{DeviceSample, DeviceSupport, Devices, NvmlSensor, Snapshot};

pub struct NvmlProcessor {
    devices: Devices,
}

impl NvmlProcessor {
    pub(crate) fn new(devices: Devices) -> Self {
        Self { devices }
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

    fn process(
        &mut self,
        previous: &Snapshot,
        current: &Snapshot,
        values: &mut Vec<MetricValue>,
    ) -> Result<()> {
        let mut push = |value: u64| values.push(MetricValue::U64(value));

        let samples = self
            .devices
            .iter()
            .zip(&current.devices)
            .zip(&previous.devices);

        for ((device, current), previous) in samples {
            if device.support.intersects(DeviceSupport::ENERGY) {
                push(energy(previous, current));
            }

            if device.support.contains(DeviceSupport::Vram) {
                let (low, high, mean) = current.vram.min_max_mean();

                push(low);
                push(high);
                push(mean);
            }

            if device.support.contains(DeviceSupport::Utilization) {
                let (low, high, mean) = current.utilization.min_max_mean();

                push(low);
                push(high);
                push(mean);
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
