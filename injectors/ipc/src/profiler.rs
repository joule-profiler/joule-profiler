use std::env;
use std::io::{self, BufRead, BufReader, BufWriter, PipeWriter, Write};
use std::net::Shutdown;
use std::os::fd::{FromRawFd, RawFd};
use std::os::unix::net::UnixStream;
use std::panic::{self, AssertUnwindSafe};
use std::sync::Arc;

use joule_profiler_core::error::BoxError;
use joule_profiler_core::exporter::Exporter;
use joule_profiler_core::injector::{Injector, PhaseToken, StopHandle, Target};
use joule_profiler_core::phase::{PhaseInfo, SourceMetrics, Summary};
use joule_profiler_core::profiler::JouleProfiler;
use joule_profiler_core::schema::Schema;

use crate::{DECLARED, DESCRIPTORS, IpcError, Request, STARTED, Values, Written};

/// The profiler's end of a session.
pub struct IpcInjector {
    program: Target,
    requests: BufReader<UnixStream>,
    /// Shared with the stop handle.
    socket: Arc<UnixStream>,
    line: String,
    undeclared: bool,
    exit_code: Option<i32>,
}

impl IpcInjector {
    pub fn new(requests: BufReader<UnixStream>, program: Target) -> io::Result<Self> {
        Ok(Self {
            program,
            socket: Arc::new(requests.get_ref().try_clone()?),
            requests,
            line: String::new(),
            undeclared: false,
            exit_code: None,
        })
    }

    fn request(&mut self) -> Result<Option<Request>, IpcError> {
        read_request(&mut self.requests, &mut self.line)
    }

    fn answer(&self, answer: u8) -> io::Result<()> {
        (&*self.socket).write_all(&[answer])
    }
}

impl Injector for IpcInjector {
    type Error = IpcError;

    fn start(&mut self) -> Result<Target, IpcError> {
        Ok(self.program)
    }

    fn stop_handle(&self) -> StopHandle {
        let socket = Arc::clone(&self.socket);
        Box::new(move || Ok(socket.shutdown(Shutdown::Both)?))
    }

    fn resume(&mut self) -> Result<(), IpcError> {
        Ok(self.answer(STARTED)?)
    }

    fn next_phase(&mut self) -> Result<Option<PhaseToken>, IpcError> {
        if std::mem::take(&mut self.undeclared) && self.answer(DECLARED).is_err() {
            return Ok(None);
        }

        match self.request()? {
            Some(Request::Phase(text)) => {
                self.undeclared = true;
                Ok(Some(PhaseToken { text, line: None }))
            }
            Some(Request::End(code)) => {
                self.exit_code = Some(code);
                Ok(None)
            }
            Some(Request::Configure(_)) => Err(IpcError::Protocol),
            None => Ok(None),
        }
    }

    fn wait(&mut self) -> Result<Option<i32>, IpcError> {
        self.socket.shutdown(Shutdown::Both)?;
        Ok(self.exit_code)
    }
}

pub struct ResultsWriter<W: Write> {
    out: W,
}

impl<W: Write> ResultsWriter<W> {
    pub fn new(out: W) -> Self {
        Self { out }
    }

    fn write(&mut self, written: &Written<'_>) -> io::Result<()> {
        serde_json::to_writer(&mut self.out, written)?;
        self.out.write_all(b"\n")
    }

    pub fn error(&mut self, error: &dyn std::error::Error) -> io::Result<()> {
        self.write(&Written::Error(&error.to_string()))?;
        self.out.flush()
    }
}

impl<W: Write + Send + 'static> Exporter for ResultsWriter<W> {
    type Error = io::Error;

    /// Flushed at once: the program waits for it.
    fn begin(&mut self, schema: &Schema) -> io::Result<()> {
        self.write(&Written::Schema(schema))?;
        self.out.flush()
    }

    fn export(&mut self, phase: &PhaseInfo, sources: &[SourceMetrics<'_>]) -> io::Result<()> {
        self.write(&Written::Phase {
            info: phase,
            values: Values(sources),
        })
    }

    fn flush(&mut self) -> io::Result<()> {
        self.out.flush()
    }

    fn finish(&mut self, summary: &Summary) -> io::Result<()> {
        self.write(&Written::Summary(summary))?;
        self.out.flush()
    }
}

pub(crate) fn serve(
    profiler: Result<JouleProfiler, BoxError>,
    requests: BufReader<UnixStream>,
    results: PipeWriter,
    program: Target,
) -> io::Result<()> {
    let written = results.try_clone()?;
    let outcome = profiler.and_then(|mut profiler| {
        profiler.set_injector(IpcInjector::new(requests, program)?);
        profiler.set_exporter(ResultsWriter::new(BufWriter::new(written)));
        profiler.profile()?;
        Ok(())
    });

    match outcome {
        Ok(()) => Ok(()),
        Err(error) => ResultsWriter::new(results).error(error.as_ref()),
    }
}

