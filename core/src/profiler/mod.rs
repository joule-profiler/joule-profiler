//! This module contains the core logic of profiling and is the entrypoint of Joule Profiler.
//!
//! It provides the main [`JouleProfiler`] struct, orchestrating
//! the execution of commands, collecting metrics from various sources (e.g. RAPL, `perf_event`, NVML, etc.),
//! and aggregate them into a clean common structure.

use log::{debug, info, trace};
use regex::Regex;
use std::io::BufWriter;
use std::os::unix::process::CommandExt;
use std::process::{Child, Command};
use std::{
    io::{BufRead, BufReader, ErrorKind, Write},
    process::{self, Stdio},
};
use tokio::task::{JoinHandle, spawn_blocking};

pub mod error;

use crate::config::ProfileConfig;
use crate::orchestrator::SourceOrchestrator;
use crate::phase::{PhaseInfo, PhaseToken};
use crate::profiler::types::{MeasurePhasesReturnType, Phase, ProfilerResults, Result};
use crate::sensor::{Sensor, Sensors};
use crate::source::{MetricReader, MetricSource, MetricSourceError};
use crate::util::fs::create_file_with_user_permissions;
use crate::util::sys::{get_uid_from_username, geteuid, signal};
use crate::util::time::get_timestamp_micros;
pub use error::JouleProfilerError;

pub mod types;

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
/// };
///
/// let results = profiler.profile(&config).await.unwrap();
/// # });
/// ```
#[derive(Default)]
pub struct JouleProfiler {
    /// The sources orchestrator, managing sources and sending them the events sent by the profiler.
    orchestrator: SourceOrchestrator,

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
    /// It starts the orchestrator with the metric sources and profile the program.
    pub async fn profile(&mut self, config: &ProfileConfig) -> Result<ProfilerResults> {
        info!("Running phase-based profiling");
        debug!("Phase regex: {}", config.token_pattern);

        let sources = std::mem::take(&mut self.sources);
        trace!("Starting orchestrator with {} source(s)", sources.len());
        self.orchestrator.run(sources)?;

        info!("Starting measurements");
        let (command_duration_ms, timestamp, exit_code, detected_phases) =
            self.measure_phases(config).await?;

        let (sources_results, sources) = self.orchestrator.finalize().await?;
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
            let phase = Phase {
                index: 0,
                metrics: end_phase.metrics,
                start_token: PhaseToken::Start,
                end_token: PhaseToken::End,
                timestamp,
                duration_ms: command_duration_ms,
                start_token_line: None,
                end_token_line: None,
            };
            phases.push(phase);
        }

