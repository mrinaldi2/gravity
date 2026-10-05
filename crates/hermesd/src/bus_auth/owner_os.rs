//! The OS half of recognising the owner's app (H-044 T4): what `service
//! install` pins, and whether a peer process is that app. macOS checks the
//! running process's code signature against the pinned requirement; Windows
//! checks that its executable sits in the pinned install folder.

#[cfg(not(any(target_os = "macos", windows)))]
use super::owner::{Peer, Pin};
#[cfg(not(any(target_os = "macos", windows)))]
use std::path::Path;

#[cfg(target_os = "macos")]
pub use super::owner_macos::{is_pinned_app, peer_audit_token, pin_for};

#[cfg(windows)]
pub use windows::{is_pinned_app, pin_for};

/// Linux has no desktop app: nothing is pinned, no process is the owner.
#[cfg(not(any(target_os = "macos", windows)))]
pub fn pin_for(_sidecar: &Path) -> anyhow::Result<Option<Pin>> {
    Ok(None)
}

#[cfg(not(any(target_os = "macos", windows)))]
pub fn is_pinned_app(_pin: &Pin, _peer: &Peer) -> bool {
    false
}

#[cfg(windows)]
mod windows {
    use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle, RawHandle};
    use std::path::{Path, PathBuf};

    use crate::bus_auth::owner::{Peer, Pin};

    const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;

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

    /// The bundled daemon ships next to the app's executable.
    pub fn pin_for(sidecar: &Path) -> anyhow::Result<Option<Pin>> {
        Ok(sidecar.parent().map(|dir| Pin {
            requirement: None,
            app_dir: Some(dir.to_string_lossy().into_owned()),
        }))
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

    /// The peer runs an executable from the app's folder other than the
    /// daemon itself.
    pub fn is_pinned_app(pin: &Pin, peer: &Peer) -> bool {
        let (Some(dir), Some(image)) = (pin.app_dir.as_deref(), image_path(peer.pid)) else {
            return false;
        };
        let in_dir = image
            .parent()
            .is_some_and(|parent| parent.as_os_str().eq_ignore_ascii_case(dir));
        let name = image
            .file_name()
            .map(|n| n.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_default();
        in_dir && !name.starts_with("hermesd")
    }
}
