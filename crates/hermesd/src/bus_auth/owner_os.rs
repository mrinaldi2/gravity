//! The OS half of recognising the owner's app (H-044 T4, H-110), against
//! what is compiled into hermesd (`app_identity`). macOS checks the running
//! process's code signature against the owner's team; Windows checks that
//! its executable is in the app's Program Files folder.

#[cfg(not(any(target_os = "macos", windows)))]
use super::owner::Peer;

#[cfg(target_os = "macos")]
pub use super::owner_macos::{is_owner_app, peer_audit_token};

#[cfg(windows)]
pub use windows::is_owner_app;

/// Linux has no desktop app: no process is the owner's.
#[cfg(not(any(target_os = "macos", windows)))]
pub fn is_owner_app(_peer: &Peer) -> bool {
    false
}

#[cfg(windows)]
mod windows {
    use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle, RawHandle};
    use std::path::PathBuf;

    use crate::bus_auth::app_identity::{is_app_image, WINDOWS_APP_FOLDER};
    use crate::bus_auth::owner::Peer;

    const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;

    /// `FOLDERID_ProgramFiles` {905E63B6-C1BF-494E-B29C-65B732D3D21A}.
    #[repr(C)]
    struct Guid(u32, u16, u16, [u8; 8]);
    const FOLDERID_PROGRAM_FILES: Guid = Guid(
        0x905E_63B6,
        0xC1BF,
        0x494E,
        [0xB2, 0x9C, 0x65, 0xB7, 0x32, 0xD3, 0xD2, 0x1A],
    );

    #[link(name = "kernel32")]
    extern "system" {
        fn OpenProcess(access: u32, inherit: i32, pid: u32) -> RawHandle;
        fn QueryFullProcessImageNameW(
            process: RawHandle,
            flags: u32,
            name: *mut u16,
            size: *mut u32,
        ) -> i32;
    }

    #[link(name = "shell32")]
    extern "system" {
        fn SHGetKnownFolderPath(
            id: *const Guid,
            flags: u32,
            token: RawHandle,
            path: *mut *mut u16,
        ) -> i32;
    }

    #[link(name = "ole32")]
    extern "system" {
        fn CoTaskMemFree(memory: *mut std::ffi::c_void);
    }

    /// `C:\Program Files`, from the shell rather than the environment, which
    /// the daemon inherits from the user and so a user could redirect.
    fn program_files() -> Option<PathBuf> {
        let mut raw: *mut u16 = std::ptr::null_mut();
        // SAFETY: on return `raw` is null or a NUL-terminated string we free.
        let status = unsafe {
            SHGetKnownFolderPath(&FOLDERID_PROGRAM_FILES, 0, std::ptr::null_mut(), &mut raw)
        };
        if raw.is_null() {
            return None;
        }
        // SAFETY: `raw` is NUL-terminated; it's freed once, after the copy.
        let path = unsafe {
            let len = (0..).take_while(|&i| *raw.add(i) != 0).count();
            let text = String::from_utf16_lossy(std::slice::from_raw_parts(raw, len));
            CoTaskMemFree(raw.cast());
            text
        };
        (status == 0).then(|| PathBuf::from(path))
    }

    fn image_path(pid: u32) -> Option<PathBuf> {
        // SAFETY: no pointers; a non-null result is ours to close.
        let raw = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
        if raw.is_null() {
            return None;
        }
        // SAFETY: raw is a live process handle nothing else owns.
        let process = unsafe { OwnedHandle::from_raw_handle(raw) };
        let mut buffer = vec![0u16; 32_768];
        let mut size = buffer.len() as u32;
        // SAFETY: the buffer is writable for `size` UTF-16 units.
        let ok = unsafe {
            QueryFullProcessImageNameW(process.as_raw_handle(), 0, buffer.as_mut_ptr(), &mut size)
        };
        (ok != 0).then(|| PathBuf::from(String::from_utf16_lossy(&buffer[..size as usize])))
    }

    /// The peer runs an executable from `Program Files\The Hermes`, which
    /// only an administrator can write, other than the daemon itself.
    pub fn is_owner_app(peer: &Peer) -> bool {
        let (Some(root), Some(image)) = (program_files(), image_path(peer.pid)) else {
            return false;
        };
        is_app_image(&image, &root.join(WINDOWS_APP_FOLDER))
    }

    #[cfg(test)]
    mod tests {
        #[test]
        fn program_files_comes_from_the_shell() {
            let path = super::program_files().expect("Program Files");
            assert!(path.is_absolute(), "{}", path.display());
            assert!(path.exists(), "{}", path.display());
        }

        #[test]
        fn a_test_binary_is_not_the_owners_app() {
            let me = crate::bus_auth::owner::Peer {
                pid: std::process::id(),
                audit_token: None,
            };
            assert!(super::image_path(me.pid).is_some());
            assert!(!super::is_owner_app(&me));
        }
    }
}
