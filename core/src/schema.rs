use serde::{Deserialize, Serialize};

use crate::metric::MetricInfo;

/// The metrics one source reports, in the order of its values.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceInfo {
    pub name: String,
    pub metrics: Vec<MetricInfo>,
}

/// The metrics of every source.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Schema {
    pub sources: Vec<SourceInfo>,
}
