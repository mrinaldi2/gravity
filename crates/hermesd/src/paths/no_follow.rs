//! Writing into a bot's folders without following a link the bot planted
//! (H-182, CE-025).
//!
//! A bot owns its folders: it can `ln -s ~/.claude .claude` in its workspace,
//! or leave `mcp.json.tmp` as a link to any file. The daemon then wrote the
//! bot's hook settings through the link, over the owner's own
//! `~/.claude/settings.json`. `own_workspace` (H-171) vets the folder itself;
//! this vets every part below it.
//!
//! On Unix every component from `base` down is opened as a directory with
//! `O_NOFOLLOW`, and the file is written as a fresh temp (`O_CREAT|O_EXCL|
//! O_NOFOLLOW`) in that directory, then renamed over the name; a rename
//! replaces a link at the name instead of writing through it. Nothing is
//! resolved by path between the checks and the write. Elsewhere each
//! component is checked with `symlink_metadata` first. Either way a link
//! anywhere is refused with [`LinkRefused`], which callers log and skip.

use std::path::{Component, Path};

/// A component below `base` is a link (or not a plain directory).
#[derive(Debug)]
pub struct LinkRefused(pub String);

impl std::fmt::Display for LinkRefused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "refusing to write {} in a bot's folder: a part of it is a link, not a plain folder \
             or file (H-182)",
            self.0
        )
    }
}

impl std::error::Error for LinkRefused {}

/// The names from `rel`, each a plain name: no root, no `.`/`..`.
fn names(rel: &Path) -> anyhow::Result<Vec<&std::ffi::OsStr>> {
    rel.components()
        .map(|c| match c {
            Component::Normal(name) => Ok(name),
            _ => Err(anyhow::anyhow!(
                "not a plain relative path: {}",
                rel.display()
            )),
        })
        .collect()
}

/// `base` made when nothing is there yet. It is the daemon's own path (a
/// bot folder in its home), so creating it follows nothing the bot made; an
/// existing `base` that is a link is still refused when it is opened.
fn ensure_base(base: &Path) -> anyhow::Result<()> {
    if std::fs::symlink_metadata(base).is_err() {
        std::fs::create_dir_all(base)?;
    }
    Ok(())
}

/// `bytes` at `base/rel`, its folders made as needed, atomically.
pub fn write(base: &Path, rel: &Path, bytes: &[u8]) -> anyhow::Result<()> {
    ensure_base(base)?;
    let parts = names(rel)?;
    let (file, dirs) = parts
        .split_last()
        .ok_or_else(|| anyhow::anyhow!("no file name"))?;
    imp::write(base, dirs, file, bytes, rel)
}

/// [`write`] of pretty JSON.
pub fn write_json(base: &Path, rel: &Path, value: &serde_json::Value) -> anyhow::Result<()> {
    write(base, rel, serde_json::to_string_pretty(value)?.as_bytes())
}

/// `bytes` at `base/rel` only when nothing is there: not a file, not a
/// link. False when something was.
pub fn create_new(base: &Path, rel: &Path, bytes: &[u8]) -> anyhow::Result<bool> {
    ensure_base(base)?;
    let parts = names(rel)?;
    let (file, dirs) = parts
        .split_last()
        .ok_or_else(|| anyhow::anyhow!("no file name"))?;
    imp::create_new(base, dirs, file, bytes, rel)
}

/// The text at `base/rel`, read through no link: `None` when it is missing,
/// a link, or not text. For a file the daemon edits and writes back, so a
/// link to the owner's files can't copy their content into a bot's folder.
pub fn read(base: &Path, rel: &Path) -> Option<String> {
    let parts = names(rel).ok()?;
    let (file, dirs) = parts.split_last()?;
    imp::read(base, dirs, file, rel).ok()
}

/// The folders `base/rel`, each made if missing, none a link.
pub fn create_dirs(base: &Path, rel: &Path) -> anyhow::Result<()> {
    ensure_base(base)?;
    let parts = names(rel)?;
    imp::create_dirs(base, &parts, rel)
}

