//! macOS: the app's code signature (H-044 §3, H-110). A peer is the owner's
//! app when the running process, named by its audit token, satisfies the
//! requirement compiled into hermesd: signed by Apple's Developer ID chain
//! for the owner's team, as the app's bundle identifier.

use super::owner::Peer;
use std::ffi::c_void;
use std::os::raw::c_char;

type CFRef = *const c_void;

const UTF8: u32 = 0x0800_0100;
const DEFAULT_FLAGS: u32 = 0;
/// `getsockopt(SOL_LOCAL, LOCAL_PEERTOKEN)`.
const SOL_LOCAL: i32 = 0;
const LOCAL_PEERTOKEN: i32 = 6;

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    static kCFTypeDictionaryKeyCallBacks: [usize; 6];
    static kCFTypeDictionaryValueCallBacks: [usize; 5];
    fn CFDataCreate(alloc: CFRef, bytes: *const u8, len: isize) -> CFRef;
    fn CFDictionaryCreate(
        alloc: CFRef,
        keys: *const CFRef,
        values: *const CFRef,
        count: isize,
        key_callbacks: *const c_void,
        value_callbacks: *const c_void,
    ) -> CFRef;
    fn CFStringCreateWithBytes(
        alloc: CFRef,
        bytes: *const u8,
        len: isize,
        encoding: u32,
        external: u8,
    ) -> CFRef;
    fn CFStringGetLength(string: CFRef) -> isize;
    fn CFStringGetMaximumSizeForEncoding(length: isize, encoding: u32) -> isize;
    fn CFStringGetCString(string: CFRef, buffer: *mut c_char, size: isize, encoding: u32) -> u8;
    #[cfg(test)]
    fn CFURLCreateFromFileSystemRepresentation(
        alloc: CFRef,
        path: *const u8,
        len: isize,
        is_dir: u8,
    ) -> CFRef;
    fn CFRelease(object: CFRef);
}

#[link(name = "Security", kind = "framework")]
extern "C" {
    static kSecGuestAttributeAudit: CFRef;
    fn SecCodeCopyGuestWithAttributes(
        host: CFRef,
        attrs: CFRef,
        flags: u32,
        guest: *mut CFRef,
    ) -> i32;
    fn SecCodeCheckValidity(code: CFRef, flags: u32, requirement: CFRef) -> i32;
    fn SecRequirementCreateWithString(text: CFRef, flags: u32, requirement: *mut CFRef) -> i32;
    #[cfg(test)]
    fn SecStaticCodeCreateWithPath(path: CFRef, flags: u32, code: *mut CFRef) -> i32;
    #[cfg(test)]
    fn SecCodeCopyDesignatedRequirement(code: CFRef, flags: u32, requirement: *mut CFRef) -> i32;
    #[cfg(test)]
    fn SecRequirementCopyString(requirement: CFRef, flags: u32, text: *mut CFRef) -> i32;
}

/// A Core Foundation object released when dropped.
struct Owned(CFRef);

impl Owned {
    fn new(object: CFRef) -> Option<Self> {
        (!object.is_null()).then_some(Self(object))
    }
}

impl Drop for Owned {
    fn drop(&mut self) {
        // SAFETY: we own one reference to a non-null object.
        unsafe { CFRelease(self.0) }
    }
}

fn cf_string(text: &str) -> Option<Owned> {
    // SAFETY: the bytes are valid for `len`; CF copies them.
    Owned::new(unsafe {
        CFStringCreateWithBytes(
            std::ptr::null(),
            text.as_ptr(),
            text.len() as isize,
            UTF8,
            0,
        )
    })
}

#[cfg_attr(not(test), allow(dead_code))]
fn rust_string(string: &Owned) -> Option<String> {
    // SAFETY: `string` is a live CFString; the buffer is sized for it.
    unsafe {
        let size = CFStringGetMaximumSizeForEncoding(CFStringGetLength(string.0), UTF8) + 1;
        let mut buffer = vec![0u8; usize::try_from(size).ok()?];
        if CFStringGetCString(string.0, buffer.as_mut_ptr().cast(), size, UTF8) == 0 {
            return None;
        }
        let end = buffer.iter().position(|&b| b == 0)?;
        String::from_utf8(buffer[..end].to_vec()).ok()
    }
}

