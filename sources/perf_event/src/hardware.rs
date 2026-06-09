use crate::{Result, error::PerfEventError, event::Event, snapshot::Snapshot};
use log::{debug, info, trace};
use perf_event::{Builder, Counter, events::Hardware};
use std::collections::HashMap;

#[cfg_attr(test, mockall::automock)]
pub trait PerfEventHardware: Send + Default {
    fn init_counters(&mut self, events: &[Event], pid: i32) -> Result<()>;
    fn read_snapshot(&mut self) -> Result<Snapshot>;
}

#[derive(Default)]
pub struct PerfEventCounters {
    counters: HashMap<Event, Counter>,
}

impl PerfEventHardware for PerfEventCounters {
    /// Create individual counters for all configured events.
    ///
    /// Each counter is built separately with `inherit(true)` and `observe_pid`,
    /// since grouped counters do not support inheritance.
    fn init_counters(&mut self, events: &[Event], pid: i32) -> Result<()> {
        debug!("Adding {} individual performance counters", events.len());
        for event in events {
            trace!("Building counter: {event:?}");
            let counter = Builder::new(Hardware::from(*event))
                .inherit(true)
                .observe_pid(pid)
                .include_hv()
                .include_kernel()
                .build()?;
            self.counters.insert(*event, counter);
        }
        info!("Initialized {} hardware performance counters", events.len());
        for (event, counter) in &mut self.counters {
            trace!("Enabling counter: {event:?}");
            counter.enable()?;
        }
        debug!("All perf_event counters enabled");
        Ok(())
    }

    /// Reads all performance counters and returns a snapshot.
    ///
    /// An error can occur if an event counter cannot be read.
    fn read_snapshot(&mut self) -> Result<Snapshot> {
        let metrics = self
            .counters
            .iter_mut()
            .map(|(event, counter)| {
                let value = counter
                    .read()
                    .map_err(|_| PerfEventError::ErrorReadingCounter(*event))?;
                Ok((*event, value))
            })
            .collect::<Result<HashMap<_, _>>>()?;
        Ok(Snapshot { metrics })
    }
}