#[cfg(unix)]
mod imp {
    use std::ffi::{CString, OsStr};
    use std::io::Write;
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
    use std::os::unix::ffi::OsStrExt;
    use std::path::Path;

    use super::LinkRefused;

    fn c(name: &OsStr) -> anyhow::Result<CString> {
        Ok(CString::new(name.as_bytes())?)
    }

    /// A link where a folder or file should be.
    fn refused(error: std::io::Error, rel: &Path) -> anyhow::Error {
        match error.raw_os_error() {
            Some(libc::ELOOP | libc::ENOTDIR) => LinkRefused(rel.display().to_string()).into(),
            _ => anyhow::Error::new(error).context(format!("writing {}", rel.display())),
        }
    }

    const DIR_FLAGS: libc::c_int =
        libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC;

    /// `base` itself, refused when it is a link.
    fn open_base(base: &Path, rel: &Path) -> anyhow::Result<OwnedFd> {
        let path = c(base.as_os_str())?;
        // SAFETY: a valid NUL-terminated path; the fd is owned on success.
        let fd = unsafe { libc::open(path.as_ptr(), DIR_FLAGS) };
        if fd < 0 {
            return Err(refused(std::io::Error::last_os_error(), rel));
        }
        // SAFETY: `fd` is a fresh descriptor nothing else owns.
        Ok(unsafe { OwnedFd::from_raw_fd(fd) })
    }

    /// `name` in `dir` as a folder, made when missing and `make`.
    fn open_dir(dir: &OwnedFd, name: &OsStr, make: bool, rel: &Path) -> anyhow::Result<OwnedFd> {
        let name = c(name)?;
        for _ in 0..2 {
            // SAFETY: `dir` is an open directory and `name` a valid C string.
            let fd = unsafe { libc::openat(dir.as_raw_fd(), name.as_ptr(), DIR_FLAGS) };
            if fd >= 0 {
                // SAFETY: as in `open_base`.
                return Ok(unsafe { OwnedFd::from_raw_fd(fd) });
            }
            let error = std::io::Error::last_os_error();
            if !(make && error.raw_os_error() == Some(libc::ENOENT)) {
                return Err(refused(error, rel));
            }
            // SAFETY: as above; a race that makes it first is caught by EEXIST.
            if unsafe { libc::mkdirat(dir.as_raw_fd(), name.as_ptr(), 0o755) } != 0 {
                let error = std::io::Error::last_os_error();
                if error.raw_os_error() != Some(libc::EEXIST) {
                    return Err(refused(error, rel));
                }
            }
        }
        Err(LinkRefused(rel.display().to_string()).into())
    }

    fn walk(base: &Path, dirs: &[&OsStr], rel: &Path) -> anyhow::Result<OwnedFd> {
        let mut dir = open_base(base, rel)?;
        for name in dirs {
            dir = open_dir(&dir, name, true, rel)?;
        }
        Ok(dir)
    }

    /// A new file `name` in `dir`; never through a link, never over one.
    fn create(dir: &OwnedFd, name: &CString, rel: &Path) -> anyhow::Result<Option<std::fs::File>> {
        let flags =
            libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC;
        // SAFETY: as in `open_dir`; the mode is passed as the variadic third argument.
        let fd =
            unsafe { libc::openat(dir.as_raw_fd(), name.as_ptr(), flags, 0o644 as libc::c_uint) };
        if fd >= 0 {
            // SAFETY: as in `open_base`.
            return Ok(Some(unsafe { std::fs::File::from_raw_fd(fd) }));
        }
        let error = std::io::Error::last_os_error();
        match error.raw_os_error() {
            Some(libc::EEXIST) => Ok(None),
            _ => Err(refused(error, rel)),
        }
    }

