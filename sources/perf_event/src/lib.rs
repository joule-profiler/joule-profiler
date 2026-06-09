//! `perf_event` source for hardware performance counters.
//!
//! Measures CPU cycles, instructions, cache misses, and branch misses
//! using Linux `perf_event` subsystem.
//!
//! Note: Counters are created individually (not grouped) because
//! `inherit(true)` is incompatible with `perf_event` groups on Linux.

use joule_profiler_core::{
    sensor::{Sensor, Sensors},
    source::MetricReader,
    types::{Metric, Metrics},
    unit::{MetricUnit, Unit, UnitPrefix},
};
use log::{debug, info, trace};

use crate::{
    config::PerfConfig,
    error::PerfEventError,
    event::{EVENTS, Event},
    hardware::{PerfEventCounters, PerfEventHardware},
    snapshot::{Phase, Snapshot},
};

pub mod config;
mod error;
mod event;
mod hardware;
mod snapshot;

type Result<T> = std::result::Result<T, PerfEventError>;

const PERF_EVENT_METRIC_UNIT: MetricUnit = MetricUnit {
    prefix: UnitPrefix::None,
    unit: Unit::Count,
};

/// Hardware performance counter source using `perf_event`.
///
/// Tracks CPU performance metrics (cycles, instructions, cache/branch misses)
/// for a specific process.
///
/// The hardware generic type is used for testing purposes, it allows to change the implementation
/// used to interact with `perf_event`. The default adapter use the `perf_event2` library.
pub struct PerfEvent<H: PerfEventHardware = PerfEventCounters> {
    hardware: H,
    events: Vec<Event>,
    begin_snapshot: Option<Snapshot>,
    last_snapshot: Option<Snapshot>,
}

impl<H: PerfEventHardware + 'static> MetricReader for PerfEvent<H> {
    type Type = Phase;
    type Error = PerfEventError;

    type Config = PerfConfig;

    fn from_config(config: PerfConfig) -> Result<Self> {
        Ok(Self {
            events: config
                .events
                .map_or(EVENTS.to_vec(), |e| e.into_iter().collect()),
            hardware: H::default(),
            begin_snapshot: None,
            last_snapshot: None,
        })
    }

    /// Initialize counters for a specific process and start monitoring.
    async fn init(&mut self, pid: i32) -> Result<()> {
        info!("Initializing perf_event source for PID {pid}");
        self.hardware.init_counters(&self.events, pid)
    }

    /// Read current counter values and compute delta since last measurement.
    async fn measure(&mut self) -> Result<()> {
        trace!("Reading perf_event counters");
        let new_snapshot = self.hardware.read_snapshot()?;
        if self.begin_snapshot.is_none() {
            self.begin_snapshot = Some(new_snapshot);
        } else {
            self.last_snapshot = Some(new_snapshot);
        }
        Ok(())
    }

    /// Retrieve and consume the last measurement snapshot.
    async fn retrieve(&mut self) -> Result<Self::Type> {
        if let Some(begin) = self.begin_snapshot.take()
            && let Some(end) = self.last_snapshot.take()
        {
            self.begin_snapshot = Some(end.clone());
            Ok(Phase { begin, end })
        } else {
            Err(PerfEventError::NotEnoughSamples)
        }
    }

    /// Returns available hardware performance counter sensors.
    fn get_sensors(&self) -> Result<Sensors> {
        trace!("Building perf_event sensor list");
        let sensors: Sensors = EVENTS
            .iter()
            .map(|event| {
                trace!("Registering sensor: {event}");
                Sensor::new(*event, PERF_EVENT_METRIC_UNIT, Self::get_name())
            })
            .collect();

        debug!("Registered {} perf_event sensors", sensors.len());
        Ok(sensors)
    }

    /// Convert raw counter values to metrics with metadata.
    fn to_metrics(&self, result: Self::Type) -> Result<Metrics> {
        trace!(
            "Converting {} counters to metrics",
            result.begin.metrics.len()
        );
        let diff = result.diff();
        Ok(diff
            .metrics
            .into_iter()
            .map(|(event, counter)| {
                Metric::new(event, counter, PERF_EVENT_METRIC_UNIT, Self::get_name())
            })
            .collect())
    }

    fn get_name() -> &'static str {
        "perf_event"
    }

    fn get_id() -> &'static str {
        "perf_event"
    }
}

#[cfg(test)]
mod tests {
    use joule_profiler_core::types::MetricValue;

    use super::*;
    use crate::{event::Event, hardware::MockPerfEventHardware, snapshot::Snapshot};

