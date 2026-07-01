//! This module contains the core logic of profiling and is the entrypoint of Joule Profiler.
//!
//! It provides the main [`JouleProfiler`] struct, orchestrating
//! the execution of commands, collecting metrics from various sources (e.g. RAPL, `perf_event`, NVML, etc.),
//! and aggregate them into a clean common structure.

pub mod error;

use crate::config::ProfileConfig;
use crate::exporter::{Exporter, ExporterWrapper};
use crate::injector::{Injector, InjectorError};
use crate::orchestrator::Orchestrator;
use crate::profiler::types::{ProfilerResults, Result};
use crate::source::{MetricSource, MetricSourceRuntime, MetricSourceWrapper};
use crate::types::Sensors;
use crate::util::time::get_timestamp_micros;
pub use error::JouleProfilerError;
use log::{debug, info, trace};

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
/// ```ignore
/// // `my_exporter`/`my_injector` are your own `Exporter`/`Injector` implementations.
/// use joule_profiler_core::{JouleProfiler, config::ProfileConfig};
///
/// # tokio_test::block_on(async {
/// let mut profiler = JouleProfiler::new();
/// profiler.with_exporter(my_exporter);
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
/// let results = profiler.profile(&config, my_injector).await.unwrap();
/// # });
/// ```
pub struct JouleProfiler {
    /// The different metric sources.
    sources: Vec<Box<dyn MetricSourceWrapper>>,

    /// The configured exporter, taken by `profile()` for the duration of the run
    /// and handed back afterwards so it can be reused across multiple profiling runs.
    exporter: Option<Box<dyn ExporterWrapper>>,
}

impl Default for JouleProfiler {
    fn default() -> Self {
        Self::new()
    }
}

impl JouleProfiler {
    pub fn new() -> Self {
        Self {
            sources: Vec::new(),
            exporter: None,
        }
    }

    pub fn with_exporter<E: Exporter>(&mut self, exporter: E) -> &mut Self {
        self.exporter = Some(exporter.into());
        self
    }

    pub fn add_source<S>(&mut self, config: S::Config) -> Result<()>
    where
        S: MetricSource,
    {
        debug!("Registering additional metric source: {}", S::get_name());
        trace!("MetricReader type: {}", std::any::type_name::<S>());
        let source = MetricSourceRuntime::<S>::boxed(config)?;
        self.sources.push(source);
        Ok(())
    }

    /// List the sensors of the provided sources.
    pub fn list_sensors(&mut self) -> Result<Sensors> {
        debug!("Listing sensors from {} source(s)", self.sources.len());

        let sensors: Sensors = self
            .sources
            .iter()
            .enumerate()
            .map(|(i, source)| {
                trace!("Querying sensors from source {i}");
                source
                    .list_sensors()
                    .map_err(JouleProfilerError::MetricSourceError)
            })
            .collect::<Result<Vec<Sensors>>>()?
            .into_iter()
            .flatten()
            .collect();

        info!("Discovered {} sensor(s)", sensors.len());
        Ok(sensors)
    }

    pub async fn profile(
        &mut self,
        config: &ProfileConfig,
        mut injector: impl Injector,
    ) -> Result<ProfilerResults> {
        info!("Running phase-based profiling");
        debug!("Phase regex: {}", config.token_pattern);

        let exporter = self
            .exporter
            .take()
            .ok_or(JouleProfilerError::NoExporterConfigured)?;

        let orchestrator = Orchestrator::new(exporter, std::mem::take(&mut self.sources))
            .pre_init()
            .await?;
        let begin_timestamp = get_timestamp_micros();
        let ctx = injector.init().await.map_err(InjectorError::boxed)?;

        let mut orchestrator = orchestrator.run(ctx).await?;

        injector.resume().await.map_err(InjectorError::boxed)?;

        orchestrator.measure("START".to_owned()).await?;

        let mut run_error = None;
        loop {
            let phase_start = match injector.next().await {
                Ok(Some(phase_start)) => phase_start,
                Ok(None) => break,
                Err(e) => {
                    run_error = Some(InjectorError::boxed(e).into());
                    break;
                }
            };

            if let Err(e) = orchestrator.measure(phase_start.token).await {
                run_error = Some(JouleProfilerError::OrchestratorError(e));
                break;
            }
        }

        orchestrator.measure("END".to_owned()).await?;
        let end_timestamp = get_timestamp_micros();

        match orchestrator.join().await {
            Ok((sources, exporter)) => {
                self.sources = sources;
                self.exporter = Some(exporter);
            }
            Err(e) => {
                run_error = Some(e.into());
                if let Err(e) = injector.abort().await.map_err(InjectorError::boxed) {
                    run_error = Some(e.into());
                }
            }
        }

        if let Some(err) = run_error {
            return Err(err);
        }

        Ok(ProfilerResults {
            timestamp: begin_timestamp,
            duration_ms: end_timestamp - begin_timestamp,
        })
    }
}