    pub fn write(
        base: &Path,
        dirs: &[&OsStr],
        file: &OsStr,
        bytes: &[u8],
        rel: &Path,
    ) -> anyhow::Result<()> {
        let dir = walk(base, dirs, rel)?;
        let target = c(file)?;
        let tmp = c(OsStr::new(&format!(
            ".{}.tmp-{}",
            file.to_string_lossy(),
            uuid::Uuid::new_v4()
        )))?;
        let mut out = create(&dir, &tmp, rel)?
            .ok_or_else(|| anyhow::anyhow!("a fresh temp name was taken: {}", rel.display()))?;
        let written = out.write_all(bytes).and_then(|()| out.sync_all());
        // SAFETY: both names are valid C strings in the same open directory.
        let renamed = written.and_then(|()| {
            let fd = dir.as_raw_fd();
            match unsafe { libc::renameat(fd, tmp.as_ptr(), fd, target.as_ptr()) } {
                0 => Ok(()),
                _ => Err(std::io::Error::last_os_error()),
            }
        });
        if let Err(error) = renamed {
            // SAFETY: as above.
            unsafe { libc::unlinkat(dir.as_raw_fd(), tmp.as_ptr(), 0) };
            return Err(anyhow::Error::new(error).context(format!("writing {}", rel.display())));
        }
        Ok(())
    }

    pub fn create_new(
        base: &Path,
        dirs: &[&OsStr],
        file: &OsStr,
        bytes: &[u8],
        rel: &Path,
    ) -> anyhow::Result<bool> {
        let dir = walk(base, dirs, rel)?;
        match create(&dir, &c(file)?, rel)? {
            Some(mut out) => {
                out.write_all(bytes)?;
                Ok(true)
            }
            None => Ok(false),
        }
    }

    pub fn create_dirs(base: &Path, dirs: &[&OsStr], rel: &Path) -> anyhow::Result<()> {
        walk(base, dirs, rel).map(drop)
    }

    pub fn read(base: &Path, dirs: &[&OsStr], file: &OsStr, rel: &Path) -> anyhow::Result<String> {
        let mut dir = open_base(base, rel)?;
        for name in dirs {
            dir = open_dir(&dir, name, false, rel)?;
        }
        let name = c(file)?;
        let flags = libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_CLOEXEC;
        // SAFETY: as in `open_dir`.
        let fd = unsafe { libc::openat(dir.as_raw_fd(), name.as_ptr(), flags) };
        if fd < 0 {
            return Err(refused(std::io::Error::last_os_error(), rel));
        }
        // SAFETY: as in `open_base`.
        let mut file = unsafe { std::fs::File::from_raw_fd(fd) };
        let mut text = String::new();
        std::io::Read::read_to_string(&mut file, &mut text)?;
        Ok(text)
    }
}

#[cfg(not(unix))]
mod imp {
    use std::ffi::OsStr;
    use std::io::Write;
    use std::path::{Path, PathBuf};

    use super::LinkRefused;

    /// Each folder from `base` down: made if missing, refused when a link
    /// (a junction or symlink is a reparse point `symlink_metadata` reports).
    fn walk(base: &Path, dirs: &[&OsStr], rel: &Path) -> anyhow::Result<PathBuf> {
        let mut at = base.to_path_buf();
        for (i, name) in std::iter::once(OsStr::new(""))
            .chain(dirs.iter().copied())
            .enumerate()
        {
            if i > 0 {
                at.push(name);
                if !at.exists() && std::fs::symlink_metadata(&at).is_err() {
                    std::fs::create_dir(&at)?;
                }
            }
            let meta = std::fs::symlink_metadata(&at)?;
            if meta.file_type().is_symlink() || !meta.is_dir() {
                return Err(LinkRefused(rel.display().to_string()).into());
            }
        }
        Ok(at)
    }

