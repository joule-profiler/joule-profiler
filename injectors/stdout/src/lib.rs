use std::io::{BufRead, BufReader, ErrorKind};
use std::process::{Child, ChildStdout, Command, Stdio};

use joule_profiler_core::injector::Injector;
use joule_profiler_core::time::get_timestamp_micros;
use joule_profiler_core::types::{InitContext, PhaseStartInfo};
use regex::Regex;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum StdOutInjectorError {
    #[error("failed to spawn child process: {0}")]
    CannotSpawnCommand(#[source] std::io::Error),

    #[error("failed to resume child process: {0}")]
    CannotResumeProcess(#[source] std::io::Error),

    #[error("failed to read child stdout: {0}")]
    Read(#[source] std::io::Error),

    #[error("invalid token pattern: {0}")]
    RegexError(#[from] regex::Error),

    #[error("stdout not piped")]
    CannotReadStdout,

    #[error(transparent)]
    IoError(#[from] std::io::Error),
}

struct ChildHandle {
    child: Child,
    reader: BufReader<ChildStdout>,
    pid: i32,
}

pub struct StdOutInjector {
    regex: Regex,
    cmd: Vec<String>,
    child_handle: Option<ChildHandle>,
    phase_index: usize,
}

impl StdOutInjector {
    pub fn new(cmd: Vec<String>, token_pattern: &str) -> Result<Self, StdOutInjectorError> {
        Ok(Self {
            regex: Regex::new(token_pattern)?,
            cmd,
            child_handle: None,
            phase_index: 0,
        })
    }

    fn spawn_process(&mut self) -> std::io::Result<Child> {
        let mut command = Command::new(&self.cmd[0]);
        if self.cmd.len() > 1 {
            command.args(&self.cmd[1..]);
        }
        command.stdout(Stdio::piped());
        command.stderr(Stdio::inherit());

        let child = command.spawn()?;
        Ok(child)
    }

    fn pause_process(pid: i32) -> std::io::Result<()> {
        signal(pid, libc::SIGSTOP)
    }

    fn resume_process(pid: i32) -> std::io::Result<()> {
        signal(pid, libc::SIGCONT)
    }

    fn next_phase(&mut self, token: String) -> PhaseStartInfo {
        let phase_index = self.phase_index;
        self.phase_index += 1;
        PhaseStartInfo {
            phase_index,
            token,
            timestamp: get_timestamp_micros(),
        }
    }
}

impl Injector for StdOutInjector {
    type Error = StdOutInjectorError;

    async fn init(&mut self) -> Result<InitContext, Self::Error> {
        let mut child = self
            .spawn_process()
            .map_err(StdOutInjectorError::CannotSpawnCommand)?;
        let pid = child.id().cast_signed();
        Self::pause_process(pid)?;
        let reader = BufReader::new(
            child
                .stdout
                .take()
                .ok_or(StdOutInjectorError::CannotReadStdout)?,
        );
        self.child_handle = Some(ChildHandle { child, reader, pid });
        Ok(InitContext { pid })
    }

    async fn resume(&mut self) -> Result<(), Self::Error> {
        if let Some(handle) = &self.child_handle {
            Self::resume_process(handle.pid).map_err(StdOutInjectorError::CannotResumeProcess)?;
        }
        Ok(())
    }

    async fn next(&mut self) -> Result<Option<PhaseStartInfo>, Self::Error> {
        if let Some(handle) = &mut self.child_handle {
            let mut line = String::new();
            Ok(loop {
                line.clear();
                let n = match handle.reader.read_line(&mut line) {
                    Ok(n) => n,
                    Err(e) if e.kind() == ErrorKind::InvalidData => continue,
                    Err(e) => return Err(StdOutInjectorError::Read(e)),
                };
                if n == 0 {
                    self.child_handle = None;
                    break None;
                }
                let trimmed = line.trim_end_matches(['\n', '\r']);
                if self.regex.is_match(trimmed) {
                    break Some(self.next_phase(trimmed.to_string()));
                }
            })
        } else {
            Ok(None)
        }
    }

    async fn abort(&mut self) -> Result<(), Self::Error> {
        if let Some(mut handle) = self.child_handle.take() {
            handle.child.kill()?;
        }
        Ok(())
    }
}

fn signal(pid: i32, signal: i32) -> std::io::Result<()> {
    let result = unsafe { libc::kill(pid, signal) };
    if result != 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}
