use std::io::{self, BufRead, BufReader, Read as _, Write};
use std::os::fd::AsRawFd;
use std::os::unix::net::UnixStream;
// use std::os::unix::process::CommandExt;
// use std::process::{Child, Command, Stdio};
use std::process::Command;

use crate::{DECLARED, IpcError, Read, Received, Request, STARTED};

/// The program's end of a session.
pub struct Session {
    control: UnixStream,
}

impl Session {
    /// Starts the profiler with `command`, which must call [`crate::serve_spawned`], and waits
    /// until it measures.
    pub fn spawn(mut command: Command, config: &str) -> Result<(Self, Results), IpcError> {
        let (control, theirs) = UnixStream::pair()?;
        let (results, written) = io::pipe()?;
        let descriptors = [theirs.as_raw_fd(), written.as_raw_fd()];

        // fork + exec version
        // command
        //     .env(crate::DESCRIPTORS, format!("{},{}", descriptors[0], descriptors[1]))
        //     .stdin(Stdio::null());
        // // SAFETY: `fcntl` is async-signal-safe, as needed between fork and exec. Clearing
        // // `FD_CLOEXEC` there, rather than here, keeps the descriptors from leaking into a
        // // process another thread starts at the same time.
        // unsafe {
        //     command.pre_exec(move || {
        //         for descriptor in descriptors {
        //             if libc::fcntl(descriptor, libc::F_SETFD, 0) == -1 {
        //                 return Err(io::Error::last_os_error());
        //             }
        //         }
        //         Ok(())
        //     });
        // }
        // let mut spawned = command.spawn()?;
        let mut spawned = posix::spawn(&mut command, descriptors)?;
        drop((theirs, written));

        let mut session = Self { control };
        let mut results = Results::new(BufReader::new(results));
        let started = session
            .send(&Request::Configure(config.to_owned()))
            .and_then(|_| reap(&mut spawned))
            .and_then(|_| session.answer(STARTED));

        match started {
            Ok(()) => Ok((session, results)),
            Err(error) => match results.failure() {
                IpcError::Closed => Err(error),
                failure => Err(failure),
            },
        }
    }

    #[cfg(test)]
    pub(crate) fn connect(
        control: UnixStream,
        results: impl BufRead + Send + 'static,
    ) -> Result<(Self, Results), IpcError> {
        let mut session = Self { control };
        let mut results = Results::new(results);

        match session.answer(STARTED) {
            Ok(()) => Ok((session, results)),
            Err(_) => Err(results.failure()),
        }
    }

    /// Returns once the profiler has woken its sensors.
    pub fn phase(&mut self, name: &str) -> Result<(), IpcError> {
        self.send(&Request::Phase(name.to_owned()))?;
        self.answer(DECLARED)
    }

    pub fn finish(mut self, exit_code: i32) -> Result<(), IpcError> {
        self.send(&Request::End(exit_code))
    }

    fn send(&mut self, request: &Request) -> Result<(), IpcError> {
        let mut line = serde_json::to_vec(request).map_err(|_| IpcError::Protocol)?;
        line.push(b'\n');
        self.control.write_all(&line).map_err(|_| IpcError::Closed)
    }

    fn answer(&mut self, expected: u8) -> Result<(), IpcError> {
        let mut answer = [0];
        match self.control.read_exact(&mut answer) {
            Ok(()) if answer[0] == expected => Ok(()),
            Ok(()) => Err(IpcError::Protocol),
            Err(_) => Err(IpcError::Closed),
        }
    }
}

/// The schema, the phases, then the summary or an error.
pub struct Results {
    reader: Box<dyn BufRead + Send>,
    line: String,
    over: bool,
}

impl Results {
    fn new(reader: impl BufRead + Send + 'static) -> Self {
        Self {
            reader: Box::new(reader),
            line: String::new(),
            over: false,
        }
    }

    pub fn failure(&mut self) -> IpcError {
        self.find_map(Result::err).unwrap_or(IpcError::Closed)
    }
}

impl Iterator for Results {
    type Item = Result<Received, IpcError>;

    fn next(&mut self) -> Option<Self::Item> {
        while !self.over {
            self.line.clear();
            match self.reader.read_line(&mut self.line) {
                Ok(0) => {
                    self.over = true;
                    return Some(Err(IpcError::Closed));
                }
                Ok(_) => {}
                Err(error) => {
                    self.over = true;
                    return Some(Err(error.into()));
                }
            }

            match serde_json::from_str::<Read>(&self.line) {
                Ok(Read::Schema(schema)) => return Some(Ok(Received::Schema(schema))),
                Ok(Read::Phase(phase)) => return Some(Ok(Received::Phase(phase))),
                Ok(Read::Summary(summary)) => {
                    self.over = true;
                    return Some(Ok(Received::Summary(summary)));
                }
                Ok(Read::Error(message)) => {
                    self.over = true;
                    return Some(Err(IpcError::Profiler(message)));
                }
                Err(error) => log::debug!("not a result, skipped ({error}): {:?}", self.line),
            }
        }
        None
    }
}

