//! The endpoint on Windows: a named pipe whose DACL lets only this user
//! open it, and the client's pid from `GetNamedPipeClientProcessId`.

use std::ffi::c_void;
use std::os::windows::io::{AsRawHandle, RawHandle};
use std::path::Path;
use std::sync::Arc;

use sha2::{Digest, Sha256};
use tokio::net::windows::named_pipe::{NamedPipeServer, ServerOptions};

use crate::app::AppState;

const TOKEN_QUERY: u32 = 0x0008;
const TOKEN_USER: u32 = 1;
const SDDL_REVISION_1: u32 = 1;

#[repr(C)]
struct SecurityAttributes {
    length: u32,
    descriptor: *mut c_void,
    inherit: i32,
}

#[link(name = "kernel32")]
extern "system" {
    fn GetCurrentProcess() -> RawHandle;
    fn GetNamedPipeClientProcessId(pipe: RawHandle, pid: *mut u32) -> i32;
    fn CloseHandle(handle: RawHandle) -> i32;
}

#[link(name = "advapi32")]
extern "system" {
    fn OpenProcessToken(process: RawHandle, access: u32, token: *mut RawHandle) -> i32;
    fn GetTokenInformation(
        token: RawHandle,
        class: u32,
        info: *mut c_void,
        length: u32,
        returned: *mut u32,
    ) -> i32;
    fn ConvertSidToStringSidW(sid: *mut c_void, out: *mut *mut u16) -> i32;
    fn ConvertStringSecurityDescriptorToSecurityDescriptorW(
        sddl: *const u16,
        revision: u32,
        descriptor: *mut *mut c_void,
        size: *mut u32,
    ) -> i32;
}

/// The current user's SID as text, e.g. `S-1-5-21-…`.
fn user_sid() -> std::io::Result<String> {
    let mut token: RawHandle = std::ptr::null_mut();
    // SAFETY: the pseudo-handle needs no closing; `token` is an out pointer.
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
        return Err(std::io::Error::last_os_error());
    }
    let mut buffer = vec![0u8; 256];
    let mut size = 0u32;
    // SAFETY: the buffer is writable for its length; the token is live.
    let ok = unsafe {
        GetTokenInformation(
            token,
            TOKEN_USER,
            buffer.as_mut_ptr().cast(),
            buffer.len() as u32,
            &mut size,
        )
    };
    // SAFETY: the token was opened above and is closed once.
    unsafe { CloseHandle(token) };
    if ok == 0 {
        return Err(std::io::Error::last_os_error());
    }
    // TOKEN_USER starts with a SID_AND_ATTRIBUTES whose first field is the SID.
    // SAFETY: GetTokenInformation filled at least a TOKEN_USER.
    let sid = unsafe { *(buffer.as_ptr() as *const *mut c_void) };
    let mut text: *mut u16 = std::ptr::null_mut();
    // SAFETY: `sid` points into `buffer`, alive here; `text` is an out pointer.
    if unsafe { ConvertSidToStringSidW(sid, &mut text) } == 0 {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: the API returned a NUL-terminated string (leaked: LocalFree
    // isn't worth binding for one string per daemon).
    let len = (0..).take_while(|&i| unsafe { *text.add(i) } != 0).count();
    Ok(String::from_utf16_lossy(unsafe {
        std::slice::from_raw_parts(text, len)
    }))
}

/// `\\.\pipe\thehermes-<sid>-<home>-bus`: the user's, and this home's, so
/// two daemons of one user (tests) don't share a pipe.
pub(super) fn pipe_name(home: &Path) -> String {
    let sid = user_sid().unwrap_or_else(|_| "user".to_string());
    let digest = Sha256::digest(home.to_string_lossy().as_bytes());
    let home = hex::encode(&digest[..4]);
    format!(r"\\.\pipe\thehermes-{sid}-{home}-bus")
}

/// A security descriptor allowing only this user, for every pipe instance.
fn only_this_user() -> std::io::Result<SecurityAttributes> {
    let sddl = format!("D:P(A;;GA;;;{})", user_sid()?);
    let wide: Vec<u16> = sddl.encode_utf16().chain(std::iter::once(0)).collect();
    let mut descriptor: *mut c_void = std::ptr::null_mut();
    // SAFETY: `wide` is NUL-terminated; `descriptor` is an out pointer. The
    // descriptor is kept for the daemon's life.
    let ok = unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            wide.as_ptr(),
            SDDL_REVISION_1,
            &mut descriptor,
            std::ptr::null_mut(),
        )
    };
    if ok == 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(SecurityAttributes {
        length: std::mem::size_of::<SecurityAttributes>() as u32,
        descriptor,
        inherit: 0,
    })
}

fn create(
    name: &str,
    attrs: &mut SecurityAttributes,
    first: bool,
) -> std::io::Result<NamedPipeServer> {
    // SAFETY: `attrs` holds a valid descriptor that outlives the call.
    unsafe {
        ServerOptions::new()
            .first_pipe_instance(first)
            .create_with_security_attributes_raw(name, (attrs as *mut SecurityAttributes).cast())
    }
}

/// The pid of the process on the other end of a connected pipe.
pub(super) fn client_pid(pipe: &NamedPipeServer) -> Option<u32> {
    let mut pid = 0u32;
    // SAFETY: the pipe handle is live; `pid` is an out pointer.
    let ok = unsafe { GetNamedPipeClientProcessId(pipe.as_raw_handle(), &mut pid) };
    (ok != 0).then_some(pid)
}

pub(super) async fn serve(app: Arc<AppState>) -> anyhow::Result<()> {
    let name = pipe_name(&app.cfg.home);
    let mut attrs = only_this_user()?;
    let mut server = create(&name, &mut attrs, true)?;
    tracing::info!(pipe = %name, "bus endpoint listening");
    loop {
        server.connect().await?;
        let connected = server;
        server = create(&name, &mut attrs, false)?;
        let pid = client_pid(&connected);
        tokio::spawn(super::serve_connection(app.clone(), connected, pid));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus_auth::os::OsProcessTable;
    use crate::bus_auth::session::ProcessTable;

    /// Test 9: a pipe client's pid is this process, and its creation-time
    /// chain holds up to its parent.
    #[tokio::test]
    async fn a_pipe_client_resolves_to_its_process() {
        let home = std::env::temp_dir().join(format!("hermes-pipe-{}", std::process::id()));
        let name = pipe_name(&home);
        let mut attrs = only_this_user().expect("descriptor");
        let server = create(&name, &mut attrs, true).expect("pipe");
        let client = tokio::net::windows::named_pipe::ClientOptions::new()
            .open(&name)
            .expect("connect");
        server.connect().await.expect("accepted");
        assert_eq!(client_pid(&server), Some(std::process::id()));
        let me = OsProcessTable
            .info(std::process::id())
            .expect("this process");
        let parent = OsProcessTable.info(me.ppid).expect("its parent");
        assert!(parent.start <= me.start);
        drop(client);
    }
}