    /// A link the bot left at the file name, removed so the rename replaces
    /// it as on Unix: the link itself, never its target. A folder link (a
    /// junction or directory symlink) needs `remove_dir`; `remove_file` on
    /// it fails with "Access is denied". Refused when it can't be removed.
    fn remove_link(target: &Path, rel: &Path) -> anyhow::Result<()> {
        let Ok(meta) = std::fs::symlink_metadata(target) else {
            return Ok(());
        };
        if !meta.file_type().is_symlink() {
            return Ok(());
        }
        #[cfg(windows)]
        let folder = std::os::windows::fs::FileTypeExt::is_symlink_dir(&meta.file_type());
        #[cfg(not(windows))]
        let folder = false;
        let removed = if folder {
            std::fs::remove_dir(target)
        } else {
            std::fs::remove_file(target)
        };
        removed.map_err(|_| LinkRefused(rel.display().to_string()).into())
    }

    fn create(path: &Path) -> std::io::Result<std::fs::File> {
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
    }

    pub fn write(
        base: &Path,
        dirs: &[&OsStr],
        file: &OsStr,
        bytes: &[u8],
        rel: &Path,
    ) -> anyhow::Result<()> {
        let dir = walk(base, dirs, rel)?;
        let target = dir.join(file);
        remove_link(&target, rel)?;
        let tmp = dir.join(format!(
            ".{}.tmp-{}",
            file.to_string_lossy(),
            uuid::Uuid::new_v4()
        ));
        let mut out = create(&tmp)?;
        out.write_all(bytes)?;
        out.sync_all()?;
        drop(out);
        replace(&tmp, &target).inspect_err(|_| {
            let _ = std::fs::remove_file(&tmp);
        })?;
        Ok(())
    }

    /// Waits between attempts to rename over a file that is briefly held.
    const HELD_RETRIES_MS: [u64; 6] = [10, 20, 40, 80, 160, 320];

    /// `tmp` renamed over `target`. On Windows a file another process has
    /// open, as an antivirus scan holds one just written, can't be renamed
    /// or replaced for a moment: "Access is denied" or a sharing violation.
    /// Every start rewrites a bot's settings this way, and a start that
    /// failed on it waited out a crash backoff (H-188); a short retry rides
    /// the scan out.
    fn replace(tmp: &Path, target: &Path) -> std::io::Result<()> {
        let mut waits = HELD_RETRIES_MS.iter();
        loop {
            match std::fs::rename(tmp, target) {
                Err(e) if held(&e) => match waits.next() {
                    Some(ms) => std::thread::sleep(std::time::Duration::from_millis(*ms)),
                    None => return Err(e),
                },
                result => return result,
            }
        }
    }

    /// ERROR_ACCESS_DENIED, ERROR_SHARING_VIOLATION or ERROR_LOCK_VIOLATION.
    fn held(e: &std::io::Error) -> bool {
        cfg!(windows) && matches!(e.raw_os_error(), Some(5 | 32 | 33))
    }

    pub fn create_new(
        base: &Path,
        dirs: &[&OsStr],
        file: &OsStr,
        bytes: &[u8],
        rel: &Path,
    ) -> anyhow::Result<bool> {
        let target = walk(base, dirs, rel)?.join(file);
        if std::fs::symlink_metadata(&target).is_ok() {
            return Ok(false);
        }
        create(&target)?.write_all(bytes)?;
        Ok(true)
    }

    pub fn create_dirs(base: &Path, dirs: &[&OsStr], rel: &Path) -> anyhow::Result<()> {
        walk(base, dirs, rel).map(drop)
    }

    pub fn read(base: &Path, dirs: &[&OsStr], file: &OsStr, rel: &Path) -> anyhow::Result<String> {
        let path = walk(base, dirs, rel)?.join(file);
        if std::fs::symlink_metadata(&path)?.file_type().is_symlink() {
            return Err(LinkRefused(rel.display().to_string()).into());
        }
        Ok(std::fs::read_to_string(path)?)
    }
}

#[cfg(test)]
#[path = "no_follow_tests.rs"]
mod tests;