        debug!("Collected {} sensor phase(s)", phases.len());
        Ok(ProfilerResults {
            timestamp,
            duration_ms: command_duration_ms,
            exit_code,
            phases,
        })
    }

    /// Spawn the configured command and profile it, separating its execution into phases through tokens matching
    /// a configured regular expression.
    ///
    /// The shared pid atomic integer is used to configure the sources supporting pid filtering (e.g. `perf_event`)
    ///
    /// The profiling is composed of several steps:
    ///
    /// - Firstly, the program is spawned and its pid is retrieved.
    /// - The process is immediately stopped using a SIGSTOP signal to configure the sources without introducing a significant overhead.
    /// - The first measure is made and the process is then resume.
    /// - The profiler listens for phase token in the standard output of the profiled process and make a measure for every token detected.
    /// - When the program exited, the profiler makes a last measure to compute the last phase metrics.
    ///
    /// After the profiling, results are aggregated into a common structure.
    async fn measure_phases(&mut self, config: &ProfileConfig) -> Result<MeasurePhasesReturnType> {
        debug!("Compiling phase regex");

        let regex = Regex::new(&config.token_pattern).map_err(|err| {
            JouleProfilerError::InvalidPattern(format!("{}: {}", config.token_pattern, err))
        })?;

        let mut sink = create_output_sink(config.stdout_file.as_ref())?;

        debug!("Spawning command: {:?}", config.cmd);
        let mut child = spawn_profiled_command(config)?;
        let pid = child.id().cast_signed();

        pause_prosess(pid)?;
        self.orchestrator.init(pid)?;

        let child_stdout = child
            .stdout
            .take()
            .ok_or(JouleProfilerError::StdOutCaptureFail)?;

        let child_stderr = child
            .stderr
            .take()
            .ok_or(JouleProfilerError::StdErrCaptureFail)?;

        let err_handle: JoinHandle<Result<()>> = spawn_blocking(move || {
            let reader = BufReader::new(child_stderr);

            let mut stderr = BufWriter::new(std::io::stderr().lock());

            for line in reader.lines() {
                writeln!(stderr, "{}", line?)?;
            }

            stderr.flush()?;

            Ok(())
        });

        let reader = BufReader::new(child_stdout);
        let mut detected_phases = Vec::with_capacity(2);

        let begin_timestamp = get_timestamp_micros();
        trace!("Begin timestamp: {begin_timestamp}");

        self.orchestrator.measure().await?;

        resume_process(pid)?;

        detected_phases.push(PhaseInfo::start(begin_timestamp));

        self.detect_and_handle_phases_from_program_output(
            &mut detected_phases,
            reader,
            &regex,
            &mut sink,
        )
        .await?;

        err_handle.await??;

        sink.flush()?;

        let end_timestamp = get_timestamp_micros();
        trace!("End timestamp: {end_timestamp}");

        self.orchestrator.measure().await?;
        self.orchestrator.new_phase().await?;

        let duration_ms = (end_timestamp - begin_timestamp) / 1000;

        detected_phases.push(PhaseInfo::end(end_timestamp));

        let exit_code = wait_for_child_exit(&mut child)?;

        info!("Command finished: duration={duration_ms} ms exit_code={exit_code}");

        Ok((duration_ms, begin_timestamp, exit_code, detected_phases))
    }

    /// Detects and measures the phases from the profiled program standard output.
    ///
    /// When a token in the standard output matches the specified regular expression,
    /// a measure is made and a new phase begins.
    async fn detect_and_handle_phases_from_program_output<R, W>(
        &mut self,
        phases: &mut Vec<PhaseInfo>,
        mut reader: R,
        regex: &Regex,
        sink: &mut W,
    ) -> Result<()>
    where
        R: BufRead,
        W: Write + ?Sized,
    {
        let mut line = String::new();
        let mut line_number: usize = 0;

        loop {
            line.clear();

            let n = match reader.read_line(&mut line) {
                Ok(n) => n,
                Err(e) if e.kind() == ErrorKind::InvalidData => {
                    trace!("Skipping invalid UTF-8 output at line {line_number}");
                    line_number += 1;
                    continue;
                }
                Err(e) => return Err(e.into()),
            };

            if n == 0 {
                break;
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

                self.orchestrator.measure().await?;
                self.orchestrator.new_phase().await?;

                let phase_info = PhaseInfo {
                    token: PhaseToken::Token(token.to_owned()),
                    timestamp: phase_timestamp,
                    line_number: Some(line_number),
                };

                phases.push(phase_info);
            }

            line_number += 1;
        }

        Ok(())
    }
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
    let mut command = process::Command::new(&cmd[0]);
    if cmd.len() > 1 {
        command.args(&cmd[1..]);
    }

    if geteuid() == 0 && !use_root {
        let username =
            std::env::var("SUDO_USER").map_err(|_| JouleProfilerError::CannotRetrieveSudoUser)?;
        let uid = get_uid_from_username(&username)?;
        command.uid(uid);
    }

    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());

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
/// SAFETY
///
/// - 'pid' must refer to a valid process identifier obtained from
///   '`std::process::Child::id()`' immediately after spawning.
/// - The process must be owned by the current user (we only signal
///   child processes we created).
///
/// Returns [`JouleProfilerError::ProcessControlFailed`] if the
/// signal delivery fails.
fn pause_prosess(pid: i32) -> Result<()> {
    signal(pid, libc::SIGSTOP)
}

