use crate::info::Info;
use crate::metric::{MetricInfo, MetricValue};
use crate::sensor::Sensor;

/// Turns two snapshots of a sensor into the metrics of the phase between them. Runs on the
/// exporter's thread, so it never delays a measurement.
pub trait Processor<S: Sensor>: Send + 'static {
    /// The metrics `process` produces, in order.
    fn metrics(&self) -> Vec<MetricInfo>;

    /// The setup of the source and the hardware it found.
    fn info(&self) -> Info {
        Info::default()
    }

    /// Pushes one value per metric into `values`.
    fn process(
        &mut self,
        previous: &S::Snapshot,
        current: &S::Snapshot,
        values: &mut Vec<MetricValue>,
    ) -> Result<(), S::Error>;
}
