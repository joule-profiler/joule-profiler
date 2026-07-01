use std::fmt::Display;

use smallvec::SmallVec;

use crate::unit::MetricUnit;

pub enum MetricValue {
    I64(i64),
    U64(u64),
    F64(f64),
}

impl From<u64> for MetricValue {
    fn from(value: u64) -> Self {
        MetricValue::U64(value)
    }
}

impl From<i64> for MetricValue {
    fn from(value: i64) -> Self {
        MetricValue::I64(value)
    }
}

impl From<f64> for MetricValue {
    fn from(value: f64) -> Self {
        MetricValue::F64(value)
    }
}

impl Display for MetricValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MetricValue::I64(v) => write!(f, "{v}"),
            MetricValue::U64(v) => write!(f, "{v}"),
            MetricValue::F64(v) => {
                let precision = f.precision().unwrap_or(4);
                write!(f, "{v:.precision$}")
            }
        }
    }
}

pub struct Metric {
    pub source: &'static str,
    pub name: String,
    pub value: MetricValue,
    pub timestamp: u128,
}

pub type Metrics = SmallVec<[Metric; 16]>;

impl Metric {
    pub fn new(
        name: impl Into<String>,
        value: impl Into<MetricValue>,
        source: &'static str,
        timestamp: u128,
    ) -> Self {
        Metric {
            name: name.into(),
            value: value.into(),
            source,
            timestamp,
        }
    }
}

pub struct AvailableSensor {
    pub name: String,
    pub source: &'static str,
    pub unit: MetricUnit,
}

impl AvailableSensor {
    pub fn new(name: impl Into<String>, unit: MetricUnit, source: &'static str) -> Self {
        AvailableSensor {
            name: name.into(),
            source,
            unit,
        }
    }
}

pub type Sensors = Vec<AvailableSensor>;

pub struct Phase {
    pub phase_index: usize,
    pub metrics: Metrics,
}

pub struct PhaseInfo {
    pub phase_index: usize,
    pub start_token: String,
    pub end_token: String,
    pub start_timestamp: u128,
    pub end_timestamp: u128,
}

pub struct PhaseStartInfo {
    pub phase_index: usize,
    pub token: String,
    pub timestamp: u128,
}

#[derive(Clone, Copy)]
pub struct InitContext {
    pub pid: i32,
}
