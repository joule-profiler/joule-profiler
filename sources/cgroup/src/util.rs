use std::{collections::HashMap, fs, path::Path};

use crate::{Result, error::CgroupError};

pub fn read_u64(path: &Path) -> Result<u64> {
    let raw = fs::read_to_string(path).map_err(|e| CgroupError::IoPath {
        path: path.to_path_buf(),
        source: e,
    })?;
    let trimmed = raw.trim();
    trimmed.parse::<u64>().map_err(|err| CgroupError::Parse {
        path: path.to_path_buf(),
        source: err,
    })
}

pub fn read_u64_opt(path: &Path) -> Result<Option<u64>> {
    if fs::exists(path)? {
        Ok(Some(read_u64(path)?))
    } else {
        Ok(None)
    }
}

pub fn read_flat_keyed_file(path: &Path) -> Result<HashMap<String, u64>> {
    let raw = fs::read_to_string(path).map_err(|err| CgroupError::IoPath {
        path: path.to_owned(),
        source: err,
    })?;

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

pub fn read_io_stat(path: &Path) -> Result<(Option<u64>, Option<u64>)> {
    let raw = fs::read_to_string(path).map_err(|e| CgroupError::IoPath {
        path: path.to_path_buf(),
        source: e,
    })?;

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