/// The designated requirement of the code at `path`, as text: what any later
/// build must satisfy to count as the same app.
#[cfg(test)]
fn requirement_of(path: &std::path::Path) -> anyhow::Result<String> {
    let path = path.to_string_lossy();
    // SAFETY: each object is checked for null and released by `Owned`.
    unsafe {
        let url = Owned::new(CFURLCreateFromFileSystemRepresentation(
            std::ptr::null(),
            path.as_ptr(),
            path.len() as isize,
            1,
        ))
        .ok_or_else(|| anyhow::anyhow!("bad app path {path}"))?;
        let mut code: CFRef = std::ptr::null();
        anyhow::ensure!(
            SecStaticCodeCreateWithPath(url.0, DEFAULT_FLAGS, &mut code) == 0,
            "can't read the signature of {path}"
        );
        let code = Owned::new(code).ok_or_else(|| anyhow::anyhow!("no code for {path}"))?;
        let mut requirement: CFRef = std::ptr::null();
        anyhow::ensure!(
            SecCodeCopyDesignatedRequirement(code.0, DEFAULT_FLAGS, &mut requirement) == 0,
            "{path} has no designated requirement"
        );
        let requirement =
            Owned::new(requirement).ok_or_else(|| anyhow::anyhow!("no requirement"))?;
        let mut text: CFRef = std::ptr::null();
        anyhow::ensure!(
            SecRequirementCopyString(requirement.0, DEFAULT_FLAGS, &mut text) == 0,
            "can't write out the requirement of {path}"
        );
        let text = Owned::new(text).ok_or_else(|| anyhow::anyhow!("empty requirement"))?;
        rust_string(&text).ok_or_else(|| anyhow::anyhow!("unreadable requirement"))
    }
}

/// The peer's audit token, which names the process exactly (pid and
/// pidversion), from a connected Unix socket.
pub fn peer_audit_token(fd: std::os::fd::RawFd) -> Option<[u32; 8]> {
    let mut token = [0u32; 8];
    let mut len = std::mem::size_of_val(&token) as libc::socklen_t;
    // SAFETY: `token` is writable for `len` bytes.
    let ok = unsafe {
        libc::getsockopt(
            fd,
            SOL_LOCAL,
            LOCAL_PEERTOKEN,
            token.as_mut_ptr().cast(),
            &mut len,
        )
    };
    (ok == 0 && len as usize == std::mem::size_of_val(&token)).then_some(token)
}

/// The peer is the owner's signed app, unmodified since it was signed.
pub fn is_owner_app(peer: &Peer) -> bool {
    satisfies(&super::app_identity::requirement(), peer)
}

/// The running process the audit token names satisfies `requirement`.
fn satisfies(requirement: &str, peer: &Peer) -> bool {
    let Some(token) = peer.audit_token else {
        return false;
    };
    // SAFETY: each object is checked for null and released by `Owned`;
    // `token` outlives the CFData copy of it.
    unsafe {
        let Some(text) = cf_string(requirement) else {
            return false;
        };
        let mut parsed: CFRef = std::ptr::null();
        if SecRequirementCreateWithString(text.0, DEFAULT_FLAGS, &mut parsed) != 0 {
            return false;
        }
        let Some(parsed) = Owned::new(parsed) else {
            return false;
        };
        let Some(audit) = Owned::new(CFDataCreate(
            std::ptr::null(),
            token.as_ptr().cast(),
            std::mem::size_of_val(&token) as isize,
        )) else {
            return false;
        };
        let keys = [kSecGuestAttributeAudit];
        let values = [audit.0];
        let Some(attrs) = Owned::new(CFDictionaryCreate(
            std::ptr::null(),
            keys.as_ptr(),
            values.as_ptr(),
            1,
            (&raw const kCFTypeDictionaryKeyCallBacks).cast(),
            (&raw const kCFTypeDictionaryValueCallBacks).cast(),
        )) else {
            return false;
        };
        let mut code: CFRef = std::ptr::null();
        if SecCodeCopyGuestWithAttributes(std::ptr::null(), attrs.0, DEFAULT_FLAGS, &mut code) != 0
        {
            return false;
        }
        let Some(code) = Owned::new(code) else {
            return false;
        };
        SecCodeCheckValidity(code.0, DEFAULT_FLAGS, parsed.0) == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The real signature check, against this test binary: its own audit
    /// token (from a socket pair) satisfies its own designated requirement,
    /// and not one for another app, nor the owner's app requirement.
    #[test]
    fn a_process_satisfies_its_own_requirement_and_no_other() {
        let exe = std::env::current_exe().expect("exe");
        let requirement = requirement_of(&exe).expect("this binary's requirement");
        let (ours, _theirs) = std::os::unix::net::UnixStream::pair().expect("pair");
        let token = peer_audit_token(std::os::fd::AsRawFd::as_raw_fd(&ours)).expect("token");
        let me = Peer {
            pid: std::process::id(),
            audit_token: Some(token),
        };
        assert!(satisfies(&requirement, &me), "{requirement}");
        assert!(!satisfies("identifier \"com.example.other\"", &me));
        let no_token = Peer {
            audit_token: None,
            ..me
        };
        assert!(!satisfies(&requirement, &no_token));
        assert!(!is_owner_app(&me), "a test binary is not the owner's app");
    }

    /// The compiled requirement is one Security can parse: an unparsable
    /// one would refuse every app without saying why.
    #[test]
    fn the_compiled_requirement_parses() {
        let text = cf_string(&super::super::app_identity::requirement()).expect("cf");
        let mut parsed: CFRef = std::ptr::null();
        // SAFETY: `text` is a live CFString; `parsed` is released below.
        let status = unsafe { SecRequirementCreateWithString(text.0, DEFAULT_FLAGS, &mut parsed) };
        assert_eq!(status, 0);
        drop(Owned::new(parsed));
    }
}