// #[cfg(test)]
// mod tests {
//     use crate::config::ProfileConfig;
//     use crate::orchestrator::SourceOrchestrator;
//     use crate::phase::PhaseToken;
//     use crate::profiler::{
//         create_output_sink, phase_token_in_line, spawn_profiled_command, wait_for_child_exit,
//     };
//     use crate::sensor::Sensors;
//     use crate::source::MetricReader;
//     use crate::types::Metrics;
//     use crate::{JouleProfiler, JouleProfilerError};
//     use mockall::mock;
//     use regex::Regex;
//     use std::fs;
//     use std::io::{BufReader, Cursor, Read};
//     use std::time::Duration;
//     use tempfile::TempDir;

//     fn joule_profiler() -> JouleProfiler {
//         JouleProfiler {
//             orchestrator: SourceOrchestrator::default(),
//             sources: Vec::new(),
//         }
//     }

//     fn create_test_config(cmd: Vec<String>) -> ProfileConfig {
//         ProfileConfig {
//             cmd,
//             token_pattern: "__PHASE__".to_string(),
//             stdout_file: None,
//             use_root: false,
//             init_timeout: Duration::from_secs(1),
//         }
//     }

//     #[derive(Debug)]
//     pub struct MockError;

//     impl std::fmt::Display for MockError {
//         fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
//             write!(f, "mock error")
//         }
//     }

//     impl std::error::Error for MockError {}

//     mock! {
//         pub MetricReader {}

//         impl MetricReader for MetricReader {
//             type Type = ();
//             type Error = MockError;

//             async fn init(&mut self, pid: i32) -> Result<(), MockError>;
//             async fn join(&mut self) -> Result<(), MockError>;
//             async fn measure(&mut self) -> Result<(), MockError>;
//             async fn retrieve(&mut self) -> Result<(), MockError>;
//             fn get_sensors(&self) -> Result<Sensors, MockError>;
//             fn to_metrics(&self, v: ()) -> Result<Metrics, MockError>;
//             fn get_name() -> &'static str;
//         }
//     }

//     #[tokio::test]
//     async fn detect_multiple_phases() {
//         let mut profiler = joule_profiler();
//         let regex = Regex::new("__[A-Z0-9_]+__").unwrap();
//         let cursor = Cursor::new("__PHASE1__\n__PHASE2__\n__PHASE3__");
//         let reader = BufReader::new(cursor);
//         let mut phases = Vec::new();
//         let mut sink: Vec<u8> = Vec::new();

//         profiler
//             .detect_and_handle_phases_from_program_output(&mut phases, reader, &regex, &mut sink)
//             .await
//             .unwrap();

//         assert_eq!(3, phases.len());
//         assert_eq!(PhaseToken::Token("__PHASE1__".to_string()), phases[0].token);
//         assert_eq!(PhaseToken::Token("__PHASE2__".to_string()), phases[1].token);
//         assert_eq!(PhaseToken::Token("__PHASE3__".to_string()), phases[2].token);
//     }

//     #[tokio::test]
//     async fn detect_no_phases() {
//         let mut profiler = joule_profiler();
//         let regex = Regex::new("__[A-Z0-9_]+__").unwrap();
//         let cursor = Cursor::new("hello\nworld\nno phases here");
//         let reader = BufReader::new(cursor);
//         let mut phases = Vec::new();
//         let mut sink: Vec<u8> = Vec::new();
//         profiler
//             .detect_and_handle_phases_from_program_output(&mut phases, reader, &regex, &mut sink)
//             .await
//             .unwrap();

//         assert!(phases.is_empty());
//     }

//     #[tokio::test]
//     async fn detect_empty_output() {
//         let mut profiler = joule_profiler();
//         let regex = Regex::new("__PHASE__").unwrap();
//         let cursor = Cursor::new("");
//         let reader = BufReader::new(cursor);
//         let mut phases = Vec::new();
//         let mut sink: Vec<u8> = Vec::new();

//         profiler
//             .detect_and_handle_phases_from_program_output(&mut phases, reader, &regex, &mut sink)
//             .await
//             .unwrap();

//         assert_eq!(phases.len(), 0);
//     }

//     #[tokio::test]
//     async fn detect_phase_in_middle_of_line() {
//         let mut profiler = joule_profiler();
//         let regex = Regex::new("__PHASE[0-9]+__").unwrap();
//         let cursor = Cursor::new("start __PHASE1__ end");
//         let reader = BufReader::new(cursor);
//         let mut phases = Vec::new();
//         let mut sink: Vec<u8> = Vec::new();

//         profiler
//             .detect_and_handle_phases_from_program_output(&mut phases, reader, &regex, &mut sink)
//             .await
//             .unwrap();