/// Call it from the command of [`crate::Session::spawn`], before starting any thread. Returns
/// once the profiler has detached.
pub fn serve_spawned(
    build: impl FnOnce(&str) -> Result<JouleProfiler, BoxError>,
) -> Result<(), IpcError> {
    // SAFETY: `getppid` has no preconditions.
    let program = Target {
        pid: unsafe { libc::getppid() },
    };
    let (socket, results) = inherited()?;

    // SAFETY: `setsid` has no preconditions.
    if unsafe { libc::setsid() } == -1 {
        return Err(io::Error::last_os_error().into());
    }
    if fork()? != 0 {
        return Ok(());
    }

    let served = panic::catch_unwind(AssertUnwindSafe(|| {
        let mut requests = BufReader::new(socket);
        let profiler = configuration(&mut requests).and_then(|config| build(&config));
        serve(profiler, requests, results, program)
    }));
    exit(i32::from(!matches!(served, Ok(Ok(())))))
}

fn configuration(requests: &mut BufReader<UnixStream>) -> Result<String, BoxError> {
    match read_request(requests, &mut String::new())? {
        Some(Request::Configure(config)) => Ok(config),
        _ => Err(IpcError::Protocol.into()),
    }
}

fn read_request(
    requests: &mut BufReader<UnixStream>,
    line: &mut String,
) -> Result<Option<Request>, IpcError> {
    line.clear();
    if requests.read_line(line)? == 0 {
        return Ok(None);
    }
    serde_json::from_str(line)
        .map(Some)
        .map_err(|_| IpcError::Protocol)
}

fn inherited() -> Result<(UnixStream, PipeWriter), IpcError> {
    let descriptors = env::var(DESCRIPTORS).map_err(|_| IpcError::NotSpawned(DESCRIPTORS))?;
    let (socket, results): (RawFd, RawFd) = descriptors
        .split_once(',')
        .and_then(|(socket, results)| Some((socket.parse().ok()?, results.parse().ok()?)))
        .ok_or(IpcError::Protocol)?;

    // SAFETY: `Session::spawn` gave these descriptors to this process, and nothing else owns them.
    unsafe {
        Ok((
            UnixStream::from_raw_fd(socket),
            PipeWriter::from_raw_fd(results),
        ))
    }
}

fn fork() -> io::Result<libc::pid_t> {
    // SAFETY: the calling process has a single thread, so the child inherits no locked lock.
    match unsafe { libc::fork() } {
        -1 => Err(io::Error::last_os_error()),
        pid => Ok(pid),
    }
}

fn exit(code: i32) -> ! {
    // SAFETY: `_exit` ends the process and does nothing else.
    unsafe { libc::_exit(code) }
}

#[cfg(test)]
mod tests {
    use std::convert::Infallible;
    use std::thread;
    use std::time::{Duration, Instant};

    use joule_profiler_core::metric::{MetricInfo, MetricValue};
    use joule_profiler_core::processor::Processor;
    use joule_profiler_core::sensor::Sensor;
    use joule_profiler_core::source::Source;
    use joule_profiler_core::unit::MetricUnit;

    use super::*;
    use crate::{PhaseValues, Received, Results, Session};

    #[derive(Default)]
    struct Ticks(u64);

    impl Sensor for Ticks {
        type Snapshot = u64;
        type Error = Infallible;

        fn name(&self) -> &'static str {
            "ticks"
        }

