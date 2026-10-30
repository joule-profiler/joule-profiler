use crate::{
    metric::{MetricInfo, MetricValue},
    sensor::Sensor,
};

/// Turns two snapshots of a sensor into the metrics of the phase between them. Runs on the
/// exporter's thread, so it never delays a measurement.
pub trait Processor<S: Sensor>: Send + 'static {
    /// Returns the metrics `process` produces, in order.
    fn metrics(&self) -> Vec<MetricInfo>;

    /// Pushes one value per metric into `values`.
    fn process(
        &mut self,
        previous: &S::Snapshot,
        current: &S::Snapshot,
        values: &mut Vec<MetricValue>,
    ) -> Result<(), S::Error>;
}
