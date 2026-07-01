// //! `perf_event` source for hardware performance counters.
// //!
// //! Measures CPU cycles, instructions, cache misses, and branch misses
// //! using Linux `perf_event` subsystem.
// //!
// //! Note: Counters are created individually (not grouped) because
// //! `inherit(true)` is incompatible with `perf_event` groups on Linux.

// use joule_profiler_core::{
//     sensor::{Sensor, Sensors},
//     source::MetricReader,
//     types::{Metric, Metrics},
//     unit::{MetricUnit, Unit, UnitPrefix},
// };
// use log::{debug, info, trace};

// use crate::{
//     error::PerfEventError,
//     event::EVENTS,
//     hardware::{PerfEventCounters, PerfEventHardware},
//     snapshot::{Phase, Snapshot},
// };

// mod error;
// mod event;
// mod hardware;
// mod snapshot;

// type Result<T> = std::result::Result<T, PerfEventError>;

const PERF_EVENT_METRIC_UNIT: MetricUnit = MetricUnit {
    prefix: UnitPrefix::None,
    unit: Unit::Count,
};

// /// Hardware performance counter source using `perf_event`.
// ///
// /// Tracks CPU performance metrics (cycles, instructions, cache/branch misses)
// /// for a specific process.
// ///
// /// The hardware generic type is used for testing purposes, it allows to change the implementation
// /// used to interact with `perf_event`. The default adapter use the `perf_event2` library.
// pub struct PerfEvent<H: PerfEventHardware = PerfEventCounters> {
//     hardware: H,
//     begin_snapshot: Option<Snapshot>,
//     last_snapshot: Option<Snapshot>,
// }

// impl PerfEvent {
//     /// Creates a new uninitialized `perf_event` source with the `perf_event2` backend.
//     pub fn new() -> Result<Self> {
//         debug!("Creating new perf_event source");
//         Ok(Self {
//             hardware: PerfEventCounters::new(),
//             begin_snapshot: None,
//             last_snapshot: None,
//         })
//     }
// }

// impl<H: PerfEventHardware + 'static> MetricReader for PerfEvent<H> {
//     type Type = Phase;
//     type Error = PerfEventError;

//     /// Initialize counters for a specific process and start monitoring.
//     async fn init(&mut self, pid: i32) -> Result<()> {
//         info!("Initializing perf_event source for PID {pid}");
//         self.hardware.init_counters(pid)
//     }

//     /// Read current counter values and compute delta since last measurement.
//     async fn measure(&mut self) -> Result<()> {
//         trace!("Reading perf_event counters");
//         let new_snapshot = self.hardware.read_snapshot()?;
//         if self.begin_snapshot.is_none() {
//             self.begin_snapshot = Some(new_snapshot);
//         } else {
//             self.last_snapshot = Some(new_snapshot);
//         }
//         Ok(())
//     }

//     /// Retrieve and consume the last measurement snapshot.
//     async fn retrieve(&mut self) -> Result<Self::Type> {
//         if let Some(begin) = self.begin_snapshot.take()
//             && let Some(end) = self.last_snapshot.take()
//         {
//             self.begin_snapshot = Some(end.clone());
//             Ok(Phase { begin, end })
//         } else {
//             Err(PerfEventError::NotEnoughSamples)
//         }
//     }

//     /// Returns available hardware performance counter sensors.
//     fn get_sensors(&self) -> Result<Sensors> {
//         trace!("Building perf_event sensor list");
//         let sensors: Sensors = EVENTS
//             .iter()
//             .map(|event| {
//                 trace!("Registering sensor: {event}");
//                 Sensor::new(*event, PERF_EVENT_METRIC_UNIT, Self::get_name())
//             })
//             .collect();

//         debug!("Registered {} perf_event sensors", sensors.len());
//         Ok(sensors)
//     }

//     /// Convert raw counter values to metrics with metadata.
//     fn to_metrics(&self, result: Self::Type) -> Result<Metrics> {
//         trace!(
//             "Converting {} counters to metrics",
//             result.begin.metrics.len()
//         );
//         let diff = result.diff();
//         Ok(diff
//             .metrics
//             .into_iter()
//             .map(|(event, counter)| {
//                 Metric::new(event, counter, PERF_EVENT_METRIC_UNIT, Self::get_name())
//             })
//             .collect())
//     }

//     fn get_name() -> &'static str {
//         "perf_event"
//     }
// }

// #[cfg(test)]
// mod tests {
//     use joule_profiler_core::types::MetricValue;

//     use super::*;
//     use crate::{event::Event, hardware::MockPerfEventHardware, snapshot::Snapshot};

//     fn snapshot(entries: Vec<(Event, u64)>) -> Snapshot {
//         Snapshot {
//             metrics: entries.into_iter().collect(),
//         }
//     }

