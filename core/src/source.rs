use std::sync::mpsc::{self, Sender};
use std::thread::Scope;

use crate::error::{Error, Result};
use crate::info::Info;
use crate::injector::Target;
use crate::metric::{MetricInfo, MetricValue};
use crate::processor::Processor;
use crate::profiler::{AbortHandle, Starting};
use crate::sensor::Sensor;
use crate::trigger::Trigger;

/// A sensor and its processor, type-erased.
pub struct Source(Box<dyn AnySource>);

impl Source {
    pub fn new<S: Sensor, P: Processor<S>>(sensor: S, processor: P) -> Self {
        Self(Box::new((sensor, processor)))
    }

    pub fn name(&self) -> &str {
        self.0.name()
    }

    pub fn metrics(&self) -> Vec<MetricInfo> {
        self.0.metrics()
    }

    /// The info of its processor.
    pub fn info(&self) -> Info {
        self.0.info()
    }

    pub(crate) fn init(&mut self) -> Result<()> {
        self.0.init()
    }

    pub(crate) fn attach(&mut self, target: Target) -> Result<()> {
        self.0.attach(target)
    }

    pub(crate) fn start<'scope>(
        &'scope mut self,
        scope: &'scope Scope<'scope, '_>,
        trigger: &'scope Trigger,
        abort: &'scope AbortHandle<'scope>,
    ) -> Result<(Starting<'scope, ()>, Process<'scope>)> {
        self.0.start(scope, trigger, abort)
    }

    pub(crate) fn close(&mut self) -> Result<()> {
        self.0.close()
    }
}

/// Processes the phase ended by the next snapshot. `false` once the sensor has stopped.
pub(crate) type Process<'a> = Box<dyn FnMut(&mut Vec<MetricValue>) -> Result<bool> + Send + 'a>;

trait AnySource: Send {
    fn name(&self) -> &str;
    fn metrics(&self) -> Vec<MetricInfo>;
    fn info(&self) -> Info;
    fn init(&mut self) -> Result<()>;
    fn attach(&mut self, target: Target) -> Result<()>;
    fn start<'scope>(
        &'scope mut self,
        scope: &'scope Scope<'scope, '_>,
        trigger: &'scope Trigger,
        abort: &'scope AbortHandle<'scope>,
    ) -> Result<(Starting<'scope, ()>, Process<'scope>)>;
    fn close(&mut self) -> Result<()>;
}

impl<S: Sensor, P: Processor<S>> AnySource for (S, P) {
    fn name(&self) -> &str {
        self.0.name()
    }

    fn metrics(&self) -> Vec<MetricInfo> {
        self.1.metrics()
    }

    fn info(&self) -> Info {
        self.1.info()
    }

    fn init(&mut self) -> Result<()> {
        self.0.init().map_err(|error| failed(self.0.name(), error))
    }

    fn attach(&mut self, target: Target) -> Result<()> {
        self.0
            .attach(target)
            .map_err(|error| failed(self.0.name(), error))
    }

    fn start<'scope>(
        &'scope mut self,
        scope: &'scope Scope<'scope, '_>,
        trigger: &'scope Trigger,
        abort: &'scope AbortHandle<'scope>,
    ) -> Result<(Starting<'scope, ()>, Process<'scope>)> {
        let (sensor, processor) = self;
        let name = sensor.name().to_owned();
        let mut previous = sensor.measure().map_err(|error| failed(&name, error))?;
        let (sender, receiver) = mpsc::channel();

        let sensor_handle = abort.spawn(scope, format!("source-{name}"), move |ready| {
            measure(sensor, &sender, trigger, ready)
        })?;

        let declared = processor.metrics().len();
        let process = move |values: &mut Vec<MetricValue>| {
            let Ok(current) = receiver.recv() else {
                return Ok(false);
            };
            processor
                .process(&previous, &current, values)
                .map_err(|error| failed(&name, error))?;
            if values.len() != declared {
                return Err(Error::MetricCount {
                    name: name.clone(),
                    declared,
                    produced: values.len(),
                });
            }
            previous = current;
            Ok(true)
        };

        Ok((sensor_handle, Box::new(process)))
    }

    fn close(&mut self) -> Result<()> {
        self.0.close().map_err(|error| failed(self.0.name(), error))
    }
}

/// Measures at every boundary until the run is over.
fn measure<S: Sensor>(
    sensor: &mut S,
    snapshots: &Sender<S::Snapshot>,
    trigger: &Trigger,
    ready: Sender<()>,
) -> Result<()> {
    let mut waiter = trigger.waiter();
    ready
        .send(())
        .map_err(|error| failed(sensor.name(), error))?;
    drop(ready);

    while waiter.wait() {
        let snapshot = sensor
            .measure()
            .map_err(|error| failed(sensor.name(), error))?;
        if snapshots.send(snapshot).is_err() {
            break;
        }
    }
    Ok(())
}

fn failed(name: &str, error: impl std::error::Error + Send + Sync + 'static) -> Error {
    Error::Source {
        name: name.to_owned(),
        error: Box::new(error),
    }
}
