use joule_profiler_core::info::Info;
use joule_profiler_core::metric::{MetricInfo, MetricValue};
use joule_profiler_core::processor::Processor;
use joule_profiler_core::unit::MetricUnit;
use joule_profiler_core::util::stats::MinMaxMean;

use crate::Procfs;
use crate::error::Result;
use crate::sensor::{CPU, GLOBAL, IO, PROC, ProcfsSensor, Snapshot};

pub struct ProcfsProcessor {
    config: Procfs,

    /// Turns the clock ticks of `/proc/<pid>/stat` into milliseconds.
    tick_ms: u64,

    mem_total: u64,
}

impl ProcfsProcessor {
    pub(crate) fn new(config: Procfs, ticks_per_second: u64, mem_total: u64) -> Self {
        Self {
            config,
            tick_ms: 1000 / ticks_per_second.max(1),
            mem_total,
        }
    }
}

impl Processor<ProcfsSensor> for ProcfsProcessor {
    fn metrics(&self) -> Vec<MetricInfo> {
        let global: &[&str] = if self.config.global { &GLOBAL } else { &[] };

        let mut metrics: Vec<MetricInfo> = PROC
            .iter()
            .chain(global)
            .flat_map(|name| {
                ["min", "max", "avg"]
                    .map(|end| MetricInfo::new(format!("{name}_{end}"), MetricUnit::BYTE))
            })
            .collect();

        metrics.extend(IO.map(|name| MetricInfo::new(name, MetricUnit::BYTE)));
        metrics.extend(CPU.map(|name| MetricInfo::new(name, MetricUnit::MILLISECOND)));
        metrics
    }

    fn info(&self) -> Info {
        Info::new()
            .with("global", self.config.global)
            .with("poll_interval", self.config.poll_interval)
            .with("process_poll_interval", self.config.process_poll_interval)
            .with("hierarchy_interval", self.config.hierarchy_interval)
            .with("mem_total_bytes", self.mem_total)
            .with("clock_tick_ms", self.tick_ms)
    }

    fn process(
        &mut self,
        previous: &Snapshot,
        current: &Snapshot,
        values: &mut Vec<MetricValue>,
    ) -> Result<()> {
        let global: &[MinMaxMean] = if self.config.global {
            &current.polled.global
        } else {
            &[]
        };

        for memory in current.polled.proc.iter().chain(global) {
            let (min, max, mean) = memory.min_max_mean();
            values.extend([min, max, mean].map(MetricValue::U64));
        }

        for (current, previous) in current.io.iter().zip(&previous.io) {
            values.push(MetricValue::U64(current.saturating_sub(*previous)));
        }

        for (current, previous) in current.cpu.iter().zip(&previous.cpu) {
            let ticks = current.saturating_sub(*previous);

            values.push(MetricValue::U64(ticks * self.tick_ms));
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[test]
    fn the_intervals_the_memory_and_the_clock_tick_are_in_the_info() {
        let config = Procfs::new()
            .global(true)
            .process_poll(Duration::from_millis(20));

        let info = ProcfsProcessor::new(config, 100, 4096).info();

        assert_eq!(info.get("global"), Some(&true.into()));
        assert_eq!(info.get("process_poll_interval"), Some(&"20ms".into()));
        assert_eq!(info.get("hierarchy_interval"), Some(&"1s".into()));
        assert_eq!(info.get("mem_total_bytes"), Some(&4096_u64.into()));
        assert_eq!(info.get("clock_tick_ms"), Some(&10_u64.into()));
    }
}
