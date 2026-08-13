use std::{
    env,
    fs::{self, File, OpenOptions, Permissions},
    os::unix::fs::{MetadataExt, PermissionsExt, chown},
    path::{Path, PathBuf},
};

use crate::{sys::is_root, util::time::get_timestamp_micros};

/// Get the absolute path of a file.
pub fn get_absolute_path(filename: &str) -> Result<String, std::io::Error> {
    let path = PathBuf::from(filename);
    let absolute_path = if path.is_absolute() {
        path
    } else {
        env::current_dir()?.join(path)
    };

    Ok(absolute_path.display().to_string())
}

/// Generates a default filename for results data.
pub fn default_results_filename(ext: &str) -> String {
    format!("data{}.{}", get_timestamp_micros(), ext)
}

const URW_GRW_OR_PERMS: u32 = 0o664;

/// Creates a file at `path`, ensuring it is owned by the parent directory's owner.
///
/// When running as root, the file ownership is set to match the parent directory
/// (via `chown`) so the file isn't accidentally left owned by root.
pub fn create_file_with_user_permissions(path: &str) -> std::io::Result<File> {
    let file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(path)?;
    file.set_permissions(Permissions::from_mode(URW_GRW_OR_PERMS))?;

    if is_root() {
        let parent = match Path::new(path).parent() {
            Some(parent) if !parent.as_os_str().is_empty() => parent,
            _ => Path::new("."),
        };
        let meta = fs::metadata(parent)?;
        chown(path, Some(meta.uid()), Some(meta.gid()))?;
    }

    Ok(file)
}
