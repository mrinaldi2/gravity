//! The Windows calls `no_follow` makes by handle (H-184): `NtCreateFile`
//! relative to a folder handle, and file information set or read by handle.

use std::ffi::{c_void, OsStr};
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::path::Path;

pub(super) type Handle = *mut c_void;

#[repr(C)]
pub(super) struct UnicodeString {
    length: u16,
    maximum_length: u16,
    buffer: *const u16,
}

#[repr(C)]
pub(super) struct ObjectAttributes {
    length: u32,
    root_directory: Handle,
    object_name: *const UnicodeString,
    attributes: u32,
    security_descriptor: *const c_void,
    security_quality_of_service: *const c_void,
}

#[repr(C)]
pub(super) struct IoStatusBlock {
    status: usize,
    information: usize,
}

#[repr(C)]
#[derive(Default)]
/// Field for field as `confine_os` has it: two declarations of one
/// kernel32 function must agree (`clashing_extern_declarations`).
pub(super) struct ByHandleFileInformation {
    pub(super) attributes: u32,
    creation: [u32; 2],
    last_access: [u32; 2],
    last_write: [u32; 2],
    pub(super) volume_serial: u32,
    size_high: u32,
    size_low: u32,
    pub(super) links: u32,
    pub(super) index_high: u32,
    pub(super) index_low: u32,
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
    fn NtSetInformationFile(
        handle: Handle,
        status: *mut IoStatusBlock,
        info: *const c_void,
        length: u32,
        class: i32,
    ) -> i32;
    fn RtlNtStatusToDosError(status: i32) -> u32;
}

#[link(name = "kernel32")]
extern "system" {
    fn GetFileInformationByHandle(file: Handle, info: *mut ByHandleFileInformation) -> i32;
    fn SetFileInformationByHandle(file: Handle, class: i32, info: *const c_void, size: u32) -> i32;
}

pub(super) const OBJ_CASE_INSENSITIVE: u32 = 0x40;
pub(super) const SYNCHRONIZE: u32 = 0x0010_0000;
pub(super) const DELETE: u32 = 0x0001_0000;
pub(super) const FILE_LIST_DIRECTORY: u32 = 0x1;
pub(super) const FILE_TRAVERSE: u32 = 0x20;
pub(super) const FILE_READ_ATTRIBUTES: u32 = 0x80;
pub(super) const FILE_GENERIC_READ: u32 = 0x0012_0089;
pub(super) const FILE_GENERIC_WRITE: u32 = 0x0012_0116;
pub(super) const DIR_ACCESS: u32 =
    FILE_LIST_DIRECTORY | FILE_TRAVERSE | FILE_READ_ATTRIBUTES | SYNCHRONIZE;
pub(super) const SHARE_ALL: u32 = 0x7;
pub(super) const FILE_OPEN: u32 = 1;
pub(super) const FILE_CREATE: u32 = 2;
pub(super) const FILE_OPEN_IF: u32 = 3;
pub(super) const FILE_DIRECTORY_FILE: u32 = 0x1;
pub(super) const FILE_SYNCHRONOUS_IO_NONALERT: u32 = 0x20;
pub(super) const FILE_NON_DIRECTORY_FILE: u32 = 0x40;
pub(super) const FILE_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
pub(super) const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
pub(super) const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
pub(super) const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x10;
pub(super) const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
pub(super) const ERROR_FILE_NOT_FOUND: i32 = 2;
pub(super) const ERROR_ACCESS_DENIED: i32 = 5;
pub(super) const ERROR_FILE_EXISTS: i32 = 80;
pub(super) const ERROR_INVALID_PARAMETER: i32 = 87;
pub(super) const ERROR_DIRECTORY: i32 = 267;
pub(super) const ERROR_NOT_SUPPORTED: i32 = 50;
/// FILE_INFO_BY_HANDLE_CLASS values.
pub(super) const FILE_DISPOSITION_INFO: i32 = 4;
pub(super) const FILE_DISPOSITION_INFO_EX: i32 = 21;
/// FILE_INFORMATION_CLASS values for `NtSetInformationFile`.
pub(super) const FILE_RENAME_INFORMATION: i32 = 10;
pub(super) const FILE_RENAME_INFORMATION_EX: i32 = 65;
pub(super) const RENAME_REPLACE_IF_EXISTS: u32 = 0x1;
pub(super) const RENAME_POSIX_SEMANTICS: u32 = 0x2;
pub(super) const DISPOSITION_DELETE: u32 = 0x1;
pub(super) const DISPOSITION_POSIX_SEMANTICS: u32 = 0x2;