    fn snapshot(entries: Vec<(Event, u64)>) -> Snapshot {
        Snapshot {
            metrics: entries.into_iter().collect(),
        }
    }

    fn with_hardware(hardware: MockPerfEventHardware) -> PerfEvent<MockPerfEventHardware> {
        PerfEvent {
            hardware,
            events: EVENTS.to_vec(),
            begin_snapshot: None,
            last_snapshot: None,
        }
    }

    #[tokio::test]
    async fn measure_stores_begin_snapshot() {
        let mut hardware = MockPerfEventHardware::new();
        hardware
            .expect_read_snapshot()
            .returning(|| Ok(snapshot(vec![(Event::CpuCycles, 100)])));

        let mut source = with_hardware(hardware);
        source.measure().await.unwrap();

        assert!(source.begin_snapshot.is_some());
        assert!(source.last_snapshot.is_none());
    }

    #[tokio::test]
    async fn measure_twice_stores_last_snapshot() {
        let mut hardware = MockPerfEventHardware::new();
        let mut read_snapshot_call_count = 0u64;
        hardware.expect_read_snapshot().returning(move || {
            read_snapshot_call_count += 1;
            Ok(snapshot(vec![(
                Event::CpuCycles,
                read_snapshot_call_count * 100,
            )]))
        });

        let mut source = with_hardware(hardware);
        source.measure().await.unwrap();
        source.measure().await.unwrap();

        assert!(source.begin_snapshot.is_some());
        assert!(source.last_snapshot.is_some());
    }

    #[tokio::test]
    async fn retrieve_without_enough_snapshots_returns_error() {
        let mut hardware = MockPerfEventHardware::new();
        hardware
            .expect_read_snapshot()
            .returning(|| Ok(snapshot(vec![(Event::CpuCycles, 100)])));

        let mut source = with_hardware(hardware);
        source.measure().await.unwrap();

        assert!(matches!(
            source.retrieve().await,
            Err(PerfEventError::NotEnoughSamples)
        ));
    }

    #[tokio::test]
    async fn retrieve_returns_correct_phase() {
        let mut hardware = MockPerfEventHardware::new();
        let mut read_snapshot_call_count = 0u64;
        hardware.expect_read_snapshot().returning(move || {
            read_snapshot_call_count += 1;
            Ok(snapshot(vec![(
                Event::CpuCycles,
                read_snapshot_call_count * 100,
            )]))
        });

        let mut source = with_hardware(hardware);
        source.measure().await.unwrap();
        source.measure().await.unwrap();
        let phase = source.retrieve().await.unwrap();

        assert_eq!(phase.begin.metrics[&Event::CpuCycles], 100);
        assert_eq!(phase.end.metrics[&Event::CpuCycles], 200);
    }

    #[tokio::test]
    async fn retrieve_rolls_begin_snapshot_to_end() {
        let mut hardware = MockPerfEventHardware::new();
        let mut read_snapshot_call_count = 0u64;
        hardware.expect_read_snapshot().returning(move || {
            read_snapshot_call_count += 1;
            Ok(snapshot(vec![(
                Event::CpuCycles,
                read_snapshot_call_count * 100,
            )]))
        });

        let mut source = with_hardware(hardware);
        source.measure().await.unwrap();
        source.measure().await.unwrap();
        source.retrieve().await.unwrap();
        assert_eq!(
            source.begin_snapshot.as_ref().unwrap().metrics[&Event::CpuCycles],
            200
        );
        assert!(source.last_snapshot.is_none());
    }

    #[tokio::test]
    async fn to_metrics_returns_correct_values() {
        let mut hardware = MockPerfEventHardware::new();
        let mut read_snapshot_call_count = 0;
        hardware.expect_read_snapshot().returning(move || {
            read_snapshot_call_count += 1;
            Ok(match read_snapshot_call_count {
                1 => snapshot(vec![(Event::CpuCycles, 0)]),
                _ => snapshot(vec![(Event::CpuCycles, 500)]),
            })
        });

        let mut source = with_hardware(hardware);
        source.measure().await.unwrap();
        source.measure().await.unwrap();
        let phase = source.retrieve().await.unwrap();
        let metrics = source.to_metrics(phase).unwrap();
        let cycles = metrics
            .iter()
            .find(|m| m.name == Event::CpuCycles.to_string())
            .unwrap();

        assert_eq!(cycles.value, MetricValue::UnsignedInteger(500));
        assert_eq!(cycles.unit, PERF_EVENT_METRIC_UNIT);
    }
}
