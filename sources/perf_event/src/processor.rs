use std::sync::Arc;

use joule_profiler_core::{
    metric::{MetricInfo, MetricValue},
    processor::Processor,
};

use crate::{
    counters::RawCount, error::Result, event::Event, scope::PerfScope, sensor::PerfSensor,
};

/// Computes the count of each event over a phase.
pub(crate) struct PerfProcessor {
    events: Arc<Vec<Event>>,
    group: bool,

    /// Whether the warning for a counter that never ran was logged.
    warned: bool,
}

impl PerfProcessor {
    pub(crate) fn new(events: Arc<Vec<Event>>, group: bool) -> Self {
        Self {
            events,
            group,
            warned: false,
        }
    }
}

impl<S: PerfScope> Processor<PerfSensor<S>> for PerfProcessor {
    fn metrics(&self) -> Vec<MetricInfo> {
        self.events
            .iter()
            .map(|event| MetricInfo::new(event.to_string(), event.unit()))
            .collect()
    }

    fn process(
        &mut self,
        previous: &Vec<RawCount>,
        current: &Vec<RawCount>,
        values: &mut Vec<MetricValue>,
    ) -> Result<()> {
        values.extend(self.events.iter().zip(current.iter().zip(previous)).map(
            |(event, (current, previous))| {
                MetricValue::U64(current.scaled_delta(previous).unwrap_or_else(|| {
                    if !self.warned {
                        self.warned = true;
                        if self.group {
                            log::warn!(
                                "the perf group was never scheduled during a phase, its counters are reported as 0: it probably does not fit in the hardware counters (set `group = false`)"
                            );
                        } else {
                            log::warn!(
                                "{event} was never scheduled during a phase and is reported as 0: too many events are scheduled on the hardware counters for a phase this short"
                            );
                        }
                    }
                    0
                }))
            },
        ));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::counters::tests::raw_count;
    use crate::scope::pid::PidScope;

    #[test]
    fn a_phase_reports_each_counter_scaled_and_0_for_one_that_never_ran() {
        let events = Arc::new(vec![Event::CpuCycles, Event::Instructions]);
        let mut processor = PerfProcessor::new(events, true);
        let previous = [raw_count(0, 0, 0), raw_count(0, 0, 0)].to_vec();
        let current = [raw_count(500, 2000, 1000), raw_count(300, 2000, 0)].to_vec();
        let mut values = Vec::new();

        <PerfProcessor as Processor<PerfSensor<PidScope>>>::process(
            &mut processor,
            &previous,
            &current,
            &mut values,
        )
        .unwrap();

        assert_eq!(values, [MetricValue::U64(1000), MetricValue::U64(0)]);
    }
}
