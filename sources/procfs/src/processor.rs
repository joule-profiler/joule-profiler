use joule_profiler_core::info::Info;
use joule_profiler_core::metric::{MetricInfo, MetricValue};
use joule_profiler_core::processor::Processor;
use joule_profiler_core::unit::MetricUnit;
use joule_profiler_core::util::stats::MinMaxMean;

use crate::Procfs;
use crate::error::Result;
use crate::sensor::{
    CPU_METRICS, GLOBAL_CPU_METRICS, GLOBAL_MEMORY_METRICS, IO_METRICS, PROCESS_METRICS,
    ProcfsSensor, Snapshot,
};

/// Computes the metrics of each phase from two snapshots of the sensor.
pub struct ProcfsProcessor {
    config: Procfs,

    /// Used to turn the ticks of `/proc/<pid>/stat` into milliseconds.
    tick_ms: u64,
}

impl ProcfsProcessor {
    pub(crate) fn new(config: Procfs, tick_ms: u64) -> Self {
        Self { config, tick_ms }
    }
}

pub(crate) fn info(config: &Procfs, tick_ms: u64, mem_total: u64) -> Info {
    Info::new()
        .with("global", config.global)
        .with("poll_interval", config.global_poll_interval)
        .with("process_poll_interval", config.process_poll_interval)
        .with("hierarchy_poll_interval", config.hierarchy_poll_interval)
        .with("mem_total_bytes", mem_total)
        .with("clock_tick_ms", tick_ms)
}

impl Processor<ProcfsSensor> for ProcfsProcessor {
    fn metrics(&self) -> Vec<MetricInfo> {
        let global: &[&str] = if self.config.global {
            &GLOBAL_MEMORY_METRICS
        } else {
            &[]
        };

        let mut metrics: Vec<MetricInfo> = PROCESS_METRICS
            .iter()
            .chain(global)
            .flat_map(|name| {
                ["min", "max", "avg"]
                    .map(|end| MetricInfo::new(format!("{name}_{end}"), MetricUnit::BYTE))
            })
            .collect();

        metrics.extend(IO_METRICS.map(|name| MetricInfo::new(name, MetricUnit::BYTE)));
        metrics.extend(CPU_METRICS.map(|name| MetricInfo::new(name, MetricUnit::MILLISECOND)));
        if self.config.global {
            metrics.extend(
                GLOBAL_CPU_METRICS.map(|name| MetricInfo::new(name, MetricUnit::MILLISECOND)),
            );
        }
        metrics
    }

    fn process(
        &mut self,
        previous: &Snapshot,
        current: &Snapshot,
        values: &mut Vec<MetricValue>,
    ) -> Result<()> {
        let global: &[MinMaxMean] = if self.config.global {
            &current.memory.global
        } else {
            &[]
        };

        for memory in current.memory.proc.iter().chain(global) {
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

        if self.config.global {
            for (current, previous) in current.global_cpu.iter().zip(&previous.global_cpu) {
                let ticks = current.saturating_sub(*previous);
                values.push(MetricValue::U64(ticks * self.tick_ms));
            }
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
            .process_poll_interval(Duration::from_millis(20));

        let info = info(&config, 10, 4096);

        assert_eq!(info.get("global"), Some(&true.into()));
        assert_eq!(info.get("process_poll_interval"), Some(&"20ms".into()));
        assert_eq!(info.get("hierarchy_poll_interval"), Some(&"500ms".into()));
        assert_eq!(info.get("mem_total_bytes"), Some(&4096_u64.into()));
        assert_eq!(info.get("clock_tick_ms"), Some(&10_u64.into()));
    }
}