//     fn nvml_with_hardware(hardware: MockPerfEventHardware) -> PerfEvent<MockPerfEventHardware> {
//         PerfEvent {
//             hardware,
//             begin_snapshot: None,
//             last_snapshot: None,
//         }
//     }

//     #[tokio::test]
//     async fn measure_stores_begin_snapshot() {
//         let mut hardware = MockPerfEventHardware::new();
//         hardware
//             .expect_read_snapshot()
//             .returning(|| Ok(snapshot(vec![(Event::CpuCycles, 100)])));

//         let mut source = nvml_with_hardware(hardware);
//         source.measure().await.unwrap();

//         assert!(source.begin_snapshot.is_some());
//         assert!(source.last_snapshot.is_none());
//     }

//     #[tokio::test]
//     async fn measure_twice_stores_last_snapshot() {
//         let mut hardware = MockPerfEventHardware::new();
//         let mut read_snapshot_call_count = 0u64;
//         hardware.expect_read_snapshot().returning(move || {
//             read_snapshot_call_count += 1;
//             Ok(snapshot(vec![(
//                 Event::CpuCycles,
//                 read_snapshot_call_count * 100,
//             )]))
//         });

//         let mut source = nvml_with_hardware(hardware);
//         source.measure().await.unwrap();
//         source.measure().await.unwrap();

//         assert!(source.begin_snapshot.is_some());
//         assert!(source.last_snapshot.is_some());
//     }

//     #[tokio::test]
//     async fn retrieve_without_enough_snapshots_returns_error() {
//         let mut hardware = MockPerfEventHardware::new();
//         hardware
//             .expect_read_snapshot()
//             .returning(|| Ok(snapshot(vec![(Event::CpuCycles, 100)])));

//         let mut source = nvml_with_hardware(hardware);
//         source.measure().await.unwrap();

//         assert!(matches!(
//             source.retrieve().await,
//             Err(PerfEventError::NotEnoughSamples)
//         ));
//     }

//     #[tokio::test]
//     async fn retrieve_returns_correct_phase() {
//         let mut hardware = MockPerfEventHardware::new();
//         let mut read_snapshot_call_count = 0u64;
//         hardware.expect_read_snapshot().returning(move || {
//             read_snapshot_call_count += 1;
//             Ok(snapshot(vec![(
//                 Event::CpuCycles,
//                 read_snapshot_call_count * 100,
//             )]))
//         });

//         let mut source = nvml_with_hardware(hardware);
//         source.measure().await.unwrap();
//         source.measure().await.unwrap();
//         let phase = source.retrieve().await.unwrap();

//         assert_eq!(phase.begin.metrics[&Event::CpuCycles], 100);
//         assert_eq!(phase.end.metrics[&Event::CpuCycles], 200);
//     }

//     #[tokio::test]
//     async fn retrieve_rolls_begin_snapshot_to_end() {
//         let mut hardware = MockPerfEventHardware::new();
//         let mut read_snapshot_call_count = 0u64;
//         hardware.expect_read_snapshot().returning(move || {
//             read_snapshot_call_count += 1;
//             Ok(snapshot(vec![(
//                 Event::CpuCycles,
//                 read_snapshot_call_count * 100,
//             )]))
//         });

//         let mut source = nvml_with_hardware(hardware);
//         source.measure().await.unwrap();
//         source.measure().await.unwrap();
//         source.retrieve().await.unwrap();
//         assert_eq!(
//             source.begin_snapshot.as_ref().unwrap().metrics[&Event::CpuCycles],
//             200
//         );
//         assert!(source.last_snapshot.is_none());
//     }

//     #[tokio::test]
//     async fn to_metrics_returns_correct_values() {
//         let mut hardware = MockPerfEventHardware::new();
//         let mut read_snapshot_call_count = 0;
//         hardware.expect_read_snapshot().returning(move || {
//             read_snapshot_call_count += 1;
//             Ok(match read_snapshot_call_count {
//                 1 => snapshot(vec![(Event::CpuCycles, 0)]),
//                 _ => snapshot(vec![(Event::CpuCycles, 500)]),
//             })
//         });

//         let mut source = nvml_with_hardware(hardware);
//         source.measure().await.unwrap();
//         source.measure().await.unwrap();
//         let phase = source.retrieve().await.unwrap();
//         let metrics = source.to_metrics(phase).unwrap();
//         let cycles = metrics
//             .iter()
//             .find(|m| m.name == Event::CpuCycles.to_string())
//             .unwrap();

//         assert_eq!(cycles.value, MetricValue::UnsignedInteger(500));
//         assert_eq!(cycles.unit, PERF_EVENT_METRIC_UNIT);
//     }
// }

use std::collections::HashMap;
use std::fmt::Display;

use joule_profiler_core::source::{MetricSource, Processor, Sensor};
use joule_profiler_core::time::get_timestamp_micros;
use joule_profiler_core::types::{AvailableSensor, InitContext, Metric, Metrics, Sensors};
use joule_profiler_core::unit::{MetricUnit, Unit, UnitPrefix};
use perf_event::events::Hardware;
use perf_event::{Builder, Counter};
use thiserror::Error;
use tokio::task::{self, JoinError};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Event {
    CpuCycles,
    Instructions,
    CacheMisses,
    BranchMisses,
}

