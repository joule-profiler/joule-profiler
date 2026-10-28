use std::sync::Arc;

use joule_profiler_core::metric::{MetricInfo, MetricValue};
use joule_profiler_core::processor::Processor;

use crate::cgroup::USAGE_USEC_INDEX;
use crate::error::Result;
use crate::layout::Layout;
use crate::sensor::{CgroupSensor, Reading, Snapshot};

pub struct CgroupProcessor {
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

        self.write(&previous.proc, &current.proc, elapsed, values);
        if let (Some(previous), Some(current)) = (&previous.global, &current.global) {
            self.write(previous, current, elapsed, values);
        }

        Ok(())
    }
}

impl CgroupProcessor {
    fn write(
        &self,
        previous: &Reading,
        current: &Reading,
        elapsed: u128,
        values: &mut Vec<MetricValue>,
    ) {
        for (_, index) in self.layout.memory() {
            let (min, max, mean) = current.memory[index].min_max_mean();
            values.extend([min, max, mean].map(MetricValue::U64));
        }

        for (_, index) in self.layout.counters() {
            let read = |reading: &Reading| reading.counters[index].unwrap_or(0);
            values.push(MetricValue::U64(
                read(current).saturating_sub(read(previous)),
            ));
        }

        values.push(cpu_usage(previous, current, elapsed));
    }
}

/// The CPU usage over the phase.
#[allow(
    clippy::cast_precision_loss,
    reason = "a phase never lasts 2^52 microseconds"
)]
fn cpu_usage(previous: &Reading, current: &Reading, elapsed: u128) -> MetricValue {
    let spent = current.counters[USAGE_USEC_INDEX]
        .zip(previous.counters[USAGE_USEC_INDEX])
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
    use crate::cgroup::{COUNTERS, MEMORY};

    fn snapshot(global: bool, usage: u64) -> Snapshot {
        let reading = || Reading {
            memory: [MinMaxMean::default(); MEMORY.len()],
            counters: [Some(usage); COUNTERS.len()],
        };
        Snapshot {
            at: Instant::now(),
            proc: reading(),
            global: global.then(reading),
        }
    }

    #[test]
    fn a_value_is_written_for_every_metric_with_or_without_the_hierarchy() {
        for global in [false, true] {
            let controllers = ["cpu", "memory", "io"].map(String::from);
            let layout = Arc::new(Layout::expected(&controllers, global));
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
