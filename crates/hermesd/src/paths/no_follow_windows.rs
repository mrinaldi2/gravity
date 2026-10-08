//! `no_follow` on Windows, by handle (H-184, CE-026 F1).
//!
//! Each folder is opened relative to its parent's handle (`NtCreateFile` with
//! a `RootDirectory`) and `FILE_OPEN_REPARSE_POINT`, so a junction or symlink
//! is opened as itself and refused, never followed. The file is created as a
//! fresh temp relative to the last folder's handle and renamed by its own
//! handle into that folder (`SetFileInformationByHandle`, `RootDirectory`).
//! No write goes by path after a check: a bot swapping a folder for a
//! junction mid-write changes nothing the daemon already holds open. A
//! rename over a briefly held file is retried (H-188), and before each retry
//! the path is walked again: a folder that became a link, or another folder,
//! is refused rather than written behind.

use std::ffi::OsStr;
use std::fs::File;
use std::io::{Read, Write};
use std::os::windows::fs::OpenOptionsExt;
use std::os::windows::io::OwnedHandle;
use std::path::Path;

use super::LinkRefused;

#[path = "no_follow_win_ffi.rs"]
mod ffi;
use ffi::*;

/// A link (or something that isn't a plain folder or file) where one should be.
fn refused(error: std::io::Error, rel: &Path) -> anyhow::Error {
    match error.raw_os_error() {
        Some(ERROR_DIRECTORY) => LinkRefused(rel.display().to_string()).into(),
        _ => anyhow::Error::new(error).context(format!("writing {}", rel.display())),
    }
}

fn link_refused(rel: &Path) -> anyhow::Error {
    LinkRefused(rel.display().to_string()).into()
}

/// A plain folder: a directory that is no reparse point.
fn plain_dir(handle: OwnedHandle, rel: &Path) -> anyhow::Result<OwnedHandle> {
    let attributes = info(&handle)?.attributes;
    if attributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 || attributes & FILE_ATTRIBUTE_DIRECTORY == 0
    {
        return Err(link_refused(rel));
    }
    Ok(handle)
}

/// `base` itself, refused when it is a link.
fn open_base(base: &Path, rel: &Path) -> anyhow::Result<OwnedHandle> {
    let file = std::fs::OpenOptions::new()
        .access_mode(DIR_ACCESS)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(base)
        .map_err(|e| refused(e, rel))?;
    plain_dir(file.into(), rel)
}

fn walk(base: &Path, dirs: &[&OsStr], make: bool, rel: &Path) -> anyhow::Result<OwnedHandle> {
    let mut dir = open_base(base, rel)?;
    let disposition = if make { FILE_OPEN_IF } else { FILE_OPEN };
    for name in dirs {
        let next = open_at(
            &dir,
            name,
            DIR_ACCESS,
            disposition,
            FILE_DIRECTORY_FILE,
            rel,
        )?
        .map_err(|e| refused(e, rel))?;
        dir = plain_dir(next, rel)?;
    }
    Ok(dir)
}


/// A link the bot left at `name` in `dir`, deleted by handle so the rename
/// replaces it as on Unix: the link itself, never its target.
fn remove_link(dir: &OwnedHandle, name: &OsStr, rel: &Path) -> anyhow::Result<()> {
    let opened = open_at(dir, name, DELETE | FILE_READ_ATTRIBUTES, FILE_OPEN, 0, rel)?;
    let handle = match opened {
        Ok(handle) => handle,
        Err(e) if e.raw_os_error() == Some(ERROR_FILE_NOT_FOUND) => return Ok(()),
        Err(e) => return Err(refused(e, rel)),
    };
    if info(&handle)?.attributes & FILE_ATTRIBUTE_REPARSE_POINT == 0 {
        return Ok(());
    }
    delete(&handle).map_err(|_| link_refused(rel))
}

/// A new file `name` in `dir`, open for writing and deleting; `None` when
/// something (a file or a link) already has the name.
fn create(dir: &OwnedHandle, name: &OsStr, rel: &Path) -> anyhow::Result<Option<File>> {
    let access = FILE_GENERIC_WRITE | DELETE | FILE_READ_ATTRIBUTES;
    match open_at(dir, name, access, FILE_CREATE, FILE_NON_DIRECTORY_FILE, rel)? {
        Ok(handle) => Ok(Some(File::from(handle))),
        Err(e) if e.raw_os_error() == Some(ERROR_FILE_EXISTS) => Ok(None),
        // STATUS_OBJECT_NAME_COLLISION maps to ERROR_ALREADY_EXISTS (183).
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Ok(None),
        Err(e) => Err(refused(e, rel)),
    }
}

