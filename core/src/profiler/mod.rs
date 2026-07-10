//! This module contains the core logic of profiling and is the entrypoint of Joule Profiler.
//!
//! It provides the main [`JouleProfiler`] struct, orchestrating
//! the execution of commands, collecting metrics from various sources (e.g. RAPL, `perf_event`, NVML, etc.),
//! and aggregate them into a clean common structure.

use log::{debug, info, trace};
use regex::Regex;
use std::io::BufWriter;
use std::os::unix::process::CommandExt;
use std::process::{Child, ChildStdout, Command};
use std::thread::JoinHandle;
use std::{
    io::{BufRead, BufReader, ErrorKind, Write},
    process::{self, Stdio},
};

pub mod error;

use crate::config::ProfileConfig;
use crate::orchestrator::Orchestrator;
use crate::phase::{PhaseInfo, PhaseToken};
use crate::profiler::types::{Phase, ProfilerResults, Result};
use crate::sensor::{Sensor, Sensors};
use crate::source::{MetricReader, MetricSource, MetricSourceError};
use crate::util::fs::create_file_with_user_permissions;
use crate::util::sys::{get_uid_from_username, is_root, signal};
use crate::util::time::get_timestamp_micros;
pub use error::JouleProfilerError;

pub mod types;

/// Phases and begin/end timestamps collected by the reader thread.
struct MeasureData {
    phases: Vec<PhaseInfo>,
    begin_timestamp: u128,
    end_timestamp: u128,
}

/// Reader thread handle: gives the orchestrator back with the collected data.
type ReaderThreadHandle = JoinHandle<(Orchestrator, Result<MeasureData>)>;

/// Orchestrates program profiling and metric collection.
///
/// `JouleProfiler` runs a command, collects energy metrics from registered
/// sources (e.g. RAPL, `perf_event`, NVML), and aggregates them into a
/// structured result organized by phases.
///
/// It is also responsible for the detection of phases (parts of a program execution)
/// through the standard output.
///
/// # Examples
///
/// ```no_run
/// use joule_profiler_core::{JouleProfiler, config::ProfileConfig};
///
/// # tokio_test::block_on(async {
/// let mut profiler = JouleProfiler::new();
///
/// // Add sources using profiler.add_source(source)
///
/// let config = ProfileConfig {
///     cmd: vec!["echo".to_string(), "hello".to_string()],
///     token_pattern: "__PHASE__".to_string(),
///     stdout_file: None,
///     use_root: false,
///     init_timeout: std::time::Duration::from_secs(1),
/// };
///
/// let results = profiler.profile(&config).await.unwrap();
/// # });
/// ```
#[derive(Default)]
pub struct JouleProfiler {
    /// The different metric sources.
    sources: Vec<Box<dyn MetricSource>>,
}

impl JouleProfiler {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a custom metric source to the profiler.
    ///
    /// A source must implement the [`MetricReader`] trait.
    pub fn add_source<T>(&mut self, reader: T)
    where
        T: MetricReader,
    {
        debug!("Registering additional metric source: {}", T::get_name());
        trace!("MetricReader type: {}", std::any::type_name::<T>());
        self.sources.push(reader.into());
    }

    /// List the sensors of the provided sources.
    pub fn list_sensors(&mut self) -> Result<Sensors> {
        debug!("Listing sensors from {} source(s)", self.sources.len());

        let sensors: Vec<Sensor> = self
            .sources
            .iter()
            .enumerate()
            .map(|(i, source)| {
                trace!("Querying sensors from source {i}");
                source.list_sensors().map_err(MetricSourceError::into)
            })
            .collect::<Result<Vec<Sensors>>>()?
            .into_iter()
            .flatten()
            .collect();

        info!("Discovered {} sensor(s)", sensors.len());
        Ok(sensors)
    }

