use std::{fs, io};

const PMUS_PATH: &str = "/sys/bus/event_source/devices";
const PARANOID_LEVEL_FILE_PATH: &str = "/proc/sys/kernel/perf_event_paranoid";

/// Returns the PMUs of the machine, sorted by name.
pub fn read_pmus_list() -> io::Result<Vec<String>> {
    let mut pmus = fs::read_dir(PMUS_PATH)?
        .map(|entry| entry.map(|entry| entry.file_name().to_string_lossy().into_owned()))
        .collect::<io::Result<Vec<_>>>()?;
    pmus.sort_unstable();
    Ok(pmus)
}

/// Returns the value of `kernel.perf_event_paranoid`.
pub fn read_paranoid_level() -> io::Result<i64> {
    fs::read_to_string(PARANOID_LEVEL_FILE_PATH)?
        .trim()
        .parse()
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}