/// `ECHILD`: already reaped, as when the program ignores `SIGCHLD`.
// fn reap(spawned: &mut Child) -> Result<(), IpcError> {
fn reap(spawned: &mut posix::Spawned) -> Result<(), IpcError> {
    match spawned.wait() {
        Ok(status) if status.success() => Ok(()),
        Ok(status) => Err(IpcError::Spawned(status)),
        Err(error) if error.raw_os_error() == Some(libc::ECHILD) => Ok(()),
        Err(error) => Err(error.into()),
    }
}

/// Unlike fork, `posix_spawn` does not copy the page tables of the program.
mod posix {
    use std::collections::HashMap;
    use std::env;
    use std::ffi::{CString, NulError, OsStr, OsString};
    use std::io;
    use std::iter;
    use std::mem::MaybeUninit;
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::process::ExitStatusExt;
    use std::process::{Command, ExitStatus};
    use std::ptr;

    use crate::DESCRIPTORS;

    /// Where the profiler finds the descriptors. A duplicate onto another number loses
    /// `FD_CLOEXEC` with any libc; onto itself, only since glibc 2.29.
    const TARGETS: [RawFd; 2] = [3, 4];

    pub(super) struct Spawned(libc::pid_t);

    impl Spawned {
        pub(super) fn wait(&mut self) -> io::Result<ExitStatus> {
            let mut status = 0;
            loop {
                // SAFETY: `waitpid` only writes `status`.
                if unsafe { libc::waitpid(self.0, &raw mut status, 0) } != -1 {
                    return Ok(ExitStatus::from_raw(status));
                }
                let error = io::Error::last_os_error();
                if error.kind() != io::ErrorKind::Interrupted {
                    return Err(error);
                }
            }
        }
    }

    /// Passes `descriptors` at [`TARGETS`], with `/dev/null` as standard input.
    pub(super) fn spawn(command: &mut Command, descriptors: [RawFd; 2]) -> io::Result<Spawned> {
        command.env(DESCRIPTORS, format!("{},{}", TARGETS[0], TARGETS[1]));
        let sources = descriptors
            .into_iter()
            .map(above_targets)
            .collect::<io::Result<Vec<_>>>()?;
        let program = CString::new(command.get_program().as_bytes())?;
        let arguments = iter::once(command.get_program())
            .chain(command.get_args())
            .map(|argument| CString::new(argument.as_bytes()))
            .collect::<Result<Vec<_>, _>>()?;

        let mut variables: HashMap<OsString, OsString> = env::vars_os().collect();
        for (key, value) in command.get_envs() {
            match value {
                Some(value) => variables.insert(key.to_owned(), value.to_owned()),
                None => variables.remove(key),
            };
        }
        let environment = variables
            .iter()
            .map(|(key, value)| variable(key, value))
            .collect::<Result<Vec<_>, _>>()?;
        let (argv, envp) = (pointers(&arguments), pointers(&environment));

        let mut actions = MaybeUninit::<libc::posix_spawn_file_actions_t>::uninit();
        let actions = actions.as_mut_ptr();
        let mut pid = 0;
        // SAFETY: `actions` is initialised before use and destroyed once; the strings outlive it.
        unsafe {
            check(libc::posix_spawn_file_actions_init(actions))?;
            let spawned = sources
                .iter()
                .zip(TARGETS)
                .try_for_each(|(source, target)| {
                    check(libc::posix_spawn_file_actions_adddup2(
                        actions,
                        source.as_raw_fd(),
                        target,
                    ))
                })
                .and_then(|()| {
                    check(libc::posix_spawn_file_actions_addopen(
                        actions,
                        libc::STDIN_FILENO,
                        c"/dev/null".as_ptr(),
                        libc::O_RDONLY,
                        0,
                    ))
                })
                .and_then(|()| {
                    check(libc::posix_spawnp(
                        &raw mut pid,
                        program.as_ptr(),
                        actions,
                        ptr::null(),
                        argv.as_ptr(),
                        envp.as_ptr(),
                    ))
                });
            libc::posix_spawn_file_actions_destroy(actions);
            spawned.map(|()| Spawned(pid))
        }
    }

    /// A duplicate numbered above the targets, so that no duplication overwrites another source.
    fn above_targets(descriptor: RawFd) -> io::Result<OwnedFd> {
        // SAFETY: `fcntl` reads no memory, and the duplicate belongs to nothing else.
        match unsafe { libc::fcntl(descriptor, libc::F_DUPFD_CLOEXEC, TARGETS[1] + 1) } {
            -1 => Err(io::Error::last_os_error()),
            duplicate => Ok(unsafe { OwnedFd::from_raw_fd(duplicate) }),
        }
    }

    fn variable(key: &OsStr, value: &OsStr) -> Result<CString, NulError> {
        CString::new([key.as_bytes(), b"=", value.as_bytes()].concat())
    }

    fn pointers(strings: &[CString]) -> Vec<*mut libc::c_char> {
        strings
            .iter()
            .map(|string| string.as_ptr().cast_mut())
            .chain(iter::once(ptr::null_mut()))
            .collect()
    }

    fn check(code: libc::c_int) -> io::Result<()> {
        match code {
            0 => Ok(()),
            code => Err(io::Error::from_raw_os_error(code)),
        }
    }
}
