//! Low-level helpers to read cgroup v2 pseudo-files.

use std::collections::HashMap;
use std::fs;
use std::path::Path;

use crate::error::CgroupError;
pub fn read_u64_opt(path: &Path) -> Result<Option<u64>, CgroupError> {
    let raw = fs::read_to_string(path).map_err(|e| CgroupError::Io {
        path: path.display().to_string(),
        source: e,
    })?;
    let trimmed = raw.trim();
    if trimmed == "max" {
        return Ok(None);
    }
    trimmed
        .parse::<u64>()
        .map(Some)
        .map_err(|_| CgroupError::Parse {
            path: path.display().to_string(),
            expected: "u64 or \"max\"",
            got: trimmed.to_string(),
        })
}

pub fn read_flat_keyed(path: &Path) -> Result<HashMap<String, u64>, CgroupError> {
    let raw = fs::read_to_string(path).map_err(|e| CgroupError::Io {
        path: path.display().to_string(),
        source: e,
    })?;

    let mut map = HashMap::new();
    for line in raw.lines() {
        let mut parts = line.splitn(2, ' ');
        if let (Some(key), Some(val_str)) = (parts.next(), parts.next()) {
            if let Ok(v) = val_str.trim().parse::<u64>() {
                map.insert(key.to_string(), v);
            }
        }
    }
    Ok(map)
}

pub fn read_io_stat(path: &Path) -> Result<(u64, u64), CgroupError> {
    let raw = fs::read_to_string(path).map_err(|e| CgroupError::Io {
        path: path.display().to_string(),
        source: e,
    })?;

    let (mut rbytes, mut wbytes) = (0u64, 0u64);
    for line in raw.lines() {
        for field in line.split_whitespace().skip(1) {
            if let Some(v) = field.strip_prefix("rbytes=") {
                rbytes += v.parse::<u64>().unwrap_or(0);
            } else if let Some(v) = field.strip_prefix("wbytes=") {
                wbytes += v.parse::<u64>().unwrap_or(0);
            }
        }
    }
    Ok((rbytes, wbytes))
}