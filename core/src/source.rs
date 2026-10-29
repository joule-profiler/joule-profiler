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

/// A sensor and its processor, type-erased, with the info of the source.
pub struct Source {
    inner: Box<dyn AnySource>,
    info: Info,
}

impl Source {
    pub fn new<S: Sensor, P: Processor<S>>(sensor: S, processor: P) -> Self {
        Self {
            inner: Box::new((sensor, processor)),
            info: Info::default(),
        }
    }

    /// Sets the info of the source: its setup and the hardware it found.
    pub fn with_info(mut self, info: Info) -> Self {
        self.info = info;
        self
    }

    pub fn name(&self) -> &str {
        self.inner.name()
    }

    pub fn metrics(&self) -> Vec<MetricInfo> {
        self.inner.metrics()
    }

    pub fn info(&self) -> &Info {
        &self.info
    }

    pub(crate) fn init(&mut self) -> Result<()> {
        self.inner.init()
    }

    pub(crate) fn attach(&mut self, target: Target) -> Result<()> {
        self.inner.attach(target)
    }

    pub(crate) fn start<'scope>(
        &'scope mut self,
        scope: &'scope Scope<'scope, '_>,
        trigger: &'scope Trigger,
        abort: &'scope AbortHandle<'scope>,
    ) -> Result<(Starting<'scope, ()>, Process<'scope>)> {
        self.inner.start(scope, trigger, abort)
    }

    pub(crate) fn close(&mut self) -> Result<()> {
        self.inner.close()
    }
}

/// Processes the phase between the next two snapshots, the first being that of the start of the
/// run. `false` once the sensor has stopped.
pub(crate) type Process<'a> = Box<dyn FnMut(&mut Vec<MetricValue>) -> Result<bool> + Send + 'a>;

trait AnySource: Send {
    fn name(&self) -> &str;
    fn metrics(&self) -> Vec<MetricInfo>;
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

    fn init(&mut self) -> Result<()> {
        self.0
            .init()
            .map_err(|error| Error::source_failed(self.0.name(), error))
    }

    fn attach(&mut self, target: Target) -> Result<()> {
        self.0
            .attach(target)
            .map_err(|error| Error::source_failed(self.0.name(), error))
    }

    fn start<'scope>(
        &'scope mut self,
        scope: &'scope Scope<'scope, '_>,
        trigger: &'scope Trigger,
        abort: &'scope AbortHandle<'scope>,
    ) -> Result<(Starting<'scope, ()>, Process<'scope>)> {
        let (sensor, processor) = self;
        let name = sensor.name().to_owned();
        let (sender, receiver) = mpsc::channel();

        let sensor_handle = abort.spawn(scope, format!("source-{name}"), move |ready| {
            measure(sensor, &sender, trigger, ready)
        })?;

        let declared = processor.metrics().len();
        let mut snapshots = receiver.into_iter();
        let mut previous = None;
        let process = move |values: &mut Vec<MetricValue>| {
            let Some(start) = previous.take().or_else(|| snapshots.next()) else {
                return Ok(false);
            };
            let Some(end) = snapshots.next() else {
                return Ok(false);
            };
            processor
                .process(&start, &end, values)
                .map_err(|error| Error::source_failed(&name, error))?;
            if values.len() != declared {
                return Err(Error::MetricCount {
                    name: name.clone(),
                    declared,
                    produced: values.len(),
                });
            }
            previous = Some(end);
            Ok(true)
        };

        Ok((sensor_handle, Box::new(process)))
    }

    fn close(&mut self) -> Result<()> {
        self.0
            .close()
            .map_err(|error| Error::source_failed(self.0.name(), error))
    }
}

/// Measures at every boundary, the start of the run included, until the run is over.
fn measure<S: Sensor>(
    sensor: &mut S,
    snapshots: &Sender<S::Snapshot>,
    trigger: &Trigger,
    ready: Sender<()>,
) -> Result<()> {
    let mut waiter = trigger.waiter();
    ready
        .send(())
        .map_err(|error| Error::source_failed(sensor.name(), error))?;
    drop(ready);

    while waiter.wait() {
        let snapshot = sensor
            .measure()
            .map_err(|error| Error::source_failed(sensor.name(), error))?;
        if snapshots.send(snapshot).is_err() {
            break;
        }
    }
    Ok(())
}
