//! Runs a command and turns the tokens it prints into phase boundaries.

use std::io::{self, BufRead, BufReader, BufWriter, Write};
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::path::PathBuf;
use std::process::{Child, ChildStdout, Command, Stdio};

use joule_profiler_core::injector::{Injector, PhaseToken, StopHandle, Target};
use joule_profiler_core::util::{fs::create_file, sys::is_root};
use regex::bytes::Regex;
use thiserror::Error;

pub const DEFAULT_PATTERN: &str = "__[A-Z0-9_]+__";

#[derive(Debug, Error)]
pub enum StdoutError {
    #[error("the command to profile is empty")]
    EmptyCommand,

    #[error("invalid token pattern: {0}")]
    Pattern(regex::Error),

    #[error("command not found: `{0}`")]
    NotFound(String),

    #[error("could not start `{0}`: {1}")]
    Spawn(String, io::Error),

    #[error(
        "running as root, but SUDO_UID and SUDO_GID do not say who asked: set them, or use \
         `use_root` to run the command as root"
    )]
    NoInvokingUser,

    #[error("could not create {0}: {1}")]
    OutputFile(PathBuf, io::Error),

    #[error("the command has not been started")]
    NotStarted,

    #[error(transparent)]
    Io(#[from] io::Error),
}

pub struct StdoutInjector {
    command: Vec<String>,
    pattern: Regex,
    use_root: bool,
    output_file: Option<PathBuf>,
    child: Option<Child>,
    stdout: Option<BufReader<ChildStdout>>,
    output: Box<dyn Write + Send>,
    line: Vec<u8>,
    line_number: usize,
}

impl StdoutInjector {
    pub fn new(command: Vec<String>, pattern: &str) -> Result<Self, StdoutError> {
        if command.is_empty() {
            return Err(StdoutError::EmptyCommand);
        }

        Ok(Self {
            command,
            pattern: Regex::new(pattern).map_err(StdoutError::Pattern)?,
            use_root: false,
            output_file: None,
            child: None,
            stdout: None,
            output: Box::new(io::sink()),
            line: Vec::new(),
            line_number: 0,
        })
    }

    /// Under `sudo`, the command runs as the user who called it unless this is set.
    pub fn use_root(mut self, use_root: bool) -> Self {
        self.use_root = use_root;
        self
    }

    /// Writes what the command prints to this file instead of the standard output.
    pub fn output_file(mut self, path: Option<PathBuf>) -> Self {
        self.output_file = path;
        self
    }
}

impl Injector for StdoutInjector {
    type Error = StdoutError;

    fn start(&mut self) -> Result<Target, StdoutError> {
        self.output = match &self.output_file {
            Some(path) => Box::new(BufWriter::new(
                create_file(path).map_err(|e| StdoutError::OutputFile(path.clone(), e))?,
            )),
            None => Box::new(BufWriter::new(io::stdout())),
        };
        self.line_number = 0;

        let (program, arguments) = self
            .command
            .split_first()
            .ok_or(StdoutError::EmptyCommand)?;
        let mut command = Command::new(program);
        command.args(arguments).stdout(Stdio::piped());

        if is_root() && !self.use_root {
            let (uid, gid) = get_current_user().ok_or(StdoutError::NoInvokingUser)?;
            command.uid(uid).gid(gid);
        }

        let mut child = command.spawn().map_err(|error| match error.kind() {
            io::ErrorKind::NotFound => StdoutError::NotFound(program.clone()),
            _ => StdoutError::Spawn(program.clone(), error),
        })?;
        let pid = get_pid(&child);

        if let Err(error) = signal(pid, libc::SIGSTOP) {
            if let Err(cleanup) = child.kill().and_then(|()| child.wait()) {
                log::warn!("the command could not be killed: {cleanup}");
            }
            return Err(error.into());
        }

        self.stdout = child.stdout.take().map(BufReader::new);
        self.child = Some(child);
        Ok(Target { pid })
    }

    fn stop_handle(&self) -> StopHandle {
        let pid = self.child.as_ref().map(get_pid);
        Box::new(move || match pid {
            Some(pid) => Ok(signal(pid, libc::SIGKILL)?),
            None => Ok(()),
        })
    }

    fn resume(&mut self) -> Result<(), StdoutError> {
        let child = self.child.as_ref().ok_or(StdoutError::NotStarted)?;
        Ok(signal(get_pid(child), libc::SIGCONT)?)
    }

