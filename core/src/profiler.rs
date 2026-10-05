use std::panic::{self, AssertUnwindSafe};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::thread::{self, Scope, ScopedJoinHandle};

use crate::error::{Error, Result};
use crate::exporter::{AnyExporter, Exporter};
use crate::info::{self, Info};
use crate::injector::{AnyInjector, Injector, StopHandle};
use crate::metric::MetricValue;
use crate::phase::{PhaseBoundary, PhaseInfo, SourceMetrics, Summary};
use crate::schema::{Schema, SourceInfo};
use crate::source::{Process, Source};
use crate::trigger::Trigger;
use crate::util::cgroup::RunCgroup;
use crate::util::time::millis_between;

/// The sources measure what the injector runs, and the exporter receives each phase.
#[derive(Default)]
pub struct JouleProfiler {
    sources: Vec<Source>,
    injector: Option<Box<dyn AnyInjector>>,
    exporter: Option<Box<dyn AnyExporter>>,
    defer: bool,
    cgroup: Option<RunCgroup>,
}

impl JouleProfiler {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_source(&mut self, source: Source) {
        self.sources.push(source);
    }

    pub fn set_injector(&mut self, injector: impl Injector) {
        self.injector = Some(Box::new(injector));
    }

    pub fn set_exporter(&mut self, exporter: impl Exporter) {
        self.exporter = Some(Box::new(exporter));
    }

    /// Processes and exports the phases only once the run is over.
    pub fn set_defer(&mut self, defer: bool) {
        self.defer = defer;
    }

    /// Moves the profiled program into `cgroup` once it is started.
    pub fn set_cgroup(&mut self, cgroup: RunCgroup) {
        self.cgroup = Some(cgroup);
    }

    pub fn schema(&self) -> Schema {
        Schema {
            sources: self
                .sources
                .iter()
                .map(|source| SourceInfo {
                    name: source.name().to_owned(),
                    metrics: source.metrics(),
                })
                .collect(),
        }
    }

    /// The info of the machine, then of each source, by name.
    pub fn info(&self) -> Vec<(String, Info)> {
        std::iter::once(("machine".to_owned(), info::machine()))
            .chain(
                self.sources
                    .iter()
                    .map(|source| (source.name().to_owned(), source.info())),
            )
            .collect()
    }

    pub fn profile(&mut self) -> Result<Summary> {
        if self.sources.is_empty() {
            return Err(Error::NoSource);
        }

        let schema = self.schema();
        let injector = self.injector.as_deref_mut().ok_or(Error::NoInjector)?;
        let exporter = self.exporter.as_deref_mut().ok_or(Error::NoExporter)?;
        let sources = &mut self.sources;

        let summary = sources
            .iter_mut()
            .try_for_each(Source::init)
            .and_then(|()| {
                let cgroup = self.cgroup.as_ref();
                profile_target(sources, injector, exporter, cgroup, &schema, self.defer)
            });
        // Every source is closed, even if one fails to.
        sources
            .iter_mut()
            .map(Source::close)
            .fold(Ok(()), Result::and)?;

        let summary = summary?;
        exporter.finish(&summary)?;

        Ok(summary)
    }
}

/// Starts the target, measures it until it ends, then waits for it.
fn profile_target(
    sources: &mut [Source],
    injector: &mut dyn AnyInjector,
    exporter: &mut dyn AnyExporter,
    cgroup: Option<&RunCgroup>,
    schema: &Schema,
    defer: bool,
) -> Result<Summary> {
    let (target, stop) = injector.start()?;
    log::info!(
        "profiling pid {} with {} source(s)",
        target.pid,
        sources.len()
    );
    let trigger = Trigger::default();
    let abort = AbortHandle {
        trigger: &trigger,
        stop,
    };

    let summary = cgroup
        .map_or(Ok(()), |cgroup| cgroup.attach(target.pid))
        .map_err(Error::from)
        .and_then(|()| {
            sources
                .iter_mut()
                .try_for_each(|source| source.attach(target))
        })
        .and_then(|()| run(sources, injector, exporter, schema, &abort, defer));
    if summary.is_err() {
        abort.abort();
    }
    let exit_code = injector.wait();

    let mut summary = summary?;
    summary.exit_code = exit_code?;
    Ok(summary)
}

/// Starts the threads, resumes the target once all are ready, then joins them.
fn run(
    sources: &mut [Source],
    injector: &mut dyn AnyInjector,
    exporter: &mut dyn AnyExporter,
    schema: &Schema,
    abort: &AbortHandle<'_>,
    defer: bool,
) -> Result<Summary> {
    thread::scope(|scope| {
        let (boundaries, received) = mpsc::channel();
        let threads = start(scope, sources, exporter, schema, received, abort)
            .inspect_err(|_| abort.abort())?;

        let summary = injector.run(abort.trigger, boundaries, defer);
        abort.trigger.stop();

        threads
            .into_iter()
            .map(join)
            .fold(Ok(()), Result::and)
            .and(summary)
    })
}

