use joule_profiler_core::{MemoryUnit, MetricInfo, MetricUnit, MetricValue, Processor};

use crate::{
    error::Result,
    hardware::AmdSmiHardware,
    sensor::{AmdSmiSensor, DeviceSample, DeviceSupport, Devices, Snapshot},
};

/// Computes the metrics of each phase from two snapshots of the sensor.
pub(crate) struct AmdSmiProcessor {
    devices: Devices,
    memory_unit: MemoryUnit,
}

impl AmdSmiProcessor {
    pub(crate) fn new(devices: Devices, memory_unit: MemoryUnit) -> Self {
        Self {
            devices,
            memory_unit,
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
                metrics.extend(["min", "max", "avg"].map(|end| {
                    MetricInfo::new(
                        format!("GPU-{uuid}-vram_{end}"),
                        MetricUnit::from(self.memory_unit),
                    )
                }));
            }

            if device.support.contains(DeviceSupport::Utilization) {
                metrics.extend(["min", "max", "avg"].map(|end| {
                    MetricInfo::new(format!("GPU-{uuid}-utilization_{end}"), MetricUnit::PERCENT)
                }));
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
        let samples = self
            .devices
            .iter()
            .zip(&current.devices)
            .zip(&previous.devices);

        for ((device, current), previous) in samples {
            if device.support.intersects(DeviceSupport::ENERGY) {
                values.push(MetricValue::U64(energy(previous, current)));
            }

            if device.support.contains(DeviceSupport::Vram) {
                let (min, max, mean) = current.vram.min_max_mean();
                values.extend([min, max, mean].map(|bytes| self.memory_unit.value(bytes)));
            }

            if device.support.contains(DeviceSupport::Utilization) {
                let (min, max, mean) = current.utilization.min_max_mean();
                values.extend([min, max, mean].map(MetricValue::U64));
            }
        }

        Ok(())
    }
}

/// Returns the energy of the phase in microjoules, from the energy counter or else from the
/// integrated power.
fn energy(previous: &DeviceSample, current: &DeviceSample) -> u64 {
    if let Some((current, previous)) = current.energy_uj.zip(previous.energy_uj) {
        return current.saturating_sub(previous);
    }

    current.drawn_nj / 1_000
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(energy_uj: Option<u64>, drawn_nj: u64) -> DeviceSample {
        DeviceSample {
            energy_uj,
            drawn_nj,
            ..DeviceSample::default()
        }
    }

    #[test]
    fn energy_from_counter_or_power() {
        assert_eq!(
            energy(&sample(Some(1_000), 0), &sample(Some(1_250), 0)),
            250
        );
        assert_eq!(energy(&sample(None, 0), &sample(None, 3_000)), 3);
    }
}