//         assert_eq!(phases.len(), 1);
//         assert_eq!(phases[0].token, PhaseToken::Token("__PHASE1__".to_string()));
//         assert_eq!(phases[0].line_number, Some(0));
//     }

//     #[tokio::test]
//     async fn detect_correct_line_numbers() {
//         let mut profiler = joule_profiler();
//         let regex = Regex::new("__PHASE[0-9]+__").unwrap();
//         let cursor = Cursor::new("a\nb\n__PHASE1__\nc\n__PHASE2__");
//         let reader = BufReader::new(cursor);
//         let mut phases = Vec::new();
//         let mut sink: Vec<u8> = Vec::new();

//         profiler
//             .detect_and_handle_phases_from_program_output(&mut phases, reader, &regex, &mut sink)
//             .await
//             .unwrap();

//         assert_eq!(phases.len(), 2);
//         assert_eq!(phases[0].line_number, Some(2));
//         assert_eq!(phases[1].line_number, Some(4));
//     }

//     #[tokio::test]
//     async fn writes_stdout_to_file() {
//         use std::fs;
//         use tempfile::NamedTempFile;

//         let mut profiler = joule_profiler();
//         let regex = Regex::new("__PHASE__").unwrap();
//         let cursor = Cursor::new("hello\n__PHASE__\nworld");
//         let reader = BufReader::new(cursor);

//         let mut temp_file = NamedTempFile::new().unwrap();
//         let mut phases = Vec::new();

//         profiler
//             .detect_and_handle_phases_from_program_output(
//                 &mut phases,
//                 reader,
//                 &regex,
//                 temp_file.as_file_mut(),
//             )
//             .await
//             .unwrap();

//         let content = fs::read_to_string(temp_file.path()).unwrap();
//         assert!(content.contains("hello"));
//         assert!(content.contains("__PHASE__"));
//         assert!(content.contains("world"));
//     }

//     #[tokio::test]
//     async fn skips_invalid_utf8_lines() {
//         let mut profiler = joule_profiler();
//         let regex = Regex::new("__PHASE__").unwrap();

//         let bytes = vec![
//             0xff, 0xfe, b'\n', b'_', b'_', b'P', b'H', b'A', b'S', b'E', b'_', b'_',
//         ];
//         let cursor = Cursor::new(bytes);
//         let reader = BufReader::new(cursor);
//         let mut phases = Vec::new();
//         let mut sink: Vec<u8> = Vec::new();

//         profiler
//             .detect_and_handle_phases_from_program_output(&mut phases, reader, &regex, &mut sink)
//             .await
//             .unwrap();

//         assert_eq!(phases.len(), 1);
//     }

//     #[test]
//     fn phase_token_in_line_returns_none_when_no_match() {
//         let regex = Regex::new("X").unwrap();
//         assert_eq!(phase_token_in_line(&regex, "abc"), None);
//     }

//     #[test]
//     fn phase_token_in_line_returns_some_when_match_exists() {
//         let regex = Regex::new("X").unwrap();
//         assert_eq!(phase_token_in_line(&regex, "aXc"), Some("X"));
//     }

//     #[test]
//     fn phase_token_in_line_returns_first_match_only() {
//         let regex = Regex::new("X").unwrap();
//         assert_eq!(phase_token_in_line(&regex, "XX"), Some("X"));
//     }

//     #[test]
//     fn phase_token_in_line_does_not_trim_or_modify_input() {
//         let regex = Regex::new("X").unwrap();
//         assert_eq!(phase_token_in_line(&regex, "  X  "), Some("X"));
//     }

//     #[test]
//     fn phase_token_in_line_returns_slice_from_input() {
//         let regex = Regex::new("X").unwrap();
//         let line = String::from("aXc");

//         let token = phase_token_in_line(&regex, &line).unwrap();

//         let line_ptr = line.as_ptr() as usize;
//         let tok_ptr = token.as_ptr() as usize;
//         assert!(tok_ptr >= line_ptr && tok_ptr < line_ptr + line.len());
//     }

//     #[test]
//     fn phase_token_in_line_empty_line_returns_none() {
//         let regex = Regex::new("X").unwrap();
//         assert_eq!(phase_token_in_line(&regex, ""), None);
//     }

//     #[test]
//     fn phase_token_in_line_full_line_match() {
//         let regex = Regex::new(".*").unwrap();
//         assert_eq!(phase_token_in_line(&regex, "abc"), Some("abc"));
//     }

//     #[tokio::test]
//     async fn profile_invalid_regex_returns_error() {
//         let mut profiler = joule_profiler();
//         let config = ProfileConfig {
//             cmd: vec!["echo".to_string()],
//             token_pattern: "[[invalid[[[regex[[".to_string(),
//             stdout_file: None,
//             use_root: false,
//             init_timeout: Duration::from_secs(1),
//         };
//         profiler.add_source(MockMetricReader::new());
//         let result = profiler.profile(&config).await;
//         assert!(matches!(result, Err(JouleProfilerError::InvalidPattern(_))));
//     }

