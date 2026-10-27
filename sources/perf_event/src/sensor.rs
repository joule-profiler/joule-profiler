use std::sync::Arc;

use joule_profiler_core::{injector::Target, sensor::Sensor};

use crate::{
    counters::RawCount,
    error::{PerfEventError, Result},
    event::Event,
    scope::PerfScope,
};

/// Reads the counters of a scope.
pub(crate) struct PerfSensor<S: PerfScope> {
    events: Arc<Vec<Event>>,
    scope: S,
}

impl<S: PerfScope> PerfSensor<S> {
    pub(crate) fn new(events: Arc<Vec<Event>>, scope: S) -> Self {
        Self { events, scope }
    }
}

impl<S: PerfScope> Sensor for PerfSensor<S> {
    type Snapshot = Vec<RawCount>;

    type Error = PerfEventError;

    fn name(&self) -> &'static str {
        "perf_event"
    }

    fn init(&mut self) -> Result<()> {
        self.scope.open(&self.events)
    }

    fn attach(&mut self, target: Target) -> Result<()> {
        self.scope.attach(target, &self.events)
    }

    fn measure(&mut self) -> Result<Self::Snapshot> {
        self.scope.read(&self.events)
    }

    fn close(&mut self) -> Result<()> {
        self.scope.close()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::counters::tests::raw_count;
    use crate::scope::MockPerfScope;

    const EVENTS: [Event; 2] = [Event::CpuCycles, Event::TaskClock];

    fn mock_sensor(scope: MockPerfScope) -> PerfSensor<MockPerfScope> {
        PerfSensor::new(Arc::new(EVENTS.to_vec()), scope)
    }

    #[test]
    fn init_opens_the_counters_of_every_event_in_order() {
        let mut scope = MockPerfScope::new();
        scope
            .expect_open()
            .withf(|events| events == EVENTS)
            .times(1)
            .returning(|_| Ok(()));

        mock_sensor(scope).init().unwrap();
    }

    #[test]
    fn attach_hands_the_target_and_the_events_to_the_scope() {
        let mut scope = MockPerfScope::new();
        scope
            .expect_attach()
            .withf(|target, events| target.pid == 42 && events == EVENTS)
            .times(1)
            .returning(|_, _| Ok(()));

        mock_sensor(scope).attach(Target { pid: 42 }).unwrap();
    }

    #[test]
    fn a_measure_is_a_read_of_the_scope() {
        let mut scope = MockPerfScope::new();
        scope
            .expect_read()
            .withf(|events| events == EVENTS)
            .times(1)
            .returning(|events| Ok(vec![raw_count(10, 20, 20); events.len()]));
        let snapshot = mock_sensor(scope).measure().unwrap();

        assert_eq!(snapshot, [raw_count(10, 20, 20); 2]);
    }

    #[test]
    fn an_error_of_the_scope_is_that_of_the_sensor() {
        let mut scope = MockPerfScope::new();
        scope
            .expect_read()
            .returning(|_| Err(PerfEventError::NotOpened));

        assert!(matches!(
            mock_sensor(scope).measure(),
            Err(PerfEventError::NotOpened)
        ));
    }

    #[test]
    fn close_closes_the_scope() {
        let mut scope = MockPerfScope::new();
        scope.expect_close().times(1).returning(|| Ok(()));

        mock_sensor(scope).close().unwrap();
    }
}