/// Starts one thread per source and the exporter thread, and waits until all are ready.
fn start<'scope>(
    scope: &'scope Scope<'scope, '_>,
    sources: &'scope mut [Source],
    exporter: &'scope mut dyn AnyExporter,
    schema: &'scope Schema,
    boundaries: Receiver<PhaseBoundary>,
    abort: &'scope AbortHandle<'scope>,
) -> Result<Vec<ScopedJoinHandle<'scope, Result<()>>>> {
    let mut threads = Vec::new();
    let mut processes = Vec::new();
    for source in sources {
        let (thread, process) = source.start(scope, abort.trigger, abort)?;
        threads.push(thread);
        processes.push(process);
    }
    threads.push(abort.spawn(scope, "exporter".to_owned(), move |ready_tx| {
        export(schema, processes, exporter, &boundaries, ready_tx)
    })?);

    threads.into_iter().map(Starting::wait_for_ready).collect()
}

/// Turns each pair of boundaries into a phase for the exporter.
fn export(
    schema: &Schema,
    mut processes: Vec<Process<'_>>,
    exporter: &mut dyn AnyExporter,
    boundaries: &Receiver<PhaseBoundary>,
    ready: Sender<()>,
) -> Result<()> {
    exporter.begin(schema)?;
    ready.send(()).map_err(Error::exporter)?;
    drop(ready);

    let mut values: Vec<Vec<MetricValue>> = schema
        .sources
        .iter()
        .map(|source| Vec::with_capacity(source.metrics.len()))
        .collect();
    let mut sources = SourceMetrics::of_schema(schema);
    let mut phases = 0;
    let mut start: Option<PhaseBoundary> = None;
    let mut unflushed = false;

    'boundaries: loop {
        let end = match boundaries.try_recv() {
            Ok(end) => end,
            Err(TryRecvError::Empty) => {
                if std::mem::take(&mut unflushed) {
                    exporter.flush()?;
                }
                match boundaries.recv() {
                    Ok(end) => end,
                    Err(_) => break,
                }
            }
            Err(TryRecvError::Disconnected) => break,
        };

        if let Some(start) = start.take() {
            for (process, values) in processes.iter_mut().zip(&mut values) {
                values.clear();
                if !process(values)? {
                    log::debug!("a sensor stopped before the end of phase {phases}");
                    break 'boundaries;
                }
            }
            SourceMetrics::fill_values(&mut sources, &values);

            let phase = PhaseInfo {
                index: phases,
                start_token: start.token,
                end_token: end.token.clone(),
                start_line: start.line,
                end_line: end.line,
                timestamp_us: start.timestamp_us,
                duration_ms: millis_between(start.timestamp_us, end.timestamp_us),
            };
            exporter.export(&phase, &sources)?;
            phases += 1;
            unflushed = true;
        }
        start = Some(end);
    }

    exporter.flush()
}

/// Ends a run early by waking the sensor threads and stopping the target. Threads are joined
/// before the target is waited for, so a reaped process is never stopped.
pub(crate) struct AbortHandle<'a> {
    trigger: &'a Trigger,
    stop: StopHandle,
}

impl AbortHandle<'_> {
    fn abort(&self) {
        self.trigger.stop();
        if let Err(error) = (self.stop)() {
            log::warn!("the target could not be stopped: {error}");
        }
    }

    /// Spawns a thread running `f`, which signals `ready` when ready. If `f` fails or panics, the
    /// run is aborted.
    pub(crate) fn spawn<'scope, T: Send + 'scope>(
        &'scope self,
        scope: &'scope Scope<'scope, '_>,
        name: String,
        f: impl FnOnce(Sender<()>) -> Result<T> + Send + 'scope,
    ) -> Result<Starting<'scope, T>> {
        let (ready_tx, wait_rx) = mpsc::channel();
        let thread_name = name.clone();
        let handle = thread::Builder::new()
            .name(name.clone())
            .spawn_scoped(scope, move || {
                let outcome = panic::catch_unwind(AssertUnwindSafe(|| f(ready_tx)))
                    .unwrap_or_else(|_| Err(Error::Panicked(thread_name)));
                if let Err(error) = &outcome {
                    log::debug!("aborting the run: {error}");
                    self.abort();
                }
                outcome
            })
            .map_err(|error| Error::Spawn { name, error })?;

        Ok(Starting { handle, wait_rx })
    }
}

/// A thread spawned by [`AbortHandle::spawn`], not ready .
pub(crate) struct Starting<'scope, T> {
    handle: ScopedJoinHandle<'scope, Result<T>>,
    wait_rx: Receiver<()>,
}

impl<'scope, T> Starting<'scope, T> {
    /// Waits until the thread is ready, or returns its error if it ends first.
    fn wait_for_ready(self) -> Result<ScopedJoinHandle<'scope, Result<T>>> {
        if self.wait_rx.recv().is_ok() {
            return Ok(self.handle);
        }
        let name = self.handle.thread().name().unwrap_or_default().to_owned();
        join(self.handle).and(Err(Error::NotReady(name)))
    }
}

fn join<T>(handle: ScopedJoinHandle<'_, Result<T>>) -> Result<T> {
    let name = handle.thread().name().unwrap_or_default().to_owned();
    handle.join().map_err(|_| Error::Panicked(name))?
}
