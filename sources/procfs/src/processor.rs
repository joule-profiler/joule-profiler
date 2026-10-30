use joule_profiler_core::{
    Info, MemoryUnit, MetricInfo, MetricUnit, MetricValue, Processor, util::stats::MinMaxMean,
};

use crate::{
    Procfs,
    error::Result,
    sensor::{
        CPU_METRICS, GLOBAL_CPU_METRICS, GLOBAL_MEMORY_METRICS, IO_METRICS, PROCESS_METRICS,
        ProcfsSensor, Snapshot,
    },
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
        .with("global_poll_interval", config.global_poll_interval)
        .with("process_poll_interval", config.process_poll_interval)
        .with("hierarchy_poll_interval", config.hierarchy_poll_interval)
        .with(
            "process_memory_unit",
            MetricUnit::from(config.process_memory_unit).to_string(),
        )
        .with(
            "global_memory_unit",
            MetricUnit::from(config.global_memory_unit).to_string(),
        )
        .with("mem_total_bytes", mem_total)
        .with("clock_tick_ms", tick_ms)
}

impl Processor<ProcfsSensor> for ProcfsProcessor {
    fn metrics(&self) -> Vec<MetricInfo> {
        let mut metrics = memory_metrics(&PROCESS_METRICS, self.config.process_memory_unit);
        if self.config.global {
            metrics.extend(memory_metrics(
                &GLOBAL_MEMORY_METRICS,
                self.config.global_memory_unit,
            ));
        }

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
        write_memory(
            &current.memory.proc,
            self.config.process_memory_unit,
            values,
        );
        if self.config.global {
            write_memory(
                &current.memory.global,
                self.config.global_memory_unit,
                values,
            );
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

/// Returns the minimum, maximum and mean metrics of each memory value, in `unit`.
fn memory_metrics(names: &[&str], unit: MemoryUnit) -> Vec<MetricInfo> {
    names
        .iter()
        .flat_map(|name| {
            ["min", "max", "avg"]
                .map(|end| MetricInfo::new(format!("{name}_{end}"), MetricUnit::from(unit)))
        })
        .collect()
}

/// Writes the minimum, maximum and mean of each memory value, in `unit`.
fn write_memory(stats: &[MinMaxMean], unit: MemoryUnit, values: &mut Vec<MetricValue>) {
    for stat in stats {
        let (min, max, mean) = stat.min_max_mean();
        values.extend([min, max, mean].map(|bytes| unit.value(bytes)));
    }
}