    /// Profiles a program spawned with the configured command and return the aggregated results.
    ///
    /// It spawns the program, initializes the metric sources with its pid, starts the
    /// orchestrator, and profiles the program.
    pub async fn profile(&mut self, config: &ProfileConfig) -> Result<ProfilerResults> {
        info!("Running phase-based profiling");
        debug!("Phase regex: {}", config.token_pattern);

        let regex = Regex::new(&config.token_pattern)?;

        let sources = std::mem::take(&mut self.sources);
        if sources.is_empty() {
            return Err(JouleProfilerError::NoSourceConfigured);
        }
        let mut orchestrator = Orchestrator::new(sources);

        orchestrator.pre_init().await?;

        debug!("Spawning command: {:?}", config.cmd);
        let mut child = spawn_profiled_command(config)?;
        let pid = child.id().cast_signed();

        pause_process(pid)?;

        let child_stdout = child
            .stdout
            .take()
            .ok_or(JouleProfilerError::StdOutCaptureFail)?;

        orchestrator.init(pid, config.init_timeout).await?;
        orchestrator.run();

        info!("Starting measurements");
        let reader_thread = spawn_reader_thread(
            orchestrator,
            child_stdout,
            regex,
            config.stdout_file.clone(),
            pid,
        )?;

        let (mut orchestrator, measure_result) =
            tokio::task::spawn_blocking(move || join_reader_thread(reader_thread))
                .await
                .map_err(|_| JouleProfilerError::ReaderThreadPanicked)??;

        let MeasureData {
            phases: detected_phases,
            begin_timestamp,
            end_timestamp,
        } = match measure_result {
            Ok(data) => data,
            Err(err) => {
                return Err(match orchestrator.finalize().await {
                    Err(source_err) => source_err.into(),
                    Ok(_) => err,
                });
            }
        };

        let command_duration_ms = (end_timestamp - begin_timestamp) / 1000;
        let timestamp = begin_timestamp;
        let exit_code = tokio::task::spawn_blocking(move || wait_for_child_exit(&mut child))
            .await
            .map_err(|_| {
                JouleProfilerError::ProcessControlFailed("wait thread panicked".to_string())
            })??;
        info!("Command finished: duration={command_duration_ms} ms exit_code={exit_code}");

        let (sources_results, sources) = orchestrator.finalize().await?;
        self.sources = sources;

        let mut phases: Vec<_> = detected_phases
            .windows(2)
            .enumerate()
            .zip(&sources_results.phases)
            .map(|((index, window), real_phase)| {
                let (d1, d2) = (&window[0], &window[1]);
                let mut phase_metrics = real_phase.metrics.clone();
                phase_metrics.sort_by(|a, b| a.name.cmp(&b.name));
                Phase {
                    index,
                    metrics: phase_metrics,
                    start_token: d1.token.clone(),
                    end_token: d2.token.clone(),
                    timestamp: d1.timestamp,
                    duration_ms: (d2.timestamp - d1.timestamp) / 1000,
                    start_token_line: d1.line_number,
                    end_token_line: d2.line_number,
                }
            })
            .collect();

        if phases.is_empty()
            && let Some(end_phase) = sources_results.phases.into_iter().last()
        {
            phases.push(Phase {
                index: 0,
                metrics: end_phase.metrics,
                start_token: PhaseToken::Start,
                end_token: PhaseToken::End,
                timestamp,
                duration_ms: command_duration_ms,
                start_token_line: None,
                end_token_line: None,
            });
        }

        debug!("Collected {} sensor phase(s)", phases.len());
        Ok(ProfilerResults {
            timestamp,
            duration_ms: command_duration_ms,
            exit_code,
            phases,
        })
    }
}

/// Profiles the already spawned and paused command, on the reader thread:
/// first measure, resume the process, then a blocking measure and new phase
/// for each detected token, until the stdout closes.
fn measure_phases_blocking(
    orchestrator: &mut Orchestrator,
    child_stdout: ChildStdout,
    regex: &Regex,
    stdout_file: Option<&String>,
    pid: i32,
) -> Result<MeasureData> {
    let sink = create_output_sink(stdout_file)?;
    let reader = BufReader::new(child_stdout);

    let mut detected_phases = Vec::with_capacity(2);

    let begin_timestamp = get_timestamp_micros();
    trace!("Begin timestamp: {begin_timestamp}");

    orchestrator.measure_blocking()?;
    resume_process(pid)?;
    detected_phases.push(PhaseInfo::start(begin_timestamp));

    read_and_detect_phases(orchestrator, &mut detected_phases, reader, regex, sink)?;

    let end_timestamp = get_timestamp_micros();
    trace!("End timestamp: {end_timestamp}");

    orchestrator.measure_blocking()?;
    orchestrator.new_phase_blocking()?;
    detected_phases.push(PhaseInfo::end(end_timestamp));

    Ok(MeasureData {
        phases: detected_phases,
        begin_timestamp,
        end_timestamp,
    })
}