/// `name` in UTF-16; a stream name (`a:b`) or a separator is never a plain name.
pub(super) fn wide(name: &OsStr, rel: &Path) -> anyhow::Result<Vec<u16>> {
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
pub(super) fn open_at(
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
        return Ok(Err(nt_error(nt)));
    }
    // SAFETY: NtCreateFile succeeded, so `handle` is open and ours alone.
    Ok(Ok(unsafe { OwnedHandle::from_raw_handle(handle) }))
}

/// The Win32 error an NTSTATUS failure stands for.
fn nt_error(nt: i32) -> std::io::Error {
    // SAFETY: a plain status-code translation.
    let code = unsafe { RtlNtStatusToDosError(nt) };
    std::io::Error::from_raw_os_error(code as i32)
}

pub(super) fn info(handle: &OwnedHandle) -> std::io::Result<ByHandleFileInformation> {
    let mut info = ByHandleFileInformation::default();
    // SAFETY: the handle is open for the call; info is a valid out-pointer.
    if unsafe { GetFileInformationByHandle(handle.as_raw_handle(), &mut info) } == 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(info)
}

/// Sets one FILE_INFO_BY_HANDLE_CLASS record on `handle`.
pub(super) fn set_info(handle: &OwnedHandle, class: i32, record: &[u8]) -> std::io::Result<()> {
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
pub(super) fn unsupported(error: &std::io::Error) -> bool {
    matches!(
        error.raw_os_error(),
        Some(ERROR_INVALID_PARAMETER | ERROR_NOT_SUPPORTED)
    )
}

/// Deletes what `handle` is open on: the link itself for a reparse point.
pub(super) fn delete(handle: &OwnedHandle) -> std::io::Result<()> {
    let flags = (DISPOSITION_DELETE | DISPOSITION_POSIX_SEMANTICS).to_ne_bytes();
    match set_info(handle, FILE_DISPOSITION_INFO_EX, &flags) {
        Err(e) if unsupported(&e) => set_info(handle, FILE_DISPOSITION_INFO, &[1]),
        other => other,
    }
}

/// A FILE_RENAME_INFO record: flags, the folder handle, then the name.
pub(super) fn rename_record(flags: u32, dir: &OwnedHandle, name: &[u16]) -> Vec<u8> {
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

/// Sets one FILE_RENAME_INFORMATION(_EX) record on `handle` through ntdll.
/// Not `SetFileInformationByHandle`: kernelbase turns the name into a full
/// DOS path first, and a full path with a `RootDirectory` is
/// STATUS_INVALID_PARAMETER (os 87) for every rename.
fn set_rename(handle: &OwnedHandle, class: i32, record: &[u8]) -> std::io::Result<()> {
    let mut status = IoStatusBlock {
        status: 0,
        information: 0,
    };
    // SAFETY: `record` is a whole, aligned rename record for the call.
    let nt = unsafe {
        NtSetInformationFile(
            handle.as_raw_handle(),
            &mut status,
            record.as_ptr().cast(),
            record.len() as u32,
            class,
        )
    };
    if nt < 0 {
        return Err(nt_error(nt));
    }
    Ok(())
}

/// Renames the open `file` to `name` in `dir`, replacing what is there.
pub(super) fn rename_into(
    file: &OwnedHandle,
    dir: &OwnedHandle,
    name: &[u16],
) -> std::io::Result<()> {
    let ex = rename_record(RENAME_REPLACE_IF_EXISTS | RENAME_POSIX_SEMANTICS, dir, name);
    match set_rename(file, FILE_RENAME_INFORMATION_EX, &ex) {
        // STATUS_INVALID_INFO_CLASS (os 87) before Windows 10 1709.
        Err(e) if unsupported(&e) => {
            set_rename(file, FILE_RENAME_INFORMATION, &rename_record(1, dir, name))
        }
        other => other,
    }
}