    fn next_phase(&mut self) -> Result<Option<PhaseToken>, StdoutError> {
        let stdout = self.stdout.as_mut().ok_or(StdoutError::NotStarted)?;

        loop {
            if stdout.buffer().is_empty() {
                self.output.flush()?;
            }

            self.line.clear();
            if stdout.read_until(b'\n', &mut self.line)? == 0 {
                self.output.flush()?;
                return Ok(None);
            }
            self.line_number += 1;
            self.output.write_all(&self.line)?;

            if let Some(token) = self.pattern.find(&self.line) {
                return Ok(Some(PhaseToken {
                    text: String::from_utf8_lossy(token.as_bytes()).into_owned(),
                    line: Some(self.line_number),
                }));
            }
        }
    }

    fn wait(&mut self) -> Result<Option<i32>, StdoutError> {
        self.stdout = None;
        let mut child = self.child.take().ok_or(StdoutError::NotStarted)?;
        let status = child.wait()?;
        self.output.flush()?;

        Ok(status
            .code()
            .or_else(|| status.signal().map(|signal| 128 + signal)))
    }
}

impl Drop for StdoutInjector {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take()
            && let Err(error) = child.kill().and_then(|()| child.wait())
        {
            log::warn!("the command could not be killed: {error}");
        }
    }
}

fn get_current_user() -> Option<(u32, u32)> {
    let uid = std::env::var("SUDO_UID").ok()?.parse().ok()?;
    let gid = std::env::var("SUDO_GID").ok()?.parse().ok()?;
    Some((uid, gid))
}

fn get_pid(child: &Child) -> i32 {
    child.id().cast_signed()
}

fn signal(pid: i32, signal: libc::c_int) -> io::Result<()> {
    // SAFETY: `kill` has no memory-safety precondition.
    if unsafe { libc::kill(pid, signal) } == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shell(script: &str) -> StdoutInjector {
        StdoutInjector::new(
            vec!["sh".into(), "-c".into(), script.into()],
            DEFAULT_PATTERN,
        )
        .unwrap()
    }

    fn tokens(injector: &mut StdoutInjector) -> Vec<PhaseToken> {
        injector.start().unwrap();
        injector.resume().unwrap();
        std::iter::from_fn(|| injector.next_phase().unwrap()).collect()
    }

    fn token(text: &str, line: usize) -> PhaseToken {
        PhaseToken {
            text: text.into(),
            line: Some(line),
        }
    }

    #[test]
    fn tokens_are_reported_with_their_line_and_every_line_is_passed_on() {
        let directory = tempfile::tempdir().unwrap();
        let output = directory.path().join("stdout.txt");
        let mut injector = shell("echo hello; echo 'step __A__ done'; printf 'x\\n__B__'")
            .output_file(Some(output.clone()));

        let found = tokens(&mut injector);
        let exit_code = injector.wait().unwrap();

        assert_eq!(found, [token("__A__", 2), token("__B__", 4)]);
        assert_eq!(exit_code, Some(0));
        assert_eq!(
            std::fs::read_to_string(output).unwrap(),
            "hello\nstep __A__ done\nx\n__B__"
        );
    }

    #[test]
    fn a_line_that_is_not_utf8_is_passed_on_and_still_matched() {
        let directory = tempfile::tempdir().unwrap();
        let output = directory.path().join("stdout.txt");
        let mut injector = shell(r"printf '\377 __A__\n'").output_file(Some(output.clone()));

        assert_eq!(tokens(&mut injector), [token("__A__", 1)]);
        injector.wait().unwrap();
        assert_eq!(std::fs::read(output).unwrap(), b"\xff __A__\n");
    }

    #[test]
    fn the_exit_code_is_reported_and_a_signal_reads_as_128_plus_its_number() {
        let mut exited = shell("exit 3");
        tokens(&mut exited);
        assert_eq!(exited.wait().unwrap(), Some(3));

        let mut killed = shell("kill -9 $$");
        tokens(&mut killed);
        assert_eq!(killed.wait().unwrap(), Some(128 + 9));
    }

    #[test]
    fn the_stop_handle_kills_the_command() {
        let mut injector = shell("echo __A__; exec sleep 60");
        injector.start().unwrap();
        injector.resume().unwrap();
        assert!(injector.next_phase().unwrap().is_some());

        injector.stop_handle()().unwrap();

        assert!(injector.next_phase().unwrap().is_none());
        assert_eq!(injector.wait().unwrap(), Some(128 + 9));
    }

    #[test]
    fn a_missing_command_is_named() {
        let mut injector =
            StdoutInjector::new(vec!["no-such-command".into()], DEFAULT_PATTERN).unwrap();

        assert!(matches!(
            injector.start(),
            Err(StdoutError::NotFound(name)) if name == "no-such-command"
        ));
    }

    #[test]
    fn an_empty_command_is_refused() {
        assert!(matches!(
            StdoutInjector::new(Vec::new(), DEFAULT_PATTERN),
            Err(StdoutError::EmptyCommand)
        ));
    }
}
