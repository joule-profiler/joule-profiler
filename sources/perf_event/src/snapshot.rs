use std::collections::HashMap;

use crate::event::Event;

/// Snapshot of `perf_event` counters, keyed by event and then by CPU.
///
/// Kept unsummed across CPUs (PID-scoped counters just use a single implicit
/// CPU key) so the per-CPU total is only computed once, in `to_metrics`,
/// instead of on every `read_snapshot` call.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Snapshot {
    pub metrics: HashMap<Event, HashMap<usize, u64>>,
}

/// A pair of snapshots delimiting a phase.
#[derive(Debug, Clone, Default)]
pub struct Phase {
    /// The snapshot made at the start of a phase.
    pub begin: Snapshot,

    /// End snapshot of the phase.
    pub end: Snapshot,
}

impl Phase {
    /// Computes the per-event, per-CPU delta between begin and end.
    pub fn diff(&self) -> Snapshot {
        let metrics = self
            .end
            .metrics
            .iter()
            .map(|(event, end_per_cpu)| {
                let begin_per_cpu = self.begin.metrics.get(event);

                let deltas = end_per_cpu
                    .iter()
                    .map(|(cpu, &current_value)| {
                        let delta = begin_per_cpu
                            .and_then(|m| m.get(cpu))
                            .map_or(current_value, |&prev| current_value.wrapping_sub(prev));
                        (*cpu, delta)
                    })
                    .collect();

                (*event, deltas)
            })
            .collect();

        Snapshot { metrics }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::Event;

    /// A single-CPU snapshot, for tests that don't care about per-CPU
    /// breakdown (e.g. PID-scoped counters, which only ever have one entry).
    fn snapshot(metrics: Vec<(Event, u64)>) -> Snapshot {
        Snapshot {
            metrics: metrics
                .into_iter()
                .map(|(event, value)| (event, HashMap::from([(0, value)])))
                .collect(),
        }
    }

    fn total(snapshot: &Snapshot, event: Event) -> u64 {
        snapshot.metrics[&event].values().sum()
    }

    #[test]
    fn diff_basic_delta() {
        let phase = Phase {
            begin: snapshot(vec![(Event::CpuCycles, 100)]),
            end: snapshot(vec![(Event::CpuCycles, 350)]),
        };
        assert_eq!(total(&phase.diff(), Event::CpuCycles), 250);
    }

    #[test]
    fn diff_multiple_events() {
        let phase = Phase {
            begin: snapshot(vec![(Event::CpuCycles, 100), (Event::Instructions, 200)]),
            end: snapshot(vec![(Event::CpuCycles, 400), (Event::Instructions, 500)]),
        };
        let diff = phase.diff();
        assert_eq!(total(&diff, Event::CpuCycles), 300);
        assert_eq!(total(&diff, Event::Instructions), 300);
    }

    #[test]
    fn diff_equal_values_returns_zero() {
        let phase = Phase {
            begin: snapshot(vec![(Event::CpuCycles, 42)]),
            end: snapshot(vec![(Event::CpuCycles, 42)]),
        };
        assert_eq!(total(&phase.diff(), Event::CpuCycles), 0);
    }

    #[test]
    fn diff_wraps_on_counter_overflow() {
        let phase = Phase {
            begin: snapshot(vec![(Event::CpuCycles, u64::MAX - 5)]),
            end: snapshot(vec![(Event::CpuCycles, 10)]),
        };
        assert_eq!(total(&phase.diff(), Event::CpuCycles), 16);
    }

    #[test]
    fn diff_event_missing_in_begin_uses_end_value() {
        let phase = Phase {
            begin: snapshot(vec![]),
            end: snapshot(vec![(Event::CacheMisses, 77)]),
        };
        assert_eq!(total(&phase.diff(), Event::CacheMisses), 77);
    }

    #[test]
    fn diff_event_missing_in_end_is_absent_from_result() {
        let phase = Phase {
            begin: snapshot(vec![(Event::CpuCycles, 100), (Event::Instructions, 200)]),
            end: snapshot(vec![(Event::CpuCycles, 150)]),
        };
        let diff = phase.diff();
        assert_eq!(diff.metrics.len(), 1);
        assert_eq!(total(&diff, Event::CpuCycles), 50);
        assert!(!diff.metrics.contains_key(&Event::Instructions));
    }

    #[test]
    fn diff_empty_snapshots_returns_empty() {
        let phase = Phase::default();
        assert_eq!(phase.diff(), Snapshot::default());
    }

    #[test]
    fn diff_sums_independent_per_cpu_deltas() {
        let phase = Phase {
            begin: Snapshot {
                metrics: HashMap::from([(Event::CpuCycles, HashMap::from([(0, 100), (1, 500)]))]),
            },
            end: Snapshot {
                metrics: HashMap::from([(Event::CpuCycles, HashMap::from([(0, 300), (1, 550)]))]),
            },
        };
        // cpu 0: 300 - 100 = 200; cpu 1: 550 - 500 = 50; total = 250.
        assert_eq!(total(&phase.diff(), Event::CpuCycles), 250);
    }

    #[test]
    fn diff_per_cpu_wraps_independently_on_overflow() {
        let phase = Phase {
            begin: Snapshot {
                metrics: HashMap::from([(
                    Event::CpuCycles,
                    HashMap::from([(0, u64::MAX - 5), (1, 500)]),
                )]),
            },
            end: Snapshot {
                metrics: HashMap::from([(Event::CpuCycles, HashMap::from([(0, 10), (1, 600)]))]),
            },
        };
        // cpu 0 wraps: 16; cpu 1: 100; total = 116.
        assert_eq!(total(&phase.diff(), Event::CpuCycles), 116);
    }
}
