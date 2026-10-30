use std::sync::Arc;

use joule_profiler_core::{MemoryUnit, MetricInfo, MetricValue, Processor};

use crate::{
    cgroup::USAGE_USEC_INDEX,
    error::Result,
    layout::{AvailableValues, Layout},
    sensor::{CgroupSensor, Reading, Snapshot},
};

/// Computes the metrics of each phase from two snapshots of the sensor.
pub(crate) struct CgroupProcessor {
    layout: Arc<Layout>,
}

impl CgroupProcessor {
    pub(crate) fn new(layout: Arc<Layout>) -> Self {
        Self { layout }
    }
}

impl Processor<CgroupSensor> for CgroupProcessor {
    fn metrics(&self) -> Vec<MetricInfo> {
        self.layout.metrics()
    }

    fn process(
        &mut self,
        previous: &Snapshot,
        current: &Snapshot,
        values: &mut Vec<MetricValue>,
    ) -> Result<()> {
        let elapsed = current.at.duration_since(previous.at).as_micros();

        write(
            &self.layout.proc,
            self.layout.process_memory_unit,
            &previous.proc,
            &current.proc,
            elapsed,
            values,
        );
        if let (Some(global_values), Some(previous), Some(current)) =
            (&self.layout.global, &previous.global, &current.global)
        {
            write(
                global_values,
                self.layout.global_memory_unit,
                previous,
                current,
                elapsed,
                values,
            );
        }

        Ok(())
    }
}

/// Writes the values of one cgroup over the phase, with the memory in `memory_unit`.
fn write(
    available_values: &AvailableValues,
    memory_unit: MemoryUnit,
    previous: &Reading,
    current: &Reading,
    elapsed: u128,
    values: &mut Vec<MetricValue>,
) {
    for (_, index) in available_values.memory() {
        let (min, max, mean) = current.memory[index].min_max_mean();
        values.extend([min, max, mean].map(|bytes| memory_unit.value(bytes)));
    }

    for (_, index) in available_values.cpu_counters() {
        values.push(counter_delta(previous.cpu[index], current.cpu[index]));
    }

    for (_, index) in available_values.io_counters() {
        values.push(counter_delta(previous.io[index], current.io[index]));
    }

    values.push(cpu_usage(previous, current, elapsed));
}

fn counter_delta(previous: Option<u64>, current: Option<u64>) -> MetricValue {
    MetricValue::U64(current.unwrap_or(0).saturating_sub(previous.unwrap_or(0)))
}

/// Compute the CPU usage of a phase.
#[allow(
    clippy::cast_precision_loss,
    reason = "a phase never lasts 2^52 microseconds"
)]
fn cpu_usage(previous: &Reading, current: &Reading, elapsed: u128) -> MetricValue {
    let spent = current.cpu[USAGE_USEC_INDEX]
        .zip(previous.cpu[USAGE_USEC_INDEX])
        .map_or(0, |(current, previous)| current.saturating_sub(previous));

    if elapsed == 0 {
        return MetricValue::F64(0.0);
    }

    MetricValue::F64(spent as f64 / elapsed as f64 * 100.0)
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use joule_profiler_core::util::stats::MinMaxMean;

    use super::*;
    use crate::cgroup::{CPU_COUNTERS, IO_COUNTERS, MEMORY};

    fn snapshot(global: bool, usage: u64) -> Snapshot {
        let reading = || Reading {
            memory: [MinMaxMean::default(); MEMORY.len()],
            cpu: [Some(usage); CPU_COUNTERS.len()],
            io: [Some(usage); IO_COUNTERS.len()],
        };
        Snapshot {
            at: Instant::now(),
            proc: reading(),
            global: global.then(reading),
        }
    }

    #[test]
    fn one_value_per_metric() {
        for global in [false, true] {
            let controllers = ["cpu", "memory", "io"].map(String::from);
            let available_values = AvailableValues::expected(&controllers);
            let layout = Arc::new(Layout {
                proc: available_values,
                global: global.then_some(available_values),
                process_memory_unit: MemoryUnit::default(),
                global_memory_unit: MemoryUnit::default(),
            });
            let mut processor = CgroupProcessor::new(Arc::clone(&layout));
            let mut values = Vec::new();

            processor
                .process(&snapshot(global, 100), &snapshot(global, 250), &mut values)
                .unwrap();

            assert_eq!(values.len(), processor.metrics().len());
            assert!(values.contains(&MetricValue::U64(150)));
        }
    }
}
