use std::{collections::HashMap, fs, io::ErrorKind, path::Path};

use crate::{Result, error::CgroupError};

/// Reads a cgroup file, tagging any I/O failure with the file it happened on.
pub fn read_file(path: &Path) -> Result<String> {
    fs::read_to_string(path).map_err(|err| CgroupError::io(path, err))
}

/// Reads a cgroup file, returning `None` when it does not exist.
///
/// Many interface files are only exposed by some kernel versions or some
/// controller configurations, so a missing file is a valid answer here while
/// any other I/O failure stays an error.
fn read_file_opt(path: &Path) -> Result<Option<String>> {
    match fs::read_to_string(path) {
        Ok(raw) => Ok(Some(raw)),
        Err(err) if err.kind() == ErrorKind::NotFound => Ok(None),
        Err(err) => Err(CgroupError::io(path, err)),
    }
}

/// Reads a `u64` value from a file, returns an error if unable to parse.
fn parse_u64(path: &Path, raw: &str) -> Result<u64> {
    raw.trim().parse::<u64>().map_err(|err| CgroupError::Parse {
        path: path.to_path_buf(),
        source: err,
    })
}

/// Reads an optional `u64` value from a file.
///
/// Return None if the file does not exist, else try to parse the file.
pub fn read_u64_opt(path: &Path) -> Result<Option<u64>> {
    read_file_opt(path)?
        .map(|raw| parse_u64(path, &raw))
        .transpose()
}

/// Returns whether `path` is a directory of a cgroup v2 hierarchy.
///
/// Every cgroup v2 directory, root or not, exposes a read-only
/// `cgroup.controllers` file, which a plain directory never has.
pub fn is_cgroup_dir(path: &Path) -> bool {
    path.join("cgroup.controllers").is_file()
}

/// Reads a space separated list of controller names.
///
/// Used for `cgroup.controllers` (controllers the hierarchy knows about) and
/// `cgroup.subtree_control` (controllers a cgroup enables for its children).
pub fn read_controllers(path: &Path) -> Result<Vec<String>> {
    Ok(read_file(path)?
        .split_whitespace()
        .map(str::to_owned)
        .collect())
}

/// Reads a flat key-value file.
///
/// Expected format:
/// ```text
/// key1 value1
/// key2 value2
/// ```
///
/// Returns a map of parsed metrics.
/// Lines that fail to parse are ignored.
pub fn read_flat_keyed_file(path: &Path) -> Result<HashMap<String, u64>> {
    let raw = read_file(path)?;

    let map = raw
        .lines()
        .filter_map(|line| {
            let mut parts = line.splitn(2, ' ');
            if let (Some(key), Some(val_str)) = (parts.next(), parts.next())
                && let Ok(v) = val_str.trim().parse::<u64>()
            {
                Some((key.to_string(), v))
            } else {
                None
            }
        })
        .collect();

    Ok(map)
}

/// Reads I/O statistics from `io.stat`.
///
/// Parses per-device entries and sums:
/// - `rbytes` (read bytes)
/// - `wbytes` (written bytes)
///
/// Returns `(read_bytes, write_bytes)` if present.
pub fn read_io_stat(path: &Path) -> Result<(Option<u64>, Option<u64>)> {
    let raw = read_file(path)?;

    let mut rbytes = 0;
    let mut wbytes = 0;
    let mut has_r = false;
    let mut has_w = false;

    for line in raw.lines() {
        for field in line.split_whitespace().skip(1) {
            if let Some(v) = field.strip_prefix("rbytes=") {
                if let Ok(val) = v.parse::<u64>() {
                    rbytes += val;
                    has_r = true;
                }
            } else if let Some(v) = field.strip_prefix("wbytes=")
                && let Ok(val) = v.parse::<u64>()
            {
                wbytes += val;
                has_w = true;
            }
        }
    }

    Ok((
        if has_r { Some(rbytes) } else { None },
        if has_w { Some(wbytes) } else { None },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs::File, io::Write, path::PathBuf};

    fn temp_file(name: &str, content: &str) -> PathBuf {
        let mut path = std::env::temp_dir();
        path.push(name);
        let mut file = File::create(&path).unwrap();
        write!(file, "{content}").unwrap();
        path
    }

    #[test]
    fn test_read_u64_ok() {
        let path = temp_file("valid_u64", "42");

        let v = read_u64_opt(&path).unwrap();
        assert_eq!(v, Some(42));

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn test_read_u64_invalid() {
        let path = temp_file("invalid_u64", "NaN");

        let err = read_u64_opt(&path).unwrap_err();
        assert!(matches!(err, CgroupError::Parse { .. }));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn test_read_u64_opt_missing() {
        let mut path = std::env::temp_dir();
        path.push("missing_file");

        let v = read_u64_opt(&path).unwrap();
        assert_eq!(v, None);
    }

    #[test]
    fn test_read_file_reports_the_faulty_path() {
        let mut path = std::env::temp_dir();
        path.push("missing_cgroup_file");

        let err = read_file(&path).unwrap_err();

        assert!(matches!(&err, CgroupError::IoPath { path: p, .. } if *p == path));
        assert!(err.to_string().contains("missing_cgroup_file"));
    }

    #[test]
    fn test_read_controllers() {
        let path = temp_file("test_controllers", "cpuset cpu io memory pids\n");

        let controllers = read_controllers(&path).unwrap();

        assert_eq!(controllers, ["cpuset", "cpu", "io", "memory", "pids"]);

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn test_is_cgroup_dir() {
        let dir = tempfile::tempdir().unwrap();

        assert!(!is_cgroup_dir(dir.path()));

        File::create(dir.path().join("cgroup.controllers")).unwrap();

        assert!(is_cgroup_dir(dir.path()));
    }

    #[test]
    fn test_read_u64_opt_present() {
        let path = temp_file("present_u64", "100");

        let v = read_u64_opt(&path).unwrap();
        assert_eq!(v, Some(100));

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn test_read_flat_keyed_file() {
        let content = "\
foo 10
bar 20
invalid_line
baz not_a_number";

        let path = temp_file("test_flat_keyed", content);

        let map = read_flat_keyed_file(&path).unwrap();
        println!("map {map:?}");
        println!("{content}");

        assert_eq!(map.get("foo"), Some(&10));
        assert_eq!(map.get("bar"), Some(&20));
        assert_eq!(map.get("baz"), None);

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn test_read_io_stat() {
        let content = "\
8:0 rbytes=100 wbytes=50
8:1 rbytes=20 wbytes=30";

        let path = temp_file("test_io_stat", content);

        let (r, w) = read_io_stat(&path).unwrap();

        assert_eq!(r, Some(120));
        assert_eq!(w, Some(80));

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn test_read_io_stat_partial() {
        let content = "\
8:0 rbytes=10
8:1 rbytes=5";

        let path = temp_file("test_io_stat_partial", content);

        let (r, w) = read_io_stat(&path).unwrap();

        assert_eq!(r, Some(15));
        assert_eq!(w, None);

        let _ = std::fs::remove_file(path);
    }
}
