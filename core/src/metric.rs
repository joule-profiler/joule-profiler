use std::fmt::{self, Display};

use serde::{Deserialize, Serialize};

use crate::unit::MetricUnit;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum MetricValue {
    U64(u64),
    I64(i64),
    F64(f64),
}

impl From<u64> for MetricValue {
    fn from(value: u64) -> Self {
        Self::U64(value)
    }
}

impl From<u32> for MetricValue {
    fn from(value: u32) -> Self {
        Self::U64(value.into())
    }
}

impl From<i64> for MetricValue {
    fn from(value: i64) -> Self {
        Self::I64(value)
    }
}

impl From<f64> for MetricValue {
    fn from(value: f64) -> Self {
        Self::F64(value)
    }
}

impl Display for MetricValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::U64(value) => Display::fmt(value, f),
            Self::I64(value) => Display::fmt(value, f),
            Self::F64(value) => Display::fmt(value, f),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MetricInfo {
    pub name: String,
    pub unit: MetricUnit,
}

impl MetricInfo {
    pub fn new(name: impl Into<String>, unit: MetricUnit) -> Self {
        Self {
            name: name.into(),
            unit,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Metric<'a> {
    pub name: &'a str,
    pub value: MetricValue,
    pub unit: MetricUnit,
}