/// Spawns the reader thread, which owns the orchestrator during the run and
/// gives it back on join.
fn spawn_reader_thread(
    mut orchestrator: Orchestrator,
    child_stdout: ChildStdout,
    regex: Regex,
    stdout_file: Option<String>,
    pid: i32,
) -> Result<ReaderThreadHandle> {
    std::thread::Builder::new()
        .name("phase-reader".to_string())
        .spawn(move || {
            let result = measure_phases_blocking(
                &mut orchestrator,
                child_stdout,
                &regex,
                stdout_file.as_ref(),
                pid,
            );
            (orchestrator, result)
        })
        .map_err(|err| JouleProfilerError::ReaderThreadSpawnFailed(err.to_string()))
}

fn join_reader_thread<T>(handle: JoinHandle<T>) -> Result<T> {
    handle
        .join()
        .map_err(|_| JouleProfilerError::ReaderThreadPanicked)
}

/// Reads the child's stdout line by line; every line matching the regex
/// triggers a blocking measure and new phase, and records a [`PhaseInfo`].
fn read_and_detect_phases<R, W>(
    orchestrator: &mut Orchestrator,
    phases: &mut Vec<PhaseInfo>,
    mut reader: R,
    regex: &Regex,
    mut sink: W,
) -> Result<()>
where
    R: BufRead,
    W: Write,
{
    let mut line = String::new();
    let mut line_number: usize = 0;

    loop {
        line.clear();

        match reader.read_line(&mut line) {
            Ok(0) => break,
            Ok(_) => {}
            Err(e) if e.kind() == ErrorKind::InvalidData => {
                trace!("Skipping invalid UTF-8 output at line {line_number}");
                line_number += 1;
                continue;
            }
            Err(e) => return Err(e.into()),
        }

        if line.ends_with('\n') {
            line.pop();
            if line.ends_with('\r') {
                line.pop();
            }
        }

        writeln!(sink, "{line}")?;

        if let Some(token) = phase_token_in_line(regex, &line) {
            let phase_timestamp = get_timestamp_micros();
            debug!("Detected phase at line {line_number}, token '{token}'");

            orchestrator.measure_blocking()?;
            orchestrator.new_phase_blocking()?;

            phases.push(PhaseInfo {
                token: PhaseToken::Token(token.to_owned()),
                timestamp: phase_timestamp,
                line_number: Some(line_number),
            });
        }

        line_number += 1;
    }

    sink.flush()?;
    Ok(())
}

/// Checks whether a line matches the specified regular expression.
pub fn phase_token_in_line<'a>(regex: &Regex, line: &'a str) -> Option<&'a str> {
    regex.find(line).map(|mat| mat.as_str())
}

/// Spawns a sub-process with the specified command and arguments.
///
/// Returns the attached sub-process on success. If an error occur, a [`JouleProfilerError::CommandNotFound`]
/// error is returned if the specified program cannot be found, or a [`JouleProfilerError::CommandExecutionFailed`] otherwise.
///
/// The standard output is piped to be analyzed for phases detection.
fn spawn_profiled_command(config: &ProfileConfig) -> Result<Child> {
    let mut command = init_command(&config.cmd, config.use_root)?;

    command.spawn().map_err(|err| {
        if err.kind() == ErrorKind::NotFound {
            JouleProfilerError::CommandNotFound(config.cmd[0].clone())
        } else {
            JouleProfilerError::CommandExecutionFailed(err.to_string())
        }
    })
}

