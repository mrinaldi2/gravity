//! Running an owner action on this computer (H-117 R1).
//!
//! The stored content runs as one argument, never from a file a bot could
//! swap between the hash check and the run (ARCH-R49 M2): `zsh -f -c`,
//! `bash --noprofile --norc -c`, `powershell -EncodedCommand` (UTF-16LE,
//! base64) or `cmd /d /c`. It runs as the daemon's user, which is the
//! owner, with stdin closed and no sudo, in its own process group (Unix) or
//! Job Object (Windows), so a timeout kills everything it started. Unix
//! gets a clean environment with the owner's login `PATH`; Windows keeps
//! the environment but drops the daemon's own tokens.

use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use base64::Engine;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::Command;

use super::model::{Proposal, Shell, State};

/// How a run ended.
#[derive(Debug)]
pub struct Ran {
    pub state: State,
    pub exit_code: Option<i32>,
    /// Everything it printed, stdout and stderr in arrival order.
    pub output: String,
}

/// The owner's login `PATH`, read once from their login shell (Unix; on
/// Windows the environment is inherited).
#[cfg(unix)]
fn login_path() -> &'static str {
    static PATH: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    PATH.get_or_init(|| {
        let fallback = "/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin";
        std::process::Command::new("/bin/zsh")
            .args(["-l", "-c", "printf %s \"$PATH\""])
            .stdin(Stdio::null())
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .filter(|p| !p.is_empty())
            .unwrap_or_else(|| fallback.to_string())
    })
}

/// PowerShell's `-EncodedCommand`: UTF-16LE, base64.
pub fn encoded_command(content: &str) -> String {
    let bytes: Vec<u8> = content.encode_utf16().flat_map(u16::to_le_bytes).collect();
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

/// The program and its arguments, the content among them.
pub fn argv(shell: Shell, content: &str) -> (String, Vec<String>) {
    match shell {
        Shell::Zsh => (
            "/bin/zsh".into(),
            vec!["-f".into(), "-c".into(), content.into()],
        ),
        Shell::Bash => (
            "/bin/bash".into(),
            vec![
                "--noprofile".into(),
                "--norc".into(),
                "-c".into(),
                content.into(),
            ],
        ),
        Shell::Powershell => (
            if cfg!(windows) {
                "powershell.exe"
            } else {
                "pwsh"
            }
            .into(),
            vec![
                "-NoProfile".into(),
                "-NonInteractive".into(),
                "-EncodedCommand".into(),
                encoded_command(content),
            ],
        ),
        Shell::Cmd => (
            "cmd.exe".into(),
            vec!["/d".into(), "/c".into(), content.into()],
        ),
    }
}

fn command(p: &Proposal) -> Command {
    let (program, args) = argv(p.shell, &p.content);
    let mut cmd = Command::new(program);
    cmd.args(args)
        .current_dir(&p.cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(unix)]
    {
        cmd.env_clear();
        for key in ["HOME", "USER", "LOGNAME", "LANG", "TMPDIR", "TERM"] {
            if let Ok(value) = std::env::var(key) {
                cmd.env(key, value);
            }
        }
        cmd.env("PATH", login_path());
        cmd.process_group(0);
    }
    #[cfg(windows)]
    for (key, _) in std::env::vars() {
        let upper = key.to_ascii_uppercase();
        if upper.starts_with("THEHERMES_") || upper.starts_with("GRAVITY_") {
            cmd.env_remove(key);
        }
    }
    cmd
}

/// Ends the run and everything it started.
#[cfg(unix)]
fn kill_all(child: &tokio::process::Child) {
    if let Some(pid) = child.id() {
        // SAFETY: the child leads its own process group (process_group(0)).
        unsafe { libc::killpg(pid as i32, libc::SIGKILL) };
    }
}

/// Runs `p`, appending what it prints to `log` and handing each chunk to
/// `on_chunk` as it comes.
pub async fn execute(p: &Proposal, log: &Path, on_chunk: impl Fn(&str)) -> Ran {
    let mut file = match open_log(log).await {
        Ok(f) => Some(f),
        Err(e) => {
            tracing::warn!(error = %e, "owner action log unavailable");
            None
        }
    };
    let mut child = match command(p).spawn() {
        Ok(child) => child,
        Err(e) => {
            let output = format!("could not start {}: {e}", p.shell.as_str());
            return Ran {
                state: State::Failed,
                exit_code: None,
                output,
            };
        }
    };
    #[cfg(windows)]
    let job = {
        let job = crate::holders::job::Job::new().ok();
        if let (Some(job), Some(raw)) = (&job, child.raw_handle()) {
            // SAFETY: the child's handle is live while `child` is.
            let handle = unsafe { std::os::windows::io::BorrowedHandle::borrow_raw(raw) };
            let _ = job.assign(&handle);
        }
        job
    };
    let mut stdout = child.stdout.take();
    let mut stderr = child.stderr.take();
    let mut output = String::new();
    let deadline = tokio::time::sleep(Duration::from_secs(u64::from(p.timeout_s)));
    tokio::pin!(deadline);
    let (mut out_buf, mut err_buf) = ([0u8; 8192], [0u8; 8192]);
    let mut timed_out = false;
    loop {
        let chunk = tokio::select! {
            n = read(&mut stdout, &mut out_buf) => n.map(|n| out_buf[..n].to_vec()),
            n = read(&mut stderr, &mut err_buf) => n.map(|n| err_buf[..n].to_vec()),
            () = &mut deadline => { timed_out = true; None }
        };
        let Some(bytes) = chunk else {
            if timed_out || (stdout.is_none() && stderr.is_none()) {
                break;
            }
            continue;
        };
        let text = String::from_utf8_lossy(&bytes).into_owned();
        if let Some(f) = file.as_mut() {
            let _ = f.write_all(&bytes).await;
        }
        on_chunk(&text);
        output.push_str(&text);
    }
    // tokio's File writes in the background: without this the log's tail
    // can be missing when the run is read back (a flake under load).
    if let Some(f) = file.as_mut() {
        let _ = f.flush().await;
    }
    if timed_out {
        #[cfg(unix)]
        kill_all(&child);
        #[cfg(windows)]
        if let Some(job) = &job {
            let _ = job.terminate();
        }
        let _ = child.kill().await;
        let note = format!("\n[timed out after {}s; stopped]\n", p.timeout_s);
        on_chunk(&note);
        output.push_str(&note);
        return Ran {
            state: State::TimedOut,
            exit_code: None,
            output,
        };
    }
    let status = child.wait().await.ok();
    let exit_code = status.and_then(|s| s.code());
    let state = if status.is_some_and(|s| s.success()) {
        State::Succeeded
    } else {
        State::Failed
    };
    Ran {
        state,
        exit_code,
        output,
    }
}

/// One read from a stream; `None` once it's done, after which it is never
/// polled again.
async fn read<R: AsyncReadExt + Unpin>(stream: &mut Option<R>, buf: &mut [u8]) -> Option<usize> {
    let Some(reader) = stream.as_mut() else {
        return std::future::pending().await;
    };
    match reader.read(buf).await {
        Ok(0) | Err(_) => {
            *stream = None;
            None
        }
        Ok(n) => Some(n),
    }
}

async fn open_log(path: &Path) -> std::io::Result<tokio::fs::File> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
        }
    }
    let mut options = tokio::fs::OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    options.mode(0o600);
    options.open(path).await
}
