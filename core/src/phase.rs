use serde::{Deserialize, Serialize};

use crate::metric::{Metric, MetricInfo, MetricValue};
use crate::schema::Schema;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PhaseInfo {
    pub index: usize,
    pub start_token: String,
    pub end_token: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_line: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_line: Option<usize>,
    pub timestamp_us: u128,
    pub duration_ms: u64,
}

impl PhaseInfo {
    pub fn name(&self) -> String {
        format!("{} -> {}", self.start_token, self.end_token)
    }
}

/// The start or end of a phase.
pub(crate) struct PhaseBoundary {
    pub token: String,
    pub line: Option<usize>,
    pub timestamp_us: u128,
}

/// The metrics of one source for a phase. The names are borrowed from the schema.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SourceMetrics<'a> {
    pub name: &'a str,
    pub metrics: Vec<Metric<'a>>,
}

impl<'a> SourceMetrics<'a> {
    /// Blank metrics for every source, reused for every phase.
    pub(crate) fn of_schema(schema: &'a Schema) -> Vec<Self> {
        let metric = |info: &'a MetricInfo| Metric {
            name: &info.name,
            value: MetricValue::U64(0),
            unit: info.unit,
        };
        schema
            .sources
            .iter()
            .map(|source| Self {
                name: &source.name,
                metrics: source.metrics.iter().map(metric).collect(),
            })
            .collect()
    }

    /// Writes the values of a phase.
    pub(crate) fn fill_values(sources: &mut [Self], values: &[Vec<MetricValue>]) {
        for (source, values) in sources.iter_mut().zip(values) {
            for (metric, value) in source.metrics.iter_mut().zip(values) {
                metric.value = *value;
            }
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Summary {
    pub timestamp_us: u128,
    pub duration_ms: u64,
    pub phases: usize,
    pub exit_code: Option<i32>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::SourceInfo;
    use crate::unit::MetricUnit;

    #[test]
    fn the_metrics_of_a_run_are_named_once_and_take_the_values_of_each_phase() {
        let schema = Schema {
            sources: vec![
                SourceInfo {
                    name: "rapl".into(),
                    metrics: vec![
                        MetricInfo::new("PACKAGE-0", MetricUnit::JOULE),
                        MetricInfo::new("DRAM-0", MetricUnit::JOULE),
                    ],
                },
                SourceInfo {
                    name: "perf".into(),
                    metrics: vec![MetricInfo::new("INSTRUCTIONS", MetricUnit::COUNT)],
                },
            ],
        };
        let mut sources = SourceMetrics::of_schema(&schema);

        for phase in 1..=2_u64 {
            let values = [
                vec![MetricValue::F64(1.5), MetricValue::F64(0.5)],
                vec![MetricValue::U64(42 * phase)],
            ];
            SourceMetrics::fill_values(&mut sources, &values);

            assert_eq!(sources[0].name, "rapl");
            assert_eq!(
                sources[0].metrics[1],
                Metric {
                    name: "DRAM-0",
                    value: MetricValue::F64(0.5),
                    unit: MetricUnit::JOULE,
                }
            );
            assert_eq!(sources[1].metrics[0].value, MetricValue::U64(42 * phase));
        }
    }
}
