use joule_profiler_core::metric::{MetricInfo, MetricValue};
use joule_profiler_core::processor::Processor;
use joule_profiler_core::unit::MetricUnit;

use crate::error::Result;
use crate::hardware::AmdSmiHardware;
use crate::sensor::{AmdSmiSensor, DeviceSample, DeviceSupport, Devices, Snapshot};

pub struct AmdSmiProcessor {
    devices: Devices,
}

impl AmdSmiProcessor {
    pub(crate) fn new(devices: Devices) -> Self {
        Self { devices }
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
