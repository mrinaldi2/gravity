//! Opening a build source, per OS (CE-010 S1, H-100): without following a
//! link at the end of the path, then asking the open handle what it is, how
//! many names it has and where it really lives. The real path closes the
//! window between the path checks and the open, in which a directory on the
//! way could be swapped for a link.

use std::fs::File;
use std::io;
use std::path::{Path, PathBuf};

/// What an open handle is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Handle {
    /// A plain file: not a directory, link or reparse point.
    pub regular: bool,
    /// How many names the file has; more than one is a hard link.
    pub links: u64,
    /// (device, inode) on unix, (volume serial, file index) on Windows.
    pub id: (u64, u64),
}

#[cfg(unix)]
pub fn open_no_follow(path: &Path) -> io::Result<File> {
    use std::os::unix::fs::OpenOptionsExt;
    std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
}

#[cfg(unix)]
pub fn describe(file: &File) -> io::Result<Handle> {
    use std::os::unix::fs::MetadataExt;
    let meta = file.metadata()?;
    Ok(Handle {
        regular: meta.is_file(),
        links: meta.nlink(),
        id: (meta.dev(), meta.ino()),
    })
}

/// The file's id by path, without following a link at its end.
#[cfg(unix)]
pub fn path_id(path: &Path) -> io::Result<Option<(u64, u64)>> {
    use std::os::unix::fs::MetadataExt;
    let meta = std::fs::symlink_metadata(path)?;
    Ok(Some((meta.dev(), meta.ino())))
}

/// Where the open file really is: F_GETPATH.
#[cfg(target_os = "macos")]
pub fn real_path(file: &File) -> io::Result<PathBuf> {
    use std::ffi::CStr;
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::io::AsRawFd;
    let mut buf = vec![0u8; libc::PATH_MAX as usize + 1];
    // SAFETY: F_GETPATH writes at most PATH_MAX bytes and a NUL into buf.
    let rc = unsafe { libc::fcntl(file.as_raw_fd(), libc::F_GETPATH, buf.as_mut_ptr()) };
    if rc == -1 {
        return Err(io::Error::last_os_error());
    }
    let path = CStr::from_bytes_until_nul(&buf).map_err(io::Error::other)?;
    Ok(PathBuf::from(std::ffi::OsStr::from_bytes(path.to_bytes())))
}

/// Where the open file really is: its /proc/self/fd link.
#[cfg(all(unix, not(target_os = "macos")))]
pub fn real_path(file: &File) -> io::Result<PathBuf> {
    use std::os::unix::io::AsRawFd;
    std::fs::read_link(format!("/proc/self/fd/{}", file.as_raw_fd()))
}

#[cfg(windows)]
mod win {
    use std::ffi::c_void;

    pub const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
    pub const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x10;
    pub const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;

    #[repr(C)]
    #[derive(Default)]
    pub struct ByHandleFileInformation {
        pub attributes: u32,
        pub creation: [u32; 2],
        pub last_access: [u32; 2],
        pub last_write: [u32; 2],
        pub volume_serial: u32,
        pub size_high: u32,
        pub size_low: u32,
        pub links: u32,
        pub index_high: u32,
        pub index_low: u32,
    }

    #[link(name = "kernel32")]
    extern "system" {
        pub fn GetFileInformationByHandle(
            file: *mut c_void,
            info: *mut ByHandleFileInformation,
        ) -> i32;
        pub fn GetFinalPathNameByHandleW(
            file: *mut c_void,
            path: *mut u16,
            len: u32,
            flags: u32,
        ) -> u32;
    }
}

/// Opens the reparse point itself rather than what it points at, so a link
/// at the end of the path is seen, not followed.
#[cfg(windows)]
pub fn open_no_follow(path: &Path) -> io::Result<File> {
    use std::os::windows::fs::OpenOptionsExt;
    std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(win::FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
}

#[cfg(windows)]
pub fn describe(file: &File) -> io::Result<Handle> {
    use std::os::windows::io::AsRawHandle;
    let mut info = win::ByHandleFileInformation::default();
    // SAFETY: the handle is open for the call; info is a valid out-pointer.
    let ok = unsafe { win::GetFileInformationByHandle(file.as_raw_handle(), &mut info) };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    let special = win::FILE_ATTRIBUTE_DIRECTORY | win::FILE_ATTRIBUTE_REPARSE_POINT;
    Ok(Handle {
        regular: info.attributes & special == 0,
        links: u64::from(info.links),
        id: (
            u64::from(info.volume_serial),
            (u64::from(info.index_high) << 32) | u64::from(info.index_low),
        ),
    })
}

/// Windows has no id by path without opening it; the real path of the
/// handle stands in for the comparison.
#[cfg(windows)]
pub fn path_id(_: &Path) -> io::Result<Option<(u64, u64)>> {
    Ok(None)
}

/// Where the open file really is, spelled as `fs::canonicalize` spells it
/// (`\\?\C:\…`): GetFinalPathNameByHandleW, normalized, with a drive.
#[cfg(windows)]
pub fn real_path(file: &File) -> io::Result<PathBuf> {
    use std::os::windows::ffi::OsStringExt;
    use std::os::windows::io::AsRawHandle;
    let mut buf = vec![0u16; 1024];
    loop {
        // SAFETY: the handle is open; buf holds `len` u16s.
        let n = unsafe {
            win::GetFinalPathNameByHandleW(
                file.as_raw_handle(),
                buf.as_mut_ptr(),
                buf.len() as u32,
                0,
            )
        } as usize;
        if n == 0 {
            return Err(io::Error::last_os_error());
        }
        if n < buf.len() {
            return Ok(PathBuf::from(std::ffi::OsString::from_wide(&buf[..n])));
        }
        buf.resize(n + 1, 0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_handle_knows_its_real_path_its_names_and_its_kind() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("app.zip");
        std::fs::write(&path, b"zip").unwrap();
        let real = std::fs::canonicalize(&path).unwrap();
        let file = open_no_follow(&real).unwrap();
        assert_eq!(real_path(&file).unwrap(), real);
        let handle = describe(&file).unwrap();
        assert!(handle.regular);
        assert_eq!(handle.links, 1);
        if let Some(id) = path_id(&real).unwrap() {
            assert_eq!(id, handle.id);
        }

        let other = tmp.path().join("other.zip");
        std::fs::hard_link(&path, &other).unwrap();
        assert_eq!(describe(&file).unwrap().links, 2, "a hard link shows");
    }

    #[cfg(unix)]
    #[test]
    fn a_link_at_the_end_is_not_followed() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("real.zip"), b"zip").unwrap();
        let link = tmp.path().join("link.zip");
        std::os::unix::fs::symlink(tmp.path().join("real.zip"), &link).unwrap();
        assert!(open_no_follow(&link).is_err());
    }

    /// Needs the symlink privilege (Developer Mode or an elevated shell).
    #[cfg(windows)]
    #[test]
    fn a_link_at_the_end_is_opened_as_a_link_on_windows() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("real.zip"), b"zip").unwrap();
        let link = tmp.path().join("link.zip");
        if std::os::windows::fs::symlink_file(tmp.path().join("real.zip"), &link).is_err() {
            eprintln!("skipped: no symlink privilege");
            return;
        }
        let file = open_no_follow(&link).unwrap();
        assert!(
            !describe(&file).unwrap().regular,
            "the reparse point itself"
        );
    }
}
