use std::{io::ErrorKind, iter};

use joule_profiler_core::{info::Info, injector::Target};
use perf_event::{Builder, Counter, ReadFormat};

use crate::{
    counters::RawCount,
    error::{PerfEventError, Result},
    event::Event,
    scope::{Leader, PerfScope, counter_builder, open_counter},
};

/// Counts the profiled process and its children.
pub(crate) struct PidScope {
    group: bool,
    opened: Option<OpenedCounters>,
}

impl PidScope {
    pub(crate) fn new(group: bool) -> Self {
        Self {
            group,
            opened: None,
        }
    }
}

impl PerfScope for PidScope {
    fn info(&self) -> Info {
        Info::new().with("scope", "pid")
    }

    fn attach(&mut self, target: Target, events: &[Event]) -> Result<()> {
        let (first, others) = events.split_first().ok_or(PerfEventError::NoEvent)?;

        let mut leader = if self.group {
            open_group(*first, target.pid)?
        } else {
            let builder = pid_counter_builder(*first, target.pid);
            Leader::ReadEach(open_counter(&builder, *first, None)?)
        };

        let mut group_leader = self.group.then(|| leader.as_counter_mut());

        let counters = others
            .iter()
            .map(|event| {
                let builder = pid_counter_builder(*event, target.pid);
                open_counter(&builder, *event, group_leader.as_deref_mut())
            })
            .collect::<Result<_>>()?;

        self.opened = Some(OpenedCounters { leader, counters });
        Ok(())
    }

    fn read(&mut self, events: &[Event]) -> Result<Vec<RawCount>> {
        self.opened
            .as_mut()
            .ok_or(PerfEventError::NotOpened)?
            .read(events)
    }

    fn close(&mut self) -> Result<()> {
        self.opened = None;
        Ok(())
    }
}

/// The opened counters, leader first.
struct OpenedCounters {
    leader: Leader,
    counters: Vec<Counter>,
}

impl OpenedCounters {
    fn read(&mut self, events: &[Event]) -> Result<Vec<RawCount>> {
        match &mut self.leader {
            Leader::ReadGrouped(group) => {
                let data = group.read().map_err(PerfEventError::ReadGroup)?;
                let (enabled, running) = (data.time_enabled(), data.time_running());
                let readings = iter::once(group.as_counter())
                    .chain(&self.counters)
                    .map(|counter| RawCount::new(data[counter], enabled, running))
                    .collect();
                Ok(readings)
            }

            Leader::ReadEach(leader) => events
                .iter()
                .zip(iter::once(leader).chain(&mut self.counters))
                .map(|(event, counter)| {
                    counter
                        .read_full()
                        .map(RawCount::from)
                        .map_err(|error| PerfEventError::ReadCounter(*event, error))
                })
                .collect(),
        }
    }
}

/// Opens `event` as the group leader on the process `pid`. Falls back to a single counter on
/// kernels older than 4.13, which cannot read inherited counters as a group.
fn open_group(event: Event, pid: i32) -> Result<Leader> {
    match pid_counter_builder(event, pid)
        .read_format(
            ReadFormat::GROUP
                | ReadFormat::ID
                | ReadFormat::TOTAL_TIME_ENABLED
                | ReadFormat::TOTAL_TIME_RUNNING,
        )
        .build_group()
    {
        Ok(group) => Ok(Leader::ReadGrouped(group)),

        Err(err) if err.kind() == ErrorKind::InvalidInput => {
            log::debug!(
                "this kernel version cannot read inherited counters as a group ({err}): reading them one at a time"
            );

            Ok(Leader::ReadEach(
                pid_counter_builder(event, pid)
                    .read_format(ReadFormat::TOTAL_TIME_ENABLED | ReadFormat::TOTAL_TIME_RUNNING)
                    .build()
                    .map_err(PerfEventError::OpenGroup)?,
            ))
        }

        Err(err) => Err(match err.kind() {
            ErrorKind::PermissionDenied => PerfEventError::PermissionDenied(err),
            _ => PerfEventError::OpenGroup(err),
        }),
    }
}

/// Returns a builder that counts `event` on the process `pid` and its children.
fn pid_counter_builder(event: Event, pid: i32) -> Builder<'static> {
    let mut builder = counter_builder(event);
    builder.observe_pid(pid).inherit(true);
    builder
}
