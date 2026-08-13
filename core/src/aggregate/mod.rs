//! Core aggregation module for `JouleProfiler`.
//!
//! Provides structures to aggregate metrics from various sources into a unified
//! format, organized by phases, and sensor-level results.
//!
//! Metrics are only instantiated *after* measurements finish to avoid runtime
//! overhead during collection.

mod metric;
pub(crate) mod phase;
pub(crate) mod sensor_result;

pub use metric::{Metric, MetricValue, Metrics};