pub fn write(
    base: &Path,
    dirs: &[&OsStr],
    file: &OsStr,
    bytes: &[u8],
    rel: &Path,
) -> anyhow::Result<()> {
    let dir = walk(base, dirs, true, rel)?;
    let target = wide(file, rel)?;
    let tmp_name = format!(".{}.tmp-{}", file.to_string_lossy(), uuid::Uuid::new_v4());
    let mut out = create(&dir, OsStr::new(&tmp_name), rel)?
        .ok_or_else(|| anyhow::anyhow!("a fresh temp name was taken: {}", rel.display()))?;
    let handle = OwnedHandle::from(out.try_clone()?);
    let vet = || vet(&dir, base, dirs, file, rel);
    let done = out
        .write_all(bytes)
        .and_then(|()| out.sync_all())
        .map_err(anyhow::Error::from)
        .and_then(|()| remove_link(&dir, file, rel))
        .and_then(|()| replace(&handle, &dir, &target, vet));
    if let Err(error) = done {
        // By handle, so the temp goes even when its folder became a link.
        let _ = delete(&handle);
        return Err(error.context(format!("writing {}", rel.display())));
    }
    Ok(())
}

/// Waits between attempts to rename over a file that is briefly held.
const HELD_RETRIES_MS: [u64; 6] = [10, 20, 40, 80, 160, 320];

/// ERROR_ACCESS_DENIED, ERROR_SHARING_VIOLATION or ERROR_LOCK_VIOLATION.
fn held(e: &std::io::Error) -> bool {
    matches!(e.raw_os_error(), Some(5 | 32 | 33))
}

/// The open `file` renamed over `target` in `dir`. A file another process
/// has open, as an antivirus scan holds one just written, can't be replaced
/// for a moment; every start rewrites a bot's settings this way, so a short
/// retry rides the scan out (H-188). `vet` re-runs the link checks before
/// each retry: a bot holding the file open to force a wait can't swap a
/// link in meanwhile (Architect M1).
fn replace(
    file: &OwnedHandle,
    dir: &OwnedHandle,
    target: &[u16],
    vet: impl Fn() -> anyhow::Result<()>,
) -> anyhow::Result<()> {
    let mut waits = HELD_RETRIES_MS.iter();
    loop {
        match rename_into(file, dir, target) {
            Err(e) if held(&e) => match waits.next() {
                Some(ms) => {
                    pause(std::time::Duration::from_millis(*ms));
                    #[cfg(test)]
                    BETWEEN_ATTEMPTS.with(|hook| hook.take().map(|f| f()));
                    vet()?;
                }
                None => return Err(e.into()),
            },
            result => return Ok(result?),
        }
    }
}

/// `dir` is still the folder the path names, and no link sits at the name:
/// walked again, a folder that became a link is refused, and so is one that
/// is now another folder (the one held was moved away).
fn vet(dir: &OwnedHandle, base: &Path, dirs: &[&OsStr], file: &OsStr, rel: &Path) -> anyhow::Result<()> {
    let again = walk(base, dirs, false, rel)?;
    let (now, held) = (info(&again)?, info(dir)?);
    if (now.volume_serial, now.index) != (held.volume_serial, held.index) {
        return Err(link_refused(rel));
    }
    remove_link(dir, file, rel)
}

#[cfg(test)]
thread_local! {
    /// Run once after the next wait, before the checks: a test's swap.
    pub(super) static BETWEEN_ATTEMPTS: std::cell::Cell<Option<Box<dyn FnOnce()>>> =
        const { std::cell::Cell::new(None) };
}

/// A wait that doesn't stall a tokio worker: on the multi-threaded runtime
/// the worker hands its tasks off while this thread sleeps.
fn pause(wait: std::time::Duration) {
    use tokio::runtime::{Handle, RuntimeFlavor};
    match Handle::try_current() {
        Ok(rt) if rt.runtime_flavor() == RuntimeFlavor::MultiThread => {
            tokio::task::block_in_place(|| std::thread::sleep(wait))
        }
        _ => std::thread::sleep(wait),
    }
}

pub fn create_new(
    base: &Path,
    dirs: &[&OsStr],
    file: &OsStr,
    bytes: &[u8],
    rel: &Path,
) -> anyhow::Result<bool> {
    let dir = walk(base, dirs, true, rel)?;
    match create(&dir, file, rel)? {
        Some(mut out) => {
            out.write_all(bytes)?;
            Ok(true)
        }
        None => Ok(false),
    }
}

pub fn create_dirs(base: &Path, dirs: &[&OsStr], rel: &Path) -> anyhow::Result<()> {
    walk(base, dirs, true, rel).map(drop)
}

/// The text of a plain file with one name: a reparse point, or a hard link
/// to a file elsewhere (the owner's config), is refused (CE-026 F2).
pub fn read(base: &Path, dirs: &[&OsStr], file: &OsStr, rel: &Path) -> anyhow::Result<String> {
    let dir = walk(base, dirs, false, rel)?;
    let handle = open_at(
        &dir,
        file,
        FILE_GENERIC_READ,
        FILE_OPEN,
        FILE_NON_DIRECTORY_FILE,
        rel,
    )?
    .map_err(|e| refused(e, rel))?;
    let about = info(&handle)?;
    if about.attributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 || about.links > 1 {
        return Err(link_refused(rel));
    }
    let mut text = String::new();
    File::from(handle).read_to_string(&mut text)?;
    Ok(text)
}
