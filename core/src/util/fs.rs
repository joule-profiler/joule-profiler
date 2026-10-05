use std::fs::{self, File};
use std::io;
use std::os::unix::fs::{MetadataExt, chown};
use std::path::Path;

use crate::util::{sys::is_root, time::get_timestamp_micros};

/// A default results filename, such as `data1712345678.json`.
pub fn default_results_filename(ext: &str) -> String {
    format!("data{}.{}", get_timestamp_micros(), ext)
}

/// Creates a results file. Under `sudo`, it is given to the owner of its directory.
pub fn create_file(path: &Path) -> io::Result<File> {
    let file = File::create(path)?;

    if is_root() {
        let directory = match path.parent() {
            Some(parent) if !parent.as_os_str().is_empty() => parent,
            _ => Path::new("."),
        };
        let owner = fs::metadata(directory)?;
        chown(path, Some(owner.uid()), Some(owner.gid()))?;
    }

    Ok(file)
}
