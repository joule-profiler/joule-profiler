use std::fmt::Display;

use serde::{Serialize, Serializer};

use crate::unit::MetricUnit;

/// Represents a single measurable metric collected from a source.
///
/// # Examples
///
/// ```
/// use joule_profiler_core::{types::Metric, unit::{MetricUnit, Unit, UnitPrefix}};
///
/// let unit = MetricUnit { unit: Unit::Joule, prefix: UnitPrefix::Micro };
/// let energy = Metric::new("energy_pkg", 123456u64, unit, "rapl");
/// ```
#[derive(Debug, Serialize, Clone)]
pub struct Metric {
    /// The metric name, (e.g. `energy_pkg`).
    pub name: String,

    /// The numeric value of the metric.
    pub value: MetricValue,

    /// The unit of measurement.
    pub unit: MetricUnit,

    /// The source providing this metric (e.g. rapl).
    pub source: String,
}

impl Metric {
    pub fn new<N, V, S>(name: N, value: V, unit: MetricUnit, source: S) -> Self
    where
        N: Into<String>,
        V: Into<MetricValue>,
        S: Into<String>,
    {
        Self {
            name: name.into(),
            value: value.into(),
            unit,
            source: source.into(),
        }
    }
}

/// A collection of metrics.
pub type Metrics = Vec<Metric>;

/// Enum representing the value of a metric,
/// with this enum, a metric can be a signed or
/// unsigned integer or a float.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MetricValue {
    UnsignedInteger(u64),
    SignedInteger(i64),
    Float(f64, Option<u8>),
}

impl From<u64> for MetricValue {
    fn from(v: u64) -> Self {
        Self::UnsignedInteger(v)
    }
}
impl From<i64> for MetricValue {
    fn from(v: i64) -> Self {
        Self::SignedInteger(v)
    }
}
impl From<f64> for MetricValue {
    fn from(v: f64) -> Self {
        Self::Float(v, None)
    }
}

impl Display for MetricValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnsignedInteger(v) => v.fmt(f),
            Self::SignedInteger(v) => v.fmt(f),
            Self::Float(v, None) => v.fmt(f),
            Self::Float(v, Some(precision)) => {
                write!(f, "{:.prec$}", v, prec = *precision as usize)
            }
        }
    }
}

impl Serialize for MetricValue {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            MetricValue::UnsignedInteger(v) => serializer.serialize_u64(*v),
            MetricValue::SignedInteger(v) => serializer.serialize_i64(*v),
            MetricValue::Float(v, _) => serializer.serialize_f64(*v),
        }
    }
}
