//! `no_follow` on Windows, by handle (H-184, CE-026 F1).
//!
//! Each folder is opened relative to its parent's handle (`NtCreateFile` with
//! a `RootDirectory`) and `FILE_OPEN_REPARSE_POINT`, so a junction or symlink
//! is opened as itself and refused, never followed. The file is created as a
//! fresh temp relative to the last folder's handle and renamed by its own
//! handle into that folder (`SetFileInformationByHandle`, `RootDirectory`).
//! No path is resolved again after a check: a bot swapping a folder for a
//! junction mid-write changes nothing the daemon already holds open.

use std::ffi::{c_void, OsStr};
use std::fs::File;
use std::io::{Read, Write};
use std::os::windows::ffi::OsStrExt;
use std::os::windows::fs::OpenOptionsExt;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::path::Path;

use super::LinkRefused;

type Handle = *mut c_void;

#[repr(C)]
struct UnicodeString {
    length: u16,
    maximum_length: u16,
    buffer: *const u16,
}

#[repr(C)]
struct ObjectAttributes {
    length: u32,
    root_directory: Handle,
    object_name: *const UnicodeString,
    attributes: u32,
    security_descriptor: *const c_void,
    security_quality_of_service: *const c_void,
}

#[repr(C)]
struct IoStatusBlock {
    status: usize,
    information: usize,
}

#[repr(C)]
#[derive(Default)]
struct ByHandleFileInformation {
    attributes: u32,
    times: [u32; 6],
    volume_serial: u32,
    size: [u32; 2],
    links: u32,
    index: [u32; 2],
}

#[link(name = "ntdll")]
extern "system" {
    #[allow(clippy::too_many_arguments)]
    fn NtCreateFile(
        handle: *mut Handle,
        access: u32,
        attributes: *const ObjectAttributes,
        status: *mut IoStatusBlock,
        allocation: *const i64,
        file_attributes: u32,
        share: u32,
        disposition: u32,
        options: u32,
        ea: *const c_void,
        ea_length: u32,
    ) -> i32;
    fn RtlNtStatusToDosError(status: i32) -> u32;
}

#[link(name = "kernel32")]
extern "system" {
    fn GetFileInformationByHandle(file: Handle, info: *mut ByHandleFileInformation) -> i32;
    fn SetFileInformationByHandle(file: Handle, class: i32, info: *const c_void, size: u32) -> i32;
}

const OBJ_CASE_INSENSITIVE: u32 = 0x40;
const SYNCHRONIZE: u32 = 0x0010_0000;
const DELETE: u32 = 0x0001_0000;
const FILE_LIST_DIRECTORY: u32 = 0x1;
const FILE_TRAVERSE: u32 = 0x20;
const FILE_READ_ATTRIBUTES: u32 = 0x80;
const FILE_GENERIC_READ: u32 = 0x0012_0089;
const FILE_GENERIC_WRITE: u32 = 0x0012_0116;
const DIR_ACCESS: u32 = FILE_LIST_DIRECTORY | FILE_TRAVERSE | FILE_READ_ATTRIBUTES | SYNCHRONIZE;
const SHARE_ALL: u32 = 0x7;
const FILE_OPEN: u32 = 1;
const FILE_CREATE: u32 = 2;
const FILE_OPEN_IF: u32 = 3;
const FILE_DIRECTORY_FILE: u32 = 0x1;
const FILE_SYNCHRONOUS_IO_NONALERT: u32 = 0x20;
const FILE_NON_DIRECTORY_FILE: u32 = 0x40;
const FILE_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x10;
const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
const ERROR_FILE_NOT_FOUND: i32 = 2;
const ERROR_FILE_EXISTS: i32 = 80;
const ERROR_INVALID_PARAMETER: i32 = 87;
const ERROR_DIRECTORY: i32 = 267;
const ERROR_NOT_SUPPORTED: i32 = 50;
/// FILE_INFO_BY_HANDLE_CLASS values.
const FILE_RENAME_INFO: i32 = 3;
const FILE_DISPOSITION_INFO: i32 = 4;
const FILE_DISPOSITION_INFO_EX: i32 = 21;
const FILE_RENAME_INFO_EX: i32 = 22;
const RENAME_REPLACE_IF_EXISTS: u32 = 0x1;
const RENAME_POSIX_SEMANTICS: u32 = 0x2;
const DISPOSITION_DELETE: u32 = 0x1;
const DISPOSITION_POSIX_SEMANTICS: u32 = 0x2;

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

/// `name` in UTF-16; a stream name (`a:b`) or a separator is never a plain name.
fn wide(name: &OsStr, rel: &Path) -> anyhow::Result<Vec<u16>> {
    let wide: Vec<u16> = name.encode_wide().collect();
    if wide.is_empty()
        || wide
            .iter()
            .any(|&c| c == u16::from(b':') || c == u16::from(b'\\'))
    {
        anyhow::bail!("not a plain name in {}", rel.display());
    }
    Ok(wide)
}