pub static EVENTS: &[Event] = &[
    Event::CpuCycles,
    Event::Instructions,
    Event::CacheMisses,
    Event::BranchMisses,
];

impl From<Event> for Hardware {
    fn from(event: Event) -> Self {
        match event {
            Event::CpuCycles => Hardware::CPU_CYCLES,
            Event::Instructions => Hardware::INSTRUCTIONS,
            Event::CacheMisses => Hardware::CACHE_MISSES,
            Event::BranchMisses => Hardware::BRANCH_MISSES,
        }
    }
}

impl Display for Event {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Event::CpuCycles => "CPU_CYCLES",
            Event::Instructions => "INSTRUCTIONS",
            Event::CacheMisses => "CACHE_MISSES",
            Event::BranchMisses => "BRANCH_MISSES",
        })
    }
}

pub struct Snapshot {
    pub timestamp: u128,
    pub counts: HashMap<Event, u64>,
}

pub struct PerfEventSensor {
    counters: HashMap<Event, Counter>,
}

fn open_counters(pid: i32) -> Result<HashMap<Event, Counter>, PerfEventError> {
    EVENTS
        .iter()
        .map(|event| {
            let counter = Builder::new(Hardware::from(*event))
                .observe_pid(pid)
                .any_cpu()
                .include_hv()
                .include_kernel()
                .inherit(true)
                .enabled(true)
                .build()
                .map_err(|e| PerfEventError::Open(*event, e))?;
            Ok((*event, counter))
        })
        .collect()
}

#[derive(Debug, Error)]
pub enum PerfEventError {
    #[error("failed to open {0} counter")]
    Open(Event, #[source] std::io::Error),

    #[error("failed to read {0} counter")]
    Read(Event, #[source] std::io::Error),

    #[error("enabling the perf group: {0}")]
    Enable(#[source] std::io::Error),

    #[error("failed to join tokio task")]
    JoinError(
        #[from]
        #[source]
        JoinError,
    ),
}

impl Sensor<PerfEventSource> for PerfEventSensor {
    async fn init(&mut self, ctx: InitContext) -> Result<(), PerfEventError> {
        self.counters = task::spawn_blocking(move || open_counters(ctx.pid)).await??;
        Ok(())
    }

    async fn measure(&mut self) -> Result<Snapshot, PerfEventError> {
        let timestamp = get_timestamp_micros();

        let mut counts = HashMap::with_capacity(self.counters.len());
        for (event, counter) in &mut self.counters {
            let value = counter
                .read()
                .map_err(|e| PerfEventError::Read(*event, e))?;
            counts.insert(*event, value);
        }
        Ok(Snapshot { timestamp, counts })
    }

    async fn join(&mut self) -> Result<(), PerfEventError> {
        for counter in self.counters.values_mut() {
            counter.disable().map_err(PerfEventError::Enable)?;
        }
        self.counters.clear();
        Ok(())
    }

    fn list_sensors(&self) -> Result<Sensors, PerfEventError> {
        Ok(EVENTS
            .iter()
            .map(|event| {
                AvailableSensor::new(
                    event.to_string(),
                    PERF_EVENT_METRIC_UNIT,
                    PerfEventSource::get_name(),
                )
            })
            .collect())
    }
}

#[derive(Default)]
pub struct PerfEventProcessor {
    baseline: Option<Snapshot>,
}

impl Processor<PerfEventSource> for PerfEventProcessor {
    async fn consume(&mut self, snapshot: Snapshot) -> Result<Option<Metrics>, PerfEventError> {
        let metrics = self.baseline.as_ref().map(|baseline| {
            snapshot
                .counts
                .iter()
                .map(|(event, &value)| {
                    let delta = baseline
                        .counts
                        .get(event)
                        .map_or(value, |&prev| value.wrapping_sub(prev));
                    Metric::new(
                        event.to_string(),
                        delta,
                        PerfEventSource::get_name(),
                        baseline.timestamp,
                    )
                })
                .collect()
        });

        self.baseline = Some(snapshot);
        Ok(metrics)
    }
}

pub struct PerfEventSource;

impl MetricSource for PerfEventSource {
    type Sensor = PerfEventSensor;
    type Processor = PerfEventProcessor;
    type Snapshot = Snapshot;
    type Error = PerfEventError;
    type Config = ();

    fn from_config(_config: ()) -> Result<(PerfEventSensor, PerfEventProcessor), PerfEventError> {
        Ok((
            PerfEventSensor {
                counters: HashMap::new(),
            },
            PerfEventProcessor::default(),
        ))
    }

    fn get_name() -> &'static str {
        "perf_event"
    }
}
