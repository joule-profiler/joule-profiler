use std::sync::mpsc::Sender;
use std::time::Instant;

use crate::error::{BoxError, Error, Result};
use crate::phase::{PhaseBoundary, Summary};
use crate::trigger::Trigger;
use crate::util::time::{get_timestamp_micros, millis_between};

/// The profiled process.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Target {
    pub pid: i32,
}

/// A phase boundary found by an injector.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PhaseToken {
    pub text: String,
    pub line: Option<usize>,
}

/// Stops the target from any thread, so that a blocked [`Injector::next_phase`] returns.
pub type StopHandle = Box<dyn Fn() -> Result<(), BoxError> + Send + Sync>;

/// Starts the target program and reports its phase boundaries.
pub trait Injector: Send + 'static {
    type Error: std::error::Error + Send + Sync + 'static;

    /// Starts the target paused until `resume`.
    fn start(&mut self) -> Result<Target, Self::Error>;

    fn stop_handle(&self) -> StopHandle;

    fn resume(&mut self) -> Result<(), Self::Error>;

    /// Blocks until the next boundary; `None` once the target is done.
    fn next_phase(&mut self) -> Result<Option<PhaseToken>, Self::Error>;

    /// Waits for the target to end and returns its exit code.
    fn wait(&mut self) -> Result<Option<i32>, Self::Error>;
}

/// Type-erased [`Injector`].
pub(crate) trait AnyInjector: Send {
    fn start(&mut self) -> Result<(Target, StopHandle)>;

    /// Resumes the target and sends its boundaries until it ends.
    fn run(
        &mut self,
        trigger: &Trigger,
        boundaries: Sender<PhaseBoundary>,
        defer: bool,
    ) -> Result<Summary>;

    fn wait(&mut self) -> Result<Option<i32>>;
}

impl<I: Injector> AnyInjector for I {
    fn start(&mut self) -> Result<(Target, StopHandle)> {
        let target = Injector::start(self).map_err(Error::injector)?;
        Ok((target, self.stop_handle()))
    }

    fn run(
        &mut self,
        trigger: &Trigger,
        boundaries: Sender<PhaseBoundary>,
        defer: bool,
    ) -> Result<Summary> {
        let send = |boundary| boundaries.send(boundary).map_err(Error::exporter);
        if !defer {
            return run_with(self, trigger, send);
        }

        let mut phases = Vec::new();
        let summary = run_with(self, trigger, |boundary| {
            phases.push(boundary);
            Ok(())
        })?;
        phases.into_iter().try_for_each(send)?;
        Ok(summary)
    }

    fn wait(&mut self) -> Result<Option<i32>> {
        Injector::wait(self).map_err(Error::injector)
    }
}

fn run_with(
    injector: &mut impl Injector,
    trigger: &Trigger,
    mut send_fn: impl FnMut(PhaseBoundary) -> Result<()>,
) -> Result<Summary> {
    let start = Instant::now();
    let start_timestamp = get_timestamp_micros();
    let timestamp = || start_timestamp + start.elapsed().as_micros();

    injector.resume().map_err(Error::injector)?;
    send_fn(PhaseBoundary {
        token: "START".to_owned(),
        line: None,
        timestamp_us: start_timestamp,
    })?;

    let mut phases = 0;
    let end_timestamp = loop {
        let token = injector.next_phase().map_err(Error::injector)?;
        let end_timestamp = timestamp();
        trigger.trigger();
        phases += 1;

        let ends = token.is_none() || !trigger.is_running();
        let (token, line) = token.map_or_else(
            || ("END".to_owned(), None),
            |token| (token.text, token.line),
        );
        send_fn(PhaseBoundary {
            token,
            line,
            timestamp_us: end_timestamp,
        })?;
        if ends {
            break end_timestamp;
        }
    };

    Ok(Summary {
        timestamp_us: start_timestamp,
        duration_ms: millis_between(start_timestamp, end_timestamp),
        phases,
        exit_code: None,
    })
}
