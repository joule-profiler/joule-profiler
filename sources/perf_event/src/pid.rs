//! Counters that follow a process and the processes it starts.

use std::io::{self, ErrorKind};

use joule_profiler_core::injector::Target;
use perf_event::events::{Hardware, Software};
use perf_event::{Builder, Counter, Group, ReadFormat};

use crate::error::{PerfEventError, Result};
use crate::event::Event;
use crate::scope::PerfScope;

enum Leader {
    /// Read in one syscall.
    Grouped(Group),

    /// Read one counter at a time, on kernels that refuse `inherit` with `PERF_FORMAT_GROUP`.
    OneByOne(Counter),
}

impl Leader {
    fn open(pid: i32) -> Result<Self> {
        let format = ReadFormat::GROUP | ReadFormat::ID;

        match observing(Software::DUMMY, pid)
            .read_format(format)
            .build_group()
        {
            Ok(group) => Ok(Self::Grouped(group)),

            // Before Linux 4.13.
            Err(error) if error.kind() == ErrorKind::InvalidInput => {
                log::debug!(
                    "this kernel will not read inherited counters as a group ({error}): reading them one at a time"
                );

                Ok(Self::OneByOne(
                    observing(Software::DUMMY, pid)
                        .build()
                        .map_err(open_error)?,
                ))
            }

            Err(error) => Err(open_error(error)),
        }
    }

    fn as_counter_mut(&mut self) -> &mut Counter {
        match self {
            Self::Grouped(group) => group.as_counter_mut(),
            Self::OneByOne(counter) => counter,
        }
    }

    fn enable(&mut self) -> Result<()> {
        match self {
            Self::Grouped(group) => group.enable(),
            Self::OneByOne(counter) => counter.enable_group(),
        }
        .map_err(PerfEventError::Switch)
    }

    fn disable(&mut self) -> Result<()> {
        match self {
            Self::Grouped(group) => group.disable(),
            Self::OneByOne(counter) => counter.disable_group(),
        }
        .map_err(PerfEventError::Switch)
    }
}

struct Opened {
    leader: Leader,
    counters: Vec<Counter>,
}

impl Opened {
    fn read(&mut self, events: &[Event]) -> Result<Vec<u64>> {
        match &mut self.leader {
            Leader::Grouped(group) => {
                let data = group.read().map_err(PerfEventError::ReadCounters)?;

                Ok(self
                    .counters
                    .iter()
                    .map(|counter| data.get(counter).map_or(0, |entry| entry.value()))
                    .collect())
            }

            Leader::OneByOne(_) => events
                .iter()
                .zip(&mut self.counters)
                .map(|(event, counter)| {
                    counter
                        .read()
                        .map_err(|e| PerfEventError::ReadCounter(*event, e))
                })
                .collect(),
        }
    }
}

pub struct PidScope {
    events: Vec<Event>,
    opened: Option<Opened>,
}

impl PidScope {
    pub(crate) fn new(events: Vec<Event>) -> Self {
        Self {
            events,
            opened: None,
        }
    }
}

impl PerfScope for PidScope {
    fn events(&self) -> &[Event] {
        &self.events
    }

    fn attach(&mut self, target: Target) -> Result<()> {
        let mut leader = Leader::open(target.pid)?;

        let mut counters = Vec::with_capacity(self.events.len());

        for event in &self.events {
            let counter = observing(Hardware::from(*event), target.pid)
                .build_with_group(leader.as_counter_mut())
                .map_err(|e| match e.kind() {
                    ErrorKind::PermissionDenied => PerfEventError::PermissionDenied(e),
                    _ => PerfEventError::OpenCounter(*event, e),
                })?;

            counters.push(counter);
        }

        leader.enable()?;
        self.opened = Some(Opened { leader, counters });

        Ok(())
    }

    fn read(&mut self) -> Result<Vec<u64>> {
        let events = &self.events;
        let opened = self.opened.as_mut().ok_or(PerfEventError::NotOpened)?;

        opened.read(events)
    }

    fn close(&mut self) -> Result<()> {
        if let Some(opened) = &mut self.opened {
            opened.leader.disable()?;
        }

        self.opened = None;
        Ok(())
    }
}

fn observing<E: perf_event::events::Event>(event: E, pid: i32) -> Builder<'static> {
    let mut builder = Builder::new(event);
    builder
        .inherit(true)
        .observe_pid(pid)
        .include_hv()
        .include_kernel()
        .exclude_guest(false)
        .exclude_host(false);
    builder
}

fn open_error(error: io::Error) -> PerfEventError {
    match error.kind() {
        ErrorKind::PermissionDenied => PerfEventError::PermissionDenied(error),
        _ => PerfEventError::OpenLeader(error),
    }
}
