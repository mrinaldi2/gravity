//! Filesystem helpers for the migration: sizes, free space, and moving a
//! directory out of the way.

use std::path::{Path, PathBuf};

use anyhow::Context;

#[cfg(windows)]
use super::steps;

/// Bytes under `path`, following no links.
pub fn tree_size(path: &Path) -> u64 {
    let Ok(meta) = path.symlink_metadata() else {
        return 0;
    };
    if !meta.is_dir() {
        return meta.len();
    }
    std::fs::read_dir(path)
        .map(|entries| {
            entries
                .flatten()
                .map(|entry| tree_size(&entry.path()))
                .sum()
        })
        .unwrap_or(0)
}

/// Windows refuses to rename a directory while any process has a file or
/// its working directory under it, and lists no culprit. Renaming the home
/// aside and back before anything else runs fails early, with a hint,
/// instead of after the backup.
#[cfg(windows)]
pub fn probe_move(from: &Path) -> anyhow::Result<()> {
    let mut aside = from.as_os_str().to_owned();
    aside.push(".migrate-probe");
    let aside = PathBuf::from(aside);
    steps::move_home(from, &aside).with_context(|| {
        format!(
            "{} is in use: close terminals, editors and Explorer windows open there, \
             and any claude, node or codex process left running from it, then retry",
            from.display()
        )
    })?;
    steps::move_home(&aside, from).with_context(|| {
        format!(
            "moving {} back to {}; rename it back by hand",
            aside.display(),
            from.display()
        )
    })
}

/// Free bytes on the filesystem holding `path`, where that can be asked.
#[cfg(unix)]
pub fn free_bytes(path: &Path) -> Option<u64> {
    use std::os::unix::ffi::OsStrExt;
    let c = std::ffi::CString::new(path.as_os_str().as_bytes()).ok()?;
    let mut stat: libc::statvfs = unsafe { std::mem::zeroed() };
    // SAFETY: `c` is a valid NUL-terminated path and `stat` a writable struct.
    let rc = unsafe { libc::statvfs(c.as_ptr(), &mut stat) };
    (rc == 0).then(|| stat.f_bavail as u64 * stat.f_frsize as u64)
}

#[cfg(windows)]
pub fn free_bytes(_path: &Path) -> Option<u64> {
    None
}

/// Renames `dir` to an unused `<dir>-stray-<timestamp>` beside it.
pub fn set_aside(dir: &Path) -> anyhow::Result<PathBuf> {
    let stamp = chrono::Utc::now().format("%Y%m%d-%H%M%S");
    let mut aside = dir.as_os_str().to_owned();
    aside.push(format!("-stray-{stamp}"));
    let aside = PathBuf::from(aside);
    std::fs::rename(dir, &aside)
        .with_context(|| format!("moving {} aside to {}", dir.display(), aside.display()))?;
    Ok(aside)
}