/// Sends `SIGCONT` to resume a previously paused process.
///
/// SAFETY
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
    use crate::orchestrator::SourceOrchestrator;
    use crate::phase::PhaseToken;
    use crate::profiler::{
        create_output_sink, phase_token_in_line, spawn_profiled_command, wait_for_child_exit,
    };
    use crate::sensor::Sensors;
    use crate::source::MetricReader;
    use crate::types::Metrics;
    use crate::{JouleProfiler, JouleProfilerError};
    use mockall::mock;
    use regex::Regex;
    use std::fs;
    use std::io::{BufReader, Cursor, Read};
    use tempfile::TempDir;

    fn joule_profiler() -> JouleProfiler {
        JouleProfiler {
            orchestrator: SourceOrchestrator::default(),
            sources: Vec::new(),
        }
    }

    fn create_test_config(cmd: Vec<String>) -> ProfileConfig {
        ProfileConfig {
            cmd,
            token_pattern: "__PHASE__".to_string(),
            stdout_file: None,
            use_root: false,
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

            async fn init(&mut self, pid: i32) -> Result<(), MockError>;
            async fn join(&mut self) -> Result<(), MockError>;
            async fn measure(&mut self) -> Result<(), MockError>;
            async fn retrieve(&mut self) -> Result<(), MockError>;
            fn get_sensors(&self) -> Result<Sensors, MockError>;
            fn to_metrics(&self, v: ()) -> Result<Metrics, MockError>;
            fn get_name() -> &'static str;
        }
    }

    #[tokio::test]
    async fn detect_multiple_phases() {
        let mut profiler = joule_profiler();
        let regex = Regex::new("__[A-Z0-9_]+__").unwrap();
        let cursor = Cursor::new("__PHASE1__\n__PHASE2__\n__PHASE3__");
        let reader = BufReader::new(cursor);
        let mut phases = Vec::new();
        let mut sink: Vec<u8> = Vec::new();

        profiler
            .detect_and_handle_phases_from_program_output(&mut phases, reader, &regex, &mut sink)
            .await
            .unwrap();

        assert_eq!(3, phases.len());
        assert_eq!(PhaseToken::Token("__PHASE1__".to_string()), phases[0].token);
        assert_eq!(PhaseToken::Token("__PHASE2__".to_string()), phases[1].token);
        assert_eq!(PhaseToken::Token("__PHASE3__".to_string()), phases[2].token);
    }

    #[tokio::test]
    async fn detect_no_phases() {
        let mut profiler = joule_profiler();
        let regex = Regex::new("__[A-Z0-9_]+__").unwrap();
        let cursor = Cursor::new("hello\nworld\nno phases here");
        let reader = BufReader::new(cursor);
        let mut phases = Vec::new();
        let mut sink: Vec<u8> = Vec::new();
        profiler
            .detect_and_handle_phases_from_program_output(&mut phases, reader, &regex, &mut sink)
            .await
            .unwrap();

        assert!(phases.is_empty());
    }

    #[tokio::test]
    async fn detect_empty_output() {
        let mut profiler = joule_profiler();
        let regex = Regex::new("__PHASE__").unwrap();
        let cursor = Cursor::new("");
        let reader = BufReader::new(cursor);
        let mut phases = Vec::new();
        let mut sink: Vec<u8> = Vec::new();

        profiler
            .detect_and_handle_phases_from_program_output(&mut phases, reader, &regex, &mut sink)
            .await
            .unwrap();

        assert_eq!(phases.len(), 0);
    }

    #[tokio::test]
    async fn detect_phase_in_middle_of_line() {
        let mut profiler = joule_profiler();
        let regex = Regex::new("__PHASE[0-9]+__").unwrap();
        let cursor = Cursor::new("start __PHASE1__ end");
        let reader = BufReader::new(cursor);
        let mut phases = Vec::new();
        let mut sink: Vec<u8> = Vec::new();

        profiler
            .detect_and_handle_phases_from_program_output(&mut phases, reader, &regex, &mut sink)
            .await
            .unwrap();

        assert_eq!(phases.len(), 1);
        assert_eq!(phases[0].token, PhaseToken::Token("__PHASE1__".to_string()));
        assert_eq!(phases[0].line_number, Some(0));
    }

    #[tokio::test]
    async fn detect_correct_line_numbers() {
        let mut profiler = joule_profiler();
        let regex = Regex::new("__PHASE[0-9]+__").unwrap();
        let cursor = Cursor::new("a\nb\n__PHASE1__\nc\n__PHASE2__");
        let reader = BufReader::new(cursor);
        let mut phases = Vec::new();
        let mut sink: Vec<u8> = Vec::new();

        profiler
            .detect_and_handle_phases_from_program_output(&mut phases, reader, &regex, &mut sink)
            .await
            .unwrap();

        assert_eq!(phases.len(), 2);
        assert_eq!(phases[0].line_number, Some(2));
        assert_eq!(phases[1].line_number, Some(4));
    }

    #[tokio::test]
    async fn writes_stdout_to_file() {
        use std::fs;
        use tempfile::NamedTempFile;

        let mut profiler = joule_profiler();
        let regex = Regex::new("__PHASE__").unwrap();
        let cursor = Cursor::new("hello\n__PHASE__\nworld");
        let reader = BufReader::new(cursor);

        let mut temp_file = NamedTempFile::new().unwrap();
        let mut phases = Vec::new();

        profiler
            .detect_and_handle_phases_from_program_output(
                &mut phases,
                reader,
                &regex,
                temp_file.as_file_mut(),
            )
            .await
            .unwrap();

        let content = fs::read_to_string(temp_file.path()).unwrap();
        assert!(content.contains("hello"));
        assert!(content.contains("__PHASE__"));
        assert!(content.contains("world"));
    }

    #[tokio::test]
    async fn skips_invalid_utf8_lines() {
        let mut profiler = joule_profiler();
        let regex = Regex::new("__PHASE__").unwrap();

        let bytes = vec![
            0xff, 0xfe, b'\n', b'_', b'_', b'P', b'H', b'A', b'S', b'E', b'_', b'_',
        ];
        let cursor = Cursor::new(bytes);
        let reader = BufReader::new(cursor);
        let mut phases = Vec::new();
        let mut sink: Vec<u8> = Vec::new();

        profiler
            .detect_and_handle_phases_from_program_output(&mut phases, reader, &regex, &mut sink)
            .await
            .unwrap();

        assert_eq!(phases.len(), 1);
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
}