//     #[test]
//     fn create_output_sink_none_returns_stdout_sink() {
//         assert!(create_output_sink(None).is_ok());
//     }

//     #[test]
//     fn create_output_sink_with_path_creates_file() {
//         let dir = TempDir::new().unwrap();
//         let path = dir.path().join("out.txt").to_str().unwrap().to_owned();

//         let result = create_output_sink(Some(&path));
//         assert!(result.is_ok());
//         assert!(fs::metadata(&path).is_ok());
//     }

//     #[test]
//     fn create_output_sink_invalid_path_returns_error() {
//         let result = create_output_sink(Some(&"/nonexistent/dir/out.txt".to_string()));
//         assert!(result.is_err());
//         assert!(matches!(
//             result.err().unwrap(),
//             JouleProfilerError::OutputFileCreationFailed(_)
//         ));
//     }

//     #[test]
//     fn spawn_profiled_command_with_valid_command() {
//         let config = create_test_config(vec!["echo".to_string(), "hello".to_string()]);

//         let result = spawn_profiled_command(&config);

//         assert!(result.is_ok());
//         let mut child = result.unwrap();

//         assert!(child.stdout.is_some());

//         let _ = child.kill();
//         let _ = child.wait();
//     }

//     #[test]
//     fn spawn_profiled_command_with_nonexistent_command() {
//         let config = create_test_config(vec!["mais_t_es_pas_la_mais_t_es_ou".to_string()]);

//         let result = spawn_profiled_command(&config);

//         assert!(result.is_err());
//         match result.unwrap_err() {
//             JouleProfilerError::CommandNotFound(cmd) => {
//                 assert_eq!(cmd, "mais_t_es_pas_la_mais_t_es_ou");
//             }
//             _ => panic!("Expected CommandNotFound error"),
//         }
//     }

//     #[test]
//     fn spawn_profiled_command_with_single_arg() {
//         let config = create_test_config(vec!["echo".to_string()]);

//         let result = spawn_profiled_command(&config);

//         assert!(result.is_ok());
//         let mut child = result.unwrap();
//         let _ = child.kill();
//         let _ = child.wait();
//     }

//     #[test]
//     fn spawn_profiled_command_with_multiple_args() {
//         let config = create_test_config(vec![
//             "echo".to_string(),
//             "help".to_string(),
//             "me".to_string(),
//             "plz".to_string(),
//         ]);

//         let result = spawn_profiled_command(&config);

//         assert!(result.is_ok());
//         let mut child = result.unwrap();

//         let mut output = String::new();
//         if let Some(mut stdout) = child.stdout.take() {
//             stdout.read_to_string(&mut output).unwrap();
//         }

//         assert!(output.contains("help"));
//         assert!(output.contains("me"));
//         assert!(output.contains("plz"));

//         let _ = child.wait();
//     }

//     #[cfg(unix)]
//     #[test]
//     fn spawn_profiled_command_permission_denied() {
//         use std::os::unix::fs::PermissionsExt;

//         let temp_dir = TempDir::new().unwrap();
//         let script_path = temp_dir.path().join("no_exec.sh");

//         fs::write(&script_path, "#!/bin/sh\necho test").unwrap();
//         let mut perms = fs::metadata(&script_path).unwrap().permissions();
//         perms.set_mode(0o644); // rw-r--r--
//         fs::set_permissions(&script_path, perms).unwrap();

//         let config = create_test_config(vec![script_path.to_string_lossy().to_string()]);

//         let result = spawn_profiled_command(&config);

//         assert!(result.is_err());
//         match result.unwrap_err() {
//             JouleProfilerError::CommandExecutionFailed(_) => (),
//             _ => panic!("Expected CommandExecutionFailed error"),
//         }
//     }

//     #[test]
//     fn wait_for_child_exit_zero_on_success() {
//         let config = create_test_config(vec!["true".to_string()]);
//         let mut child = spawn_profiled_command(&config).unwrap();
//         assert_eq!(wait_for_child_exit(&mut child).unwrap(), 0);
//     }

//     #[test]
//     fn wait_for_child_exit_nonzero_on_failure() {
//         let config = create_test_config(vec!["false".to_string()]);
//         let mut child = spawn_profiled_command(&config).unwrap();
//         assert_ne!(wait_for_child_exit(&mut child).unwrap(), 0);
//     }

//     #[test]
//     fn list_sensors_no_sources_returns_empty() {
//         let mut profiler = joule_profiler();
//         let sensors = profiler.list_sensors().unwrap();
//         assert!(sensors.is_empty());
//     }
// }