/// Initializes the command used to spawn the profiled process and handles it's privileges.
///
/// If the current user is root and the parameter `use_root` is true, then the command
/// is spawned with root privileges, else the real user id is retrieved and the process
/// runs with user privileges. If Joule Profiler is not launched with root privileges,
/// then the program is spawned with default user privileges.
///
/// An error can occur if:
/// - The `SUDO_USER` environment variable cannot be retrieved, even so the user is root.
/// - The user uid cannot be retrieved with it's username provided by the environment variable.  
pub fn init_command(cmd: &[String], use_root: bool) -> Result<Command> {
    let mut command = if let Some(program) = cmd.first() {
        process::Command::new(program)
    } else {
        return Err(JouleProfilerError::EmptyCommand);
    };

    if cmd.len() > 1 {
        command.args(&cmd[1..]);
    }

    if is_root() && !use_root {
        let username =
            std::env::var("SUDO_USER").map_err(|_| JouleProfilerError::CannotRetrieveSudoUser)?;
        let uid = get_uid_from_username(&username)?;
        command.uid(uid);
    }

    command.stdout(Stdio::piped());
    command.stderr(Stdio::inherit());

    Ok(command)
}

/// Waits for the sub-process termination, returns the status code of the child.
///
/// If the child cannot be terminated, its associated error will be forwarded, also if the
/// exit status code cannot be retrieved, 1 is returned, signifying that an error occured.
fn wait_for_child_exit(child: &mut process::Child) -> Result<i32> {
    let status = child.wait()?;
    Ok(status.code().unwrap_or(1))
}

/// Creates a sink to be able to write the program output into either the process output file, either the standard output of the profiler.
///
/// A buffered writer is used to limit the system calls made, thus reducing the overhead introduced by the profiler.
fn create_output_sink(path: Option<&String>) -> Result<Box<dyn Write>> {
    if let Some(path) = path {
        let file = create_file_with_user_permissions(path).map_err(|err| {
            JouleProfilerError::OutputFileCreationFailed(format!("{path:?}: {err}"))
        })?;

        Ok(Box::new(BufWriter::new(file)))
    } else {
        Ok(Box::new(BufWriter::new(std::io::stdout().lock())))
    }
}

/// Sends `SIGSTOP` to a child process to pause its execution.
///
/// # Preconditions
///
/// - 'pid' must refer to a valid process identifier obtained from
///   '`std::process::Child::id()`' immediately after spawning.
/// - The process must be owned by the current user (we only signal
///   child processes we created).
///
/// Returns [`JouleProfilerError::ProcessControlFailed`] if the
/// signal delivery fails.
fn pause_process(pid: i32) -> Result<()> {
    signal(pid, libc::SIGSTOP)
}

/// Sends `SIGCONT` to resume a previously paused process.
///
/// # Preconditions
///
/// - `pid` must refer to a valid, running or stopped child process.
/// - The process must still exist when the signal is sent.
///
/// Returns [`JouleProfilerError::ProcessControlFailed`] if the
/// signal delivery fails.
fn resume_process(pid: i32) -> Result<()> {
    signal(pid, libc::SIGCONT)
}

#[cfg(test)]
mod tests {
    use crate::config::ProfileConfig;
    use crate::orchestrator::Orchestrator;
    use crate::phase::{PhaseInfo, PhaseToken};
    use crate::profiler::{
        create_output_sink, join_reader_thread, phase_token_in_line, read_and_detect_phases,
        spawn_profiled_command, wait_for_child_exit,
    };
    use crate::sensor::Sensors;
    use crate::source::MetricReader;
    use crate::types::Metrics;
    use crate::{JouleProfiler, JouleProfilerError};
    use mockall::mock;
    use regex::Regex;
    use std::fs;
    use std::io::{BufReader, Cursor, Read, Write};
    use std::time::Duration;
    use tempfile::TempDir;

    fn joule_profiler() -> JouleProfiler {
        JouleProfiler {
            sources: Vec::new(),
        }
    }

    /// Drives [`read_and_detect_phases`] to completion over an in-memory reader
    /// with a sourceless orchestrator, and collects the recorded [`PhaseInfo`]s.
    fn collect_phases<R, W>(
        reader: R,
        regex: &Regex,
        sink: W,
    ) -> crate::profiler::types::Result<Vec<PhaseInfo>>
    where
        R: std::io::BufRead,
        W: Write,
    {
        let mut orchestrator = Orchestrator::new(Vec::new());
        let mut phases = Vec::new();
        read_and_detect_phases(&mut orchestrator, &mut phases, reader, regex, sink)?;
        Ok(phases)
    }