/// `name` opened relative to `dir`, the reparse point itself if it is one.
fn open_at(
    dir: &OwnedHandle,
    name: &OsStr,
    access: u32,
    disposition: u32,
    options: u32,
    rel: &Path,
) -> anyhow::Result<std::io::Result<OwnedHandle>> {
    let wide = wide(name, rel)?;
    let bytes = u16::try_from(wide.len() * 2)?;
    let object_name = UnicodeString {
        length: bytes,
        maximum_length: bytes,
        buffer: wide.as_ptr(),
    };
    let attributes = ObjectAttributes {
        length: std::mem::size_of::<ObjectAttributes>() as u32,
        root_directory: dir.as_raw_handle(),
        object_name: &object_name,
        attributes: OBJ_CASE_INSENSITIVE,
        security_descriptor: std::ptr::null(),
        security_quality_of_service: std::ptr::null(),
    };
    let mut status = IoStatusBlock {
        status: 0,
        information: 0,
    };
    let mut handle: Handle = std::ptr::null_mut();
    // SAFETY: every pointer is to a live local for the call; on success the
    // handle is fresh and owned below.
    let nt = unsafe {
        NtCreateFile(
            &mut handle,
            access | SYNCHRONIZE,
            &attributes,
            &mut status,
            std::ptr::null(),
            0,
            SHARE_ALL,
            disposition,
            options | FILE_SYNCHRONOUS_IO_NONALERT | FILE_OPEN_REPARSE_POINT,
            std::ptr::null(),
            0,
        )
    };
    if nt < 0 {
        // SAFETY: a plain status-code translation.
        let code = unsafe { RtlNtStatusToDosError(nt) };
        return Ok(Err(std::io::Error::from_raw_os_error(code as i32)));
    }
    // SAFETY: NtCreateFile succeeded, so `handle` is open and ours alone.
    Ok(Ok(unsafe { OwnedHandle::from_raw_handle(handle) }))
}

fn info(handle: &OwnedHandle) -> std::io::Result<ByHandleFileInformation> {
    let mut info = ByHandleFileInformation::default();
    // SAFETY: the handle is open for the call; info is a valid out-pointer.
    if unsafe { GetFileInformationByHandle(handle.as_raw_handle(), &mut info) } == 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(info)
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

/// Sets one FILE_INFO_BY_HANDLE_CLASS record on `handle`.
fn set_info(handle: &OwnedHandle, class: i32, record: &[u8]) -> std::io::Result<()> {
    // SAFETY: `record` is a whole, aligned record of `class` for the call.
    let ok = unsafe {
        SetFileInformationByHandle(
            handle.as_raw_handle(),
            class,
            record.as_ptr().cast(),
            record.len() as u32,
        )
    };
    if ok == 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

/// The class unsupported here (an older Windows or another file system).
fn unsupported(error: &std::io::Error) -> bool {
    matches!(
        error.raw_os_error(),
        Some(ERROR_INVALID_PARAMETER | ERROR_NOT_SUPPORTED)
    )
}

/// Deletes what `handle` is open on: the link itself for a reparse point.
fn delete(handle: &OwnedHandle) -> std::io::Result<()> {
    let flags = (DISPOSITION_DELETE | DISPOSITION_POSIX_SEMANTICS).to_ne_bytes();
    match set_info(handle, FILE_DISPOSITION_INFO_EX, &flags) {
        Err(e) if unsupported(&e) => set_info(handle, FILE_DISPOSITION_INFO, &[1]),
        other => other,
    }
}

/// A FILE_RENAME_INFO record: flags, the folder handle, then the name.
fn rename_record(flags: u32, dir: &OwnedHandle, name: &[u16]) -> Vec<u8> {
    // Offsets of FILE_RENAME_INFO: the handle is pointer-aligned after the
    // 4-byte flags; the name follows its 4-byte length.
    let handle_at = std::mem::size_of::<usize>();
    let name_at = handle_at + std::mem::size_of::<usize>() + 4;
    let mut record = vec![0u8; name_at + (name.len() + 1) * 2];
    record[..4].copy_from_slice(&flags.to_ne_bytes());
    record[handle_at..handle_at + std::mem::size_of::<usize>()]
        .copy_from_slice(&(dir.as_raw_handle() as usize).to_ne_bytes());
    record[name_at - 4..name_at].copy_from_slice(&((name.len() * 2) as u32).to_ne_bytes());
    for (i, unit) in name.iter().enumerate() {
        record[name_at + i * 2..name_at + i * 2 + 2].copy_from_slice(&unit.to_ne_bytes());
    }
    record
}

/// Renames the open `file` to `name` in `dir`, replacing what is there.
fn rename_into(file: &OwnedHandle, dir: &OwnedHandle, name: &[u16]) -> std::io::Result<()> {
    let ex = rename_record(RENAME_REPLACE_IF_EXISTS | RENAME_POSIX_SEMANTICS, dir, name);
    match set_info(file, FILE_RENAME_INFO_EX, &ex) {
        Err(e) if unsupported(&e) => set_info(file, FILE_RENAME_INFO, &rename_record(1, dir, name)),
        other => other,
    }
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
    let done = out
        .write_all(bytes)
        .and_then(|()| out.sync_all())
        .map_err(anyhow::Error::from)
        .and_then(|()| remove_link(&dir, file, rel))
        .and_then(|()| rename_into(&handle, &dir, &target).map_err(anyhow::Error::from));
    if let Err(error) = done {
        let _ = delete(&handle);
        return Err(error.context(format!("writing {}", rel.display())));
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
