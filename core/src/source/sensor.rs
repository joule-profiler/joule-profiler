// //! Representation of measurable sensors.
// //!
// //! This module defines the structures used to describe sensors collected
// //! by `JouleProfiler`. Sensors are associated with metric sources and are
// //! used to represent individual measurements.

// use serde::Serialize;

// use crate::unit::MetricUnit;

// /// Represents a measurable sensor.
// ///
// /// A sensor corresponds to a metric collected from a source.
// ///
// /// # Examples
// ///
// /// ```no_run
// /// use joule_profiler_core::{
// ///     sensor::Sensor,
// ///     unit::{MetricUnit, UnitPrefix, Unit},
// /// };
// ///
// /// let micro_joule_unit: MetricUnit = MetricUnit {
// ///     prefix: UnitPrefix::Micro,
// ///     unit: Unit::Joule,
// /// };
// ///
// /// let sensor = Sensor::new("CORE-0", micro_joule_unit, "powercap");
// ///
// /// assert_eq!(sensor.name, "CORE-0");
// /// assert_eq!(sensor.unit.to_string(), "µJ");
// /// assert_eq!(sensor.source, "powercap");
// /// ```
// #[derive(Debug, Serialize, Clone, PartialEq, Eq)]
// pub struct Sensor {
//     /// The name of the sensor.
//     pub name: String,

//     /// The standard international unit associated to this sensor.
//     pub unit: MetricUnit,

//     /// The metric source associated to the sensor.
//     pub source: String,
// }

// impl Sensor {
//     pub fn new<N, S>(name: N, unit: MetricUnit, source: S) -> Self
//     where
//         N: Into<String>,
//         S: Into<String>,
//     {
//         Self {
//             name: name.into(),
//             unit,
//             source: source.into(),
//         }
//     }
// }

// /// A collection of sensors.
// pub type Sensors = Vec<Sensor>;

use crate::{
    source::MetricSource,
    types::{InitContext, Sensors},
};

pub trait Sensor<S: MetricSource>: Send {
    fn pre_init(&mut self) -> impl Future<Output = Result<(), S::Error>> + Send {
        async { Ok(()) }
    }

    fn init(&mut self, _ctx: InitContext) -> impl Future<Output = Result<(), S::Error>> + Send {
        async { Ok(()) }
    }

    fn measure(&mut self) -> impl Future<Output = Result<S::Snapshot, S::Error>> + Send;

    fn join(&mut self) -> impl Future<Output = Result<(), S::Error>> + Send {
        async { Ok(()) }
    }

    fn clean(&mut self) -> impl Future<Output = Result<(), S::Error>> + Send {
        async { Ok(()) }
    }

    fn list_sensors(&self) -> Result<Sensors, S::Error>;
}