    fn create_test_config(cmd: Vec<String>) -> ProfileConfig {
        ProfileConfig {
            cmd,
            token_pattern: "__PHASE__".to_string(),
            stdout_file: None,
            use_root: false,
            init_timeout: Duration::from_secs(1),
        }
    }

    #[derive(Debug)]
    pub struct MockError;

    impl std::fmt::Display for MockError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "mock error")
        }
    }

    impl std::error::Error for MockError {}

    mock! {
        pub MetricReader {}

        impl MetricReader for MetricReader {
            type Type = ();
            type Error = MockError;
            type Config = ();
            fn from_config(config: ()) -> Result<Self, MockError>;
            async fn init(&mut self, pid: i32) -> Result<(), MockError>;
            async fn join(&mut self) -> Result<(), MockError>;
            async fn measure(&mut self) -> Result<(), MockError>;
            async fn retrieve(&mut self) -> Result<(), MockError>;
            fn get_sensors(&self) -> Result<Sensors, MockError>;
            fn to_metrics(&self, v: ()) -> Result<Metrics, MockError>;
            fn get_name() -> &'static str;
            fn get_id() -> &'static str;
        }
    }

    #[test]
    fn detect_multiple_phases() {
        let regex = Regex::new("__[A-Z0-9_]+__").unwrap();
        let cursor = Cursor::new("__PHASE1__\n__PHASE2__\n__PHASE3__");
        let reader = BufReader::new(cursor);
        let sink: Vec<u8> = Vec::new();

        let phases = collect_phases(reader, &regex, sink).unwrap();

        assert_eq!(3, phases.len());
        assert_eq!(PhaseToken::Token("__PHASE1__".to_string()), phases[0].token);
        assert_eq!(PhaseToken::Token("__PHASE2__".to_string()), phases[1].token);
        assert_eq!(PhaseToken::Token("__PHASE3__".to_string()), phases[2].token);
    }

    #[test]
    fn detect_no_phases() {
        let regex = Regex::new("__[A-Z0-9_]+__").unwrap();
        let cursor = Cursor::new("hello\nworld\nno phases here");
        let reader = BufReader::new(cursor);
        let sink: Vec<u8> = Vec::new();

        let phases = collect_phases(reader, &regex, sink).unwrap();

        assert!(phases.is_empty());
    }

    #[test]
    fn detect_empty_output() {
        let regex = Regex::new("__PHASE__").unwrap();
        let cursor = Cursor::new("");
        let reader = BufReader::new(cursor);
        let sink: Vec<u8> = Vec::new();

        let phases = collect_phases(reader, &regex, sink).unwrap();

        assert_eq!(phases.len(), 0);
    }

    #[test]
    fn detect_phase_in_middle_of_line() {
        let regex = Regex::new("__PHASE[0-9]+__").unwrap();
        let cursor = Cursor::new("start __PHASE1__ end");
        let reader = BufReader::new(cursor);
        let sink: Vec<u8> = Vec::new();

        let phases = collect_phases(reader, &regex, sink).unwrap();

        assert_eq!(phases.len(), 1);
        assert_eq!(phases[0].token, PhaseToken::Token("__PHASE1__".to_string()));
        assert_eq!(phases[0].line_number, Some(0));
    }

    #[test]
    fn detect_correct_line_numbers() {
        let regex = Regex::new("__PHASE[0-9]+__").unwrap();
        let cursor = Cursor::new("a\nb\n__PHASE1__\nc\n__PHASE2__");
        let reader = BufReader::new(cursor);
        let sink: Vec<u8> = Vec::new();

        let phases = collect_phases(reader, &regex, sink).unwrap();

        assert_eq!(phases.len(), 2);
        assert_eq!(phases[0].line_number, Some(2));
        assert_eq!(phases[1].line_number, Some(4));
    }

    #[test]
    fn writes_stdout_to_file() {
        use std::fs;
        use tempfile::NamedTempFile;

        let regex = Regex::new("__PHASE__").unwrap();
        let cursor = Cursor::new("hello\n__PHASE__\nworld");
        let reader = BufReader::new(cursor);

        let mut temp_file = NamedTempFile::new().unwrap();

        collect_phases(reader, &regex, temp_file.as_file_mut()).unwrap();

        let content = fs::read_to_string(temp_file.path()).unwrap();
        assert!(content.contains("hello"));
        assert!(content.contains("__PHASE__"));
        assert!(content.contains("world"));
    }

    #[test]
    fn skips_invalid_utf8_lines() {
        let regex = Regex::new("__PHASE__").unwrap();

        let bytes = vec![
            0xff, 0xfe, b'\n', b'_', b'_', b'P', b'H', b'A', b'S', b'E', b'_', b'_',
        ];
        let cursor = Cursor::new(bytes);
        let reader = BufReader::new(cursor);
        let sink: Vec<u8> = Vec::new();

        let phases = collect_phases(reader, &regex, sink).unwrap();

        assert_eq!(phases.len(), 1);
    }

    #[test]
    fn join_reader_thread_propagates_panic_as_error() {
        let handle = std::thread::spawn(|| -> crate::profiler::types::Result<()> {
            panic!("boom");
        });

        let result = join_reader_thread(handle);

        assert!(matches!(
            result,
            Err(JouleProfilerError::ReaderThreadPanicked)
        ));
    }

    #[test]
    fn join_reader_thread_returns_thread_result() {
        let handle = std::thread::spawn(|| -> crate::profiler::types::Result<()> {
            Err(JouleProfilerError::StdOutCaptureFail)
        });

        let result = join_reader_thread(handle);

        assert!(matches!(
            result,
            Ok(Err(JouleProfilerError::StdOutCaptureFail))
        ));
    }

    #[test]
    fn phase_token_in_line_returns_none_when_no_match() {
        let regex = Regex::new("X").unwrap();
        assert_eq!(phase_token_in_line(&regex, "abc"), None);
    }

    #[test]
    fn phase_token_in_line_returns_some_when_match_exists() {
        let regex = Regex::new("X").unwrap();
        assert_eq!(phase_token_in_line(&regex, "aXc"), Some("X"));
    }

    #[test]
    fn phase_token_in_line_returns_first_match_only() {
        let regex = Regex::new("X").unwrap();
        assert_eq!(phase_token_in_line(&regex, "XX"), Some("X"));
    }

    #[test]
    fn phase_token_in_line_does_not_trim_or_modify_input() {
        let regex = Regex::new("X").unwrap();
        assert_eq!(phase_token_in_line(&regex, "  X  "), Some("X"));
    }

    #[test]
    fn phase_token_in_line_returns_slice_from_input() {
        let regex = Regex::new("X").unwrap();
        let line = String::from("aXc");

        let token = phase_token_in_line(&regex, &line).unwrap();

        let line_ptr = line.as_ptr() as usize;
        let tok_ptr = token.as_ptr() as usize;
        assert!(tok_ptr >= line_ptr && tok_ptr < line_ptr + line.len());
    }

    #[test]
    fn phase_token_in_line_empty_line_returns_none() {
        let regex = Regex::new("X").unwrap();
        assert_eq!(phase_token_in_line(&regex, ""), None);
    }

    #[test]
    fn phase_token_in_line_full_line_match() {
        let regex = Regex::new(".*").unwrap();
        assert_eq!(phase_token_in_line(&regex, "abc"), Some("abc"));
    }

    #[tokio::test]
    async fn profile_invalid_regex_returns_error() {
        let mut profiler = joule_profiler();
        let config = ProfileConfig {
            cmd: vec!["echo".to_string()],
            token_pattern: "[[invalid[[[regex[[".to_string(),
            stdout_file: None,
            use_root: false,
            init_timeout: Duration::from_secs(1),
        };
        profiler.add_source(MockMetricReader::new());
        let result = profiler.profile(&config).await;
        assert!(matches!(result, Err(JouleProfilerError::InvalidPattern(_))));
    }

    #[test]
    fn create_output_sink_none_returns_stdout_sink() {
        assert!(create_output_sink(None).is_ok());
    }

    #[test]
    fn create_output_sink_with_path_creates_file() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("out.txt").to_str().unwrap().to_owned();

        let result = create_output_sink(Some(&path));
        assert!(result.is_ok());
        assert!(fs::metadata(&path).is_ok());
    }

    #[test]
    fn create_output_sink_invalid_path_returns_error() {
        let result = create_output_sink(Some(&"/nonexistent/dir/out.txt".to_string()));
        assert!(result.is_err());
        assert!(matches!(
            result.err().unwrap(),
            JouleProfilerError::OutputFileCreationFailed(_)
        ));
    }

    #[test]
    fn spawn_profiled_command_with_valid_command() {
        let config = create_test_config(vec!["echo".to_string(), "hello".to_string()]);

        let result = spawn_profiled_command(&config);

        assert!(result.is_ok());
        let mut child = result.unwrap();

        assert!(child.stdout.is_some());

        let _ = child.kill();
        let _ = child.wait();
    }

    #[test]
    fn spawn_profiled_command_with_nonexistent_command() {
        let config = create_test_config(vec!["mais_t_es_pas_la_mais_t_es_ou".to_string()]);

        let result = spawn_profiled_command(&config);

        assert!(result.is_err());
        match result.unwrap_err() {
            JouleProfilerError::CommandNotFound(cmd) => {
                assert_eq!(cmd, "mais_t_es_pas_la_mais_t_es_ou");
            }
            _ => panic!("Expected CommandNotFound error"),
        }
    }

    #[test]
    fn spawn_profiled_command_with_single_arg() {
        let config = create_test_config(vec!["echo".to_string()]);

        let result = spawn_profiled_command(&config);

        assert!(result.is_ok());
        let mut child = result.unwrap();
        let _ = child.kill();
        let _ = child.wait();
    }

    #[test]
    fn spawn_profiled_command_with_multiple_args() {
        let config = create_test_config(vec![
            "echo".to_string(),
            "help".to_string(),
            "me".to_string(),
            "plz".to_string(),
        ]);

        let result = spawn_profiled_command(&config);

        assert!(result.is_ok());
        let mut child = result.unwrap();

        let mut output = String::new();
        if let Some(mut stdout) = child.stdout.take() {
            stdout.read_to_string(&mut output).unwrap();
        }

        assert!(output.contains("help"));
        assert!(output.contains("me"));
        assert!(output.contains("plz"));

        let _ = child.wait();
    }

    #[cfg(unix)]
    #[test]
    fn spawn_profiled_command_permission_denied() {
        use std::os::unix::fs::PermissionsExt;

        let temp_dir = TempDir::new().unwrap();
        let script_path = temp_dir.path().join("no_exec.sh");

        fs::write(&script_path, "#!/bin/sh\necho test").unwrap();
        let mut perms = fs::metadata(&script_path).unwrap().permissions();
        perms.set_mode(0o644); // rw-r--r--
        fs::set_permissions(&script_path, perms).unwrap();

        let config = create_test_config(vec![script_path.to_string_lossy().to_string()]);

        let result = spawn_profiled_command(&config);

        assert!(result.is_err());
        match result.unwrap_err() {
            JouleProfilerError::CommandExecutionFailed(_) => (),
            _ => panic!("Expected CommandExecutionFailed error"),
        }
    }

    #[test]
    fn wait_for_child_exit_zero_on_success() {
        let config = create_test_config(vec!["true".to_string()]);
        let mut child = spawn_profiled_command(&config).unwrap();
        assert_eq!(wait_for_child_exit(&mut child).unwrap(), 0);
    }

    #[test]
    fn wait_for_child_exit_nonzero_on_failure() {
        let config = create_test_config(vec!["false".to_string()]);
        let mut child = spawn_profiled_command(&config).unwrap();
        assert_ne!(wait_for_child_exit(&mut child).unwrap(), 0);
    }

    #[test]
    fn list_sensors_no_sources_returns_empty() {
        let mut profiler = joule_profiler();
        let sensors = profiler.list_sensors().unwrap();
        assert!(sensors.is_empty());
    }

    #[tokio::test]
    async fn run_with_no_source_returns_no_source_configured_error() {
        let config = create_test_config(vec!["test".to_string()]);
        assert!(matches!(
            joule_profiler().profile(&config).await,
            Err(JouleProfilerError::NoSourceConfigured)
        ));
    }
}
