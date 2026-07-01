// use std::ffi::{CStr, CString};

// use crate::JouleProfilerError;

// /// Sends a signal to a process.
// ///
// /// SAFETY
// ///
// /// - 'pid' must refer to a valid process identifier.
// /// - Calling `libc::kill` is unsafe because it performs a raw syscall using the Linux FFI.
// ///
// /// Race Condition
// ///
// /// The target process may terminate before the
// /// signal is made. In that case, the system call will fail and
// /// an error will be returned and the signal name will
// /// be retrieved for better error handling.
// ///
// /// Errors
// ///
// /// Returns [`JouleProfilerError::ProcessControlFailed`] if the
// /// signal could not be delivered (e.g., invalid PID or already exited).
// pub fn signal(pid: i32, sig: i32) -> Result<(), JouleProfilerError> {
//     if unsafe { libc::kill(pid, sig) } != 0 {
//         let sig_name = unsafe {
//             let ptr = libc::strsignal(sig);
//             if ptr.is_null() {
//                 "unknown signal".to_string()
//             } else {
//                 CStr::from_ptr(ptr).to_string_lossy().to_string()
//             }
//         };
//         Err(JouleProfilerError::ProcessControlFailed(format!(
//             "Cannot send signal {sig_name} to process {pid}"
//         )))
//     } else {
//         Ok(())
//     }
// }

// /// Gets the effective user id of the current process.
// ///
// /// SAFETY
// ///
// /// - Calling `libc::geteuid` is unsafe because it performs a raw syscall using the Linux FFI.
// pub fn geteuid() -> u32 {
//     unsafe { libc::geteuid() }
// }

// /// Gets the current process' user id from its username.
// ///
// /// SAFETY
// ///
// /// - Calling `libc::getpwnam` is unsafe because it uses the Linux FFI.
// pub fn get_uid_from_username(username: &str) -> Result<u32, JouleProfilerError> {
//     let cname =
//         CString::new(username).map_err(|_| JouleProfilerError::CannotRetrieveCurrentUserId)?;
//     let passwd = unsafe { libc::getpwnam(cname.as_ptr()) };

//     if passwd.is_null() {
//         return Err(JouleProfilerError::CannotRetrieveCurrentUserId);
//     }

//     Ok(unsafe { (*passwd).pw_uid })
// }
