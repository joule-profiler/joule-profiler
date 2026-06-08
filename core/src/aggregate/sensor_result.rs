use crate::{aggregate::phase::SensorPhase, orchestrator::error::OrchestratorError};

/// Aggregated results of sensors.
#[derive(Debug)]
pub struct SensorResult {
    /// Phases collected from all metric sources.
    pub phases: Vec<SensorPhase>,
}

impl SensorResult {
    /// Merge multiple sensor results into one.
    ///
    /// Returns an error if the results are empty and ignore the sources that produced zero phases.
    pub fn merge(results: Vec<Self>) -> Result<SensorResult, OrchestratorError> {
        if results.is_empty() {
            return Err(OrchestratorError::NoSensorResults);
        }

        let mut valid_results = Vec::with_capacity(results.len());
        let mut empty_results_count = 0;

        for result in results {
            if result.phases.is_empty() {
                empty_results_count += 1;
            } else {
                valid_results.push(result);
            }
        }

        if empty_results_count > 0 {
            log::warn!(
                "SensorResult::merge: {empty_results_count} source(s) produced zero phases and were skipped."
            );
        }

        let mut iter = valid_results.into_iter();

        match iter.next() {
            None => Err(OrchestratorError::AllSourcesEmpty),
            Some(first) => iter.try_fold(first, SensorResult::try_add),
        }
    }
    /// Combine two sensor results by adding corresponding phases.
    ///
    /// Returns an error if the two results have a different number of phases,
    /// since combining misaligned phases would produce incorrect results.
    pub fn try_add(self, rhs: Self) -> Result<Self, OrchestratorError> {
        let lhs_len = self.phases.len();
        let rhs_len = rhs.phases.len();

        if lhs_len != rhs_len {
            return Err(OrchestratorError::PhaseMismatch {
                lhs: lhs_len,
                rhs: rhs_len,
            });
        }

        let phases = self
            .phases
            .into_iter()
            .zip(rhs.phases)
            .map(|(l, r)| l + r)
            .collect();

        Ok(Self { phases })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::aggregate::phase::SensorPhase;
    use crate::types::{Metric, MetricValue};
    use crate::unit::{MetricUnit, Unit, UnitPrefix};

    fn metric(value: u64) -> Metric {
        let unit = MetricUnit {
            unit: Unit::Joule,
            prefix: UnitPrefix::Micro,
        };
        Metric::new("energy_pkg", value, unit, "rapl")
    }

    fn phase(metrics: Vec<Metric>) -> SensorPhase {
        SensorPhase { metrics }
    }

    fn result(phases: Vec<SensorPhase>) -> SensorResult {
        SensorResult { phases }
    }

    #[test]
    fn merge_empty_vec_returns_error() {
        assert!(matches!(
            SensorResult::merge(vec![]),
            Err(OrchestratorError::NoSensorResults)
        ));
    }

    #[test]
    fn merge_single_result_returns_it() {
        let r = result(vec![phase(vec![metric(100)])]);
        let merged = SensorResult::merge(vec![r]).unwrap();
        assert_eq!(merged.phases.len(), 1);
        assert_eq!(
            merged.phases[0].metrics[0].value,
            MetricValue::UnsignedInteger(100)
        );
    }

    #[test]
    fn merge_multiple_results_accumulates_metrics() {
        let r1 = result(vec![phase(vec![metric(100)])]);
        let r2 = result(vec![phase(vec![metric(200)])]);
        let merged = SensorResult::merge(vec![r1, r2]).unwrap();
        assert_eq!(merged.phases[0].metrics.len(), 2);
        assert_eq!(
            merged.phases[0].metrics[0].value,
            MetricValue::UnsignedInteger(100)
        );
        assert_eq!(
            merged.phases[0].metrics[1].value,
            MetricValue::UnsignedInteger(200)
        );
    }

    #[test]
    fn merge_skips_empty_sources_and_warns() {
        let r1 = result(vec![phase(vec![metric(100)])]);
        let r2 = result(vec![]);
        let merged = SensorResult::merge(vec![r1, r2]).unwrap();
        assert_eq!(merged.phases.len(), 1);
        assert_eq!(
            merged.phases[0].metrics[0].value,
            MetricValue::UnsignedInteger(100)
        );
    }

    #[test]
    fn merge_all_empty_sources_returns_error() {
        assert!(matches!(
            SensorResult::merge(vec![result(vec![]), result(vec![])]),
            Err(OrchestratorError::AllSourcesEmpty)
        ));
    }

    #[test]
    fn merge_returns_error_on_phase_mismatch() {
        let r1 = result(vec![phase(vec![metric(100)]), phase(vec![metric(200)])]);
        let r2 = result(vec![phase(vec![metric(300)])]);
        assert!(matches!(
            SensorResult::merge(vec![r1, r2]),
            Err(OrchestratorError::PhaseMismatch { .. })
        ));
    }
}