        fn measure(&mut self) -> Result<u64, Infallible> {
            self.0 += 1;
            Ok(self.0)
        }
    }

    struct Elapsed;

    impl Processor<Ticks> for Elapsed {
        fn metrics(&self) -> Vec<MetricInfo> {
            vec![MetricInfo::new("ticks", MetricUnit::COUNT)]
        }

        fn process(
            &mut self,
            previous: &u64,
            current: &u64,
            values: &mut Vec<MetricValue>,
        ) -> Result<(), Infallible> {
            values.push(MetricValue::U64(current - previous));
            Ok(())
        }
    }

    fn ticks() -> JouleProfiler {
        let mut profiler = JouleProfiler::new();
        profiler.add_source(Source::new(Ticks::default(), Elapsed));
        profiler
    }

    fn this_process() -> Target {
        Target {
            pid: std::process::id().cast_signed(),
        }
    }

    fn profiler_thread(socket: UnixStream, results: PipeWriter) -> thread::JoinHandle<()> {
        thread::spawn(move || {
            serve(Ok(ticks()), BufReader::new(socket), results, this_process()).unwrap();
        })
    }

    fn read(results: Results) -> (Vec<PhaseValues>, Summary) {
        let mut phases = Vec::new();
        for received in results {
            match received.unwrap() {
                Received::Schema(_) => {}
                Received::Phase(phase) => phases.push(phase),
                Received::Summary(summary) => return (phases, summary),
            }
        }
        panic!("the results ended without a summary");
    }

    #[test]
    fn the_phases_a_program_says_come_back_measured() {
        let (program, profiler) = UnixStream::pair().unwrap();
        let (results, written) = io::pipe().unwrap();
        let running = profiler_thread(profiler, written);

        let (mut session, mut results) =
            Session::connect(program, BufReader::new(results)).unwrap();
        assert!(matches!(
            results.next(),
            Some(Ok(Received::Schema(schema))) if schema.sources[0].name == "ticks"
        ));
        session.phase("load").unwrap();
        session.phase("solve").unwrap();
        session.finish(3).unwrap();
        let (phases, summary) = read(results);

        let names: Vec<String> = phases.iter().map(|phase| phase.info.name()).collect();
        assert_eq!(names, ["START -> load", "load -> solve", "solve -> END"]);
        assert!(
            phases
                .iter()
                .all(|phase| phase.values == [[MetricValue::U64(1)]])
        );
        assert_eq!(summary.exit_code, Some(3));
        assert_eq!(summary.phases, 3);
        running.join().unwrap();
    }

    #[test]
    fn a_session_with_more_results_than_a_pipe_holds_ends() {
        let (program, profiler) = UnixStream::pair().unwrap();
        let (results, written) = io::pipe().unwrap();
        let running = profiler_thread(profiler, written);

        let (mut session, results) = Session::connect(program, BufReader::new(results)).unwrap();
        for phase in 0..2_000 {
            session.phase(&format!("phase {phase}")).unwrap();
        }
        session.finish(0).unwrap();

        assert_eq!(read(results).0.len(), 2_001);
        running.join().unwrap();
    }

    #[test]
    fn a_profiler_that_fails_says_why() {
        let (program, profiler) = UnixStream::pair().unwrap();
        let (results, written) = io::pipe().unwrap();
        thread::spawn(move || {
            serve(
                Err("no source available".into()),
                BufReader::new(profiler),
                written,
                this_process(),
            )
        });

        let outcome = Session::connect(program, BufReader::new(results));

        assert!(
            matches!(outcome, Err(IpcError::Profiler(message)) if message == "no source available")
        );
    }

    #[test]
    fn the_configuration_is_read_first_without_losing_what_follows() {
        let (mut program, profiler) = UnixStream::pair().unwrap();
        program
            .write_all(b"{\"Configure\":\"ticks\"}\n{\"Phase\":\"load\"}\n")
            .unwrap();
        let mut requests = BufReader::new(profiler);

        assert_eq!(configuration(&mut requests).unwrap(), "ticks");

        let mut injector = IpcInjector::new(requests, this_process()).unwrap();
        assert!(matches!(injector.next_phase(), Ok(Some(token)) if token.text == "load"));
    }

    #[test]
    fn a_profiler_that_is_not_configured_first_refuses_to_start() {
        let (mut program, profiler) = UnixStream::pair().unwrap();
        program.write_all(b"{\"Phase\":\"load\"}\n").unwrap();

        assert!(configuration(&mut BufReader::new(profiler)).is_err());
    }

    #[test]
    fn the_stop_handle_wakes_a_profiler_waiting_for_a_phase() {
        let (program, profiler) = UnixStream::pair().unwrap();
        let mut injector = IpcInjector::new(BufReader::new(profiler), this_process()).unwrap();
        let stop = injector.stop_handle();
        let waiting = thread::spawn(move || injector.next_phase());

        thread::sleep(Duration::from_millis(50));
        let stopped = Instant::now();
        stop().unwrap();

        assert!(matches!(waiting.join().unwrap(), Ok(None)));
        assert!(stopped.elapsed() < Duration::from_secs(1));
        drop(program);
    }

    #[test]
    fn a_program_that_disappears_ends_the_run() {
        let (program, profiler) = UnixStream::pair().unwrap();
        let mut injector = IpcInjector::new(BufReader::new(profiler), this_process()).unwrap();
        let waiting = thread::spawn(move || injector.next_phase());

        thread::sleep(Duration::from_millis(50));
        drop(program);

        assert!(matches!(waiting.join().unwrap(), Ok(None)));
    }
}
