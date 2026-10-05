use std::{fs, io};

use joule_profiler_core::info::{Info, InfoValue};
use joule_profiler_core::injector::Target;
use joule_profiler_core::metric::{MetricInfo, MetricValue};
use joule_profiler_core::processor::Processor;
use joule_profiler_core::sensor::Sensor;
use joule_profiler_core::unit::MetricUnit;

use crate::error::{PerfEventError, Result};
use crate::event::Event;

const PMUS: &str = "/sys/bus/event_source/devices";
const PARANOID: &str = "/proc/sys/kernel/perf_event_paranoid";

/// What the counters are opened on: a process tree or a cgroup.
pub trait PerfScope: Send + 'static {
    fn events(&self) -> &[Event];

    /// Opens the counters that do not need the target.
    fn open(&mut self) -> Result<()> {
        Ok(())
    }

    /// Opens the counters on the target, started but not resumed yet.
    fn attach(&mut self, _target: Target) -> Result<()> {
        Ok(())
    }

    fn read(&mut self) -> Result<Vec<u64>>;

    fn close(&mut self) -> Result<()> {
        Ok(())
    }
}

pub struct PerfSensor<S: PerfScope> {
    scope: S,
}

impl<S: PerfScope> PerfSensor<S> {
    pub(crate) fn new(scope: S) -> Self {
        Self { scope }
    }
}

impl<S: PerfScope> Sensor for PerfSensor<S> {
    type Snapshot = Vec<u64>;

    type Error = PerfEventError;

    fn name(&self) -> &'static str {
        "perf_event"
    }

    fn init(&mut self) -> Result<()> {
        self.scope.open()
    }

    fn attach(&mut self, target: Target) -> Result<()> {
        self.scope.attach(target)
    }

    fn measure(&mut self) -> Result<Self::Snapshot> {
        self.scope.read()
    }

    fn close(&mut self) -> Result<()> {
        self.scope.close()
    }
}

/// The PMUs of the machine, sorted by name.
fn pmus() -> io::Result<Vec<String>> {
    let mut pmus = fs::read_dir(PMUS)?
        .map(|entry| entry.map(|entry| entry.file_name().to_string_lossy().into_owned()))
        .collect::<io::Result<Vec<_>>>()?;

    pmus.sort_unstable();
    Ok(pmus)
}

fn paranoid() -> io::Result<i64> {
    fs::read_to_string(PARANOID)?
        .trim()
        .parse()
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

pub struct PerfProcessor {
    events: Vec<Event>,

    /// What the counters are opened on, as the scope reports it.
    scope: Info,
}

impl PerfProcessor {
    pub(crate) fn new(events: Vec<Event>, scope: Info) -> Self {
        Self { events, scope }
    }
}

impl<S: PerfScope> Processor<PerfSensor<S>> for PerfProcessor {
    fn metrics(&self) -> Vec<MetricInfo> {
        self.events
            .iter()
            .map(|event| MetricInfo::new(event.to_string(), MetricUnit::COUNT))
            .collect()
    }

    fn info(&self) -> Info {
        self.scope
            .clone()
            .with("events", InfoValue::list(&self.events))
            .with("pmus", pmus().map(InfoValue::list))
            .with("perf_event_paranoid", paranoid())
    }

    fn process(
        &mut self,
        previous: &Vec<u64>,
        current: &Vec<u64>,
        values: &mut Vec<MetricValue>,
    ) -> Result<()> {
        values.extend(
            current
                .iter()
                .zip(previous)
                .map(|(current, previous)| MetricValue::U64(current.wrapping_sub(*previous))),
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pid::PidScope;

    #[test]
    fn the_info_is_the_scope_then_the_events() {
        let events = vec![Event::Instructions, Event::CpuCycles];
        let processor = PerfProcessor::new(events, Info::new().with("scope", "pid"));

        let info = Processor::<PerfSensor<PidScope>>::info(&processor);

        let keys: Vec<&str> = info.entries().iter().map(|(key, _)| key.as_str()).collect();
        assert_eq!(keys, ["scope", "events", "pmus", "perf_event_paranoid"]);
        assert_eq!(
            info.get("events"),
            Some(&InfoValue::list(["INSTRUCTIONS", "CPU_CYCLES"]))
        );
    }
}
