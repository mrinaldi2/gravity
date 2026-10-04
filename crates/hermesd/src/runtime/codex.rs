//! Codex CLI App Server over stdio JSONL. Bus input is a separate RPC stream.
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Stdio};
use std::sync::{mpsc, Arc, Mutex};
use std::time::Duration;

use super::{BotSpec, Capabilities, RuntimeAdapter, RuntimeSession, SessionEvent, StartedSession};
use anyhow::Context;
use serde_json::Value;

mod approvals;
mod input;
mod native;
mod observations;
mod transport;
mod worker;

pub use native::NativeCodexAdapter;

/// Where a Codex bot's thread id is kept, in its bot directory: the next
/// session resumes that thread.
pub const THREAD_FILE: &str = "codex-thread-id";

#[derive(Debug, Clone)]
pub struct CodexSpec {
    pub bin: String,
    pub args: Vec<String>,
    pub port: u16,
    pub artifacts: Option<PathBuf>,
    /// The bot's own browser server, `{command, args, env}`, when it has one.
    pub browser: Option<Value>,
    /// The project's permission profile (H-031) and the configured trusted
    /// paths, which Trusted and Full add to the sandbox's writable roots.
    pub profile: bus::PermissionProfile,
    pub trusted_paths: Vec<String>,
}

impl CodexSpec {
    /// Where the sandbox may write, whether it may reach the network, and
    /// its approval policy, from the permission profile. Codex has no
    /// deny-list, so even Full keeps the workspace-write sandbox: it only
    /// stops asking and widens where it may write.
    pub fn policy(&self, mut roots: Vec<String>) -> (Vec<String>, bool, &'static str) {
        let wider = self.profile != bus::PermissionProfile::Standard;
        if wider {
            roots.extend(self.trusted_paths.iter().map(|p| expand_home(p)));
        }
        let approval = if self.profile == bus::PermissionProfile::Full {
            "never"
        } else {
            "on-request"
        };
        (roots, wider, approval)
    }
}

/// `~/x` against the user's home; anything else as given.
fn expand_home(path: &str) -> String {
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"));
    match (path.strip_prefix("~/"), home) {
        (Some(rest), Some(home)) => std::path::Path::new(&home).join(rest).display().to_string(),
        _ => path.to_string(),
    }
}

pub struct CodexAdapter;

pub(super) enum Wire {
    Server(Value),
    Closed,
    Input(Vec<u8>),
    Deliver(String, mpsc::SyncSender<anyhow::Result<()>>),
    /// The owner's answer to a permission request, by its number.
    Answer(u64, super::PermissionAnswer),
    Stop,
}

pub fn transcript_path(workspace: &Path) -> PathBuf {
    workspace
        .parent()
        .unwrap_or(workspace)
        .join("codex-observations.jsonl")
}

impl RuntimeAdapter for CodexAdapter {
    fn capabilities(&self) -> Capabilities {
        Capabilities {
            kind: "codex_cli",
            native_background: false,
            channel_delivery: true,
            permission_relay: true,
        }
    }
    fn probe(&self) -> anyhow::Result<String> {
        super::executable::version("codex")
    }
    fn start(&self, spec: &BotSpec) -> anyhow::Result<StartedSession> {
        start_server(spec, None)
    }
}

/// `codex app-server` in the bot's workspace, with the bot's environment.
fn server_command(spec: &BotSpec) -> anyhow::Result<std::process::Command> {
    let codex = spec
        .codex
        .as_ref()
        .context("missing Codex runtime settings")?;
    let mut command = super::executable::command(&codex.bin);
    command
        .args(&codex.args)
        .arg("app-server")
        .current_dir(&spec.workspace)
        .envs(spec.env.iter().cloned())
        .env_remove("CODEX_THREAD_ID");
    for key in super::withheld_env() {
        command.env_remove(key);
    }
    Ok(command)
}

fn start_server(
    spec: &BotSpec,
    remote: Option<&transport::Remote>,
) -> anyhow::Result<StartedSession> {
    let mut command = server_command(spec)?;
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(remote) = remote {
        command.args([
            "--listen",
            &remote.address,
            "--ws-auth",
            "capability-token",
            "--ws-token-sha256",
            &remote.verifier(),
        ]);
    } else {
        command.args(["--listen", "stdio://"]);
    }
    let bin = command.get_program().to_string_lossy().into_owned();
    let mut child = command
        .spawn()
        .with_context(|| format!("spawn Codex CLI App Server using {bin}"))?;
    let stdin = child.stdin.take().context("Codex stdin")?;
    let stdout = child.stdout.take().context("Codex stdout")?;
    let stderr = child.stderr.take().context("Codex stderr")?;
    let child = Arc::new(Mutex::new(child));
    let (tx, rx) = mpsc::channel();
    let (events_tx, events_rx) = tokio::sync::mpsc::unbounded_channel();
    let native = remote.is_some();
    let input = if let Some(remote) = remote {
        drop(stdin);
        std::thread::spawn(
            move || {
                for _ in BufReader::new(stdout).lines().map_while(Result::ok) {}
            },
        );
        match transport::connect(remote, tx.clone(), child.clone()) {
            Ok(input) => input,
            Err(error) => {
                let _ = child.lock().unwrap_or_else(|e| e.into_inner()).kill();
                return Err(error);
            }
        }
    } else {
        let reader_tx = tx.clone();
        std::thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            loop {
                let mut line = String::new();
                match reader.read_line(&mut line) {
                    Ok(0) | Err(_) => break,
                    Ok(_) => match serde_json::from_str(&line) {
                        Ok(value) => {
                            if reader_tx.send(Wire::Server(value)).is_err() {
                                break;
                            }
                        }
                        Err(_) => break,
                    },
                }
            }
            let _ = reader_tx.send(Wire::Closed);
        });
        transport::RpcWriter::Stdio(std::io::BufWriter::new(stdin))
    };
    let err_tx = events_tx.clone();
    let stderr_tail = Arc::new(Mutex::new(String::new()));
    let captured = stderr_tail.clone();
    std::thread::spawn(move || {
        for line in BufReader::new(stderr).lines().map_while(Result::ok) {
            let mut tail = captured.lock().unwrap_or_else(|e| e.into_inner());
            if tail.len() + line.len() > 32_768 {
                tail.clear();
            }
            tail.push_str(&line);
            tail.push('\n');
            drop(tail);
            if !native
                && err_tx
                    .send(SessionEvent::Output(
                        format!("[codex] {line}\r\n").into_bytes(),
                    ))
                    .is_err()
            {
                break;
            }
        }
    });
    let (ready_tx, ready_rx) = mpsc::sync_channel(1);
    let spec = spec.clone();
    let worker_child = child.clone();
    std::thread::spawn(move || {
        worker::run(spec, input, native, rx, events_tx, ready_tx, worker_child)
    });
    let ready = ready_rx
        .recv_timeout(Duration::from_secs(30))
        .context("Codex initialization timed out");
    match ready {
        Ok(Ok(())) => Ok(StartedSession {
            session: Box::new(CodexSession { tx, child }),
            events: events_rx,
            msg_socket: None,
        }),
        outcome => {
            let _ = child.lock().unwrap_or_else(|e| e.into_inner()).kill();
            let detail = stderr_tail
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .clone();
            match outcome {
                Ok(Err(error)) | Err(error) => {
                    if detail.trim().is_empty() {
                        Err(error)
                    } else {
                        Err(error).context(detail.trim().to_string())
                    }
                }
                Ok(Ok(())) => unreachable!(),
            }
        }
    }
}

struct CodexSession {
    tx: mpsc::Sender<Wire>,
    child: Arc<Mutex<Child>>,
}

impl RuntimeSession for CodexSession {
    fn send_input(&mut self, bytes: &[u8]) -> anyhow::Result<()> {
        self.tx
            .send(Wire::Input(bytes.to_vec()))
            .map_err(|_| anyhow::anyhow!("Codex session stopped"))
    }
    fn deliver(&mut self, text: &str) -> Option<anyhow::Result<()>> {
        let (reply_tx, reply_rx) = mpsc::sync_channel(1);
        Some((|| {
            self.tx
                .send(Wire::Deliver(text.to_string(), reply_tx))
                .map_err(|_| anyhow::anyhow!("Codex session stopped"))?;
            reply_rx
                .recv_timeout(Duration::from_secs(12))
                .context("Codex delivery timed out")?
        })())
    }
    fn answer_permission(
        &mut self,
        key: u64,
        answer: super::PermissionAnswer,
    ) -> anyhow::Result<()> {
        self.tx
            .send(Wire::Answer(key, answer))
            .map_err(|_| anyhow::anyhow!("Codex session stopped"))
    }
    fn resize(&mut self, _cols: u16, _rows: u16) -> anyhow::Result<()> {
        Ok(())
    }
    fn kill(&mut self) -> anyhow::Result<()> {
        let mut child = self.child.lock().unwrap_or_else(|e| e.into_inner());
        if child.try_wait()?.is_none() {
            child.kill()?;
        }
        let _ = self.tx.send(Wire::Stop);
        Ok(())
    }
}

impl Drop for CodexSession {
    fn drop(&mut self) {
        let _ = self.kill();
    }
}

#[cfg(test)]
mod tests {
    use super::{server_command, BotSpec, CodexSpec};
    use std::ffi::OsStr;

    /// A Codex bot never sees the daemon's home variables, even when they
    /// reach its environment; see `withheld_env`.
    #[test]
    fn the_server_never_inherits_the_daemon_home() {
        let spec = BotSpec {
            codex: Some(CodexSpec {
                bin: "codex".into(),
                args: Vec::new(),
                port: 1,
                artifacts: None,
                browser: None,
                profile: bus::PermissionProfile::Standard,
                trusted_paths: Vec::new(),
            }),
            bot_id: "codex-test".into(),
            bot_name: "codex".into(),
            workspace: std::env::temp_dir(),
            claude_bin: String::new(),
            claude_args: Vec::new(),
            env: vec![
                ("THEHERMES_HOME".into(), "/home".into()),
                ("GRAVITY_HOME".into(), "/home".into()),
                ("THEHERMES_TOKEN".into(), "t".into()),
            ],
            cols: 80,
            rows: 24,
        };
        let command = server_command(&spec).unwrap();
        let envs: Vec<_> = command.get_envs().collect();
        for key in ["THEHERMES_HOME", "GRAVITY_HOME"] {
            assert!(envs.contains(&(OsStr::new(key), None)), "{key}: {envs:?}");
        }
        assert!(envs.contains(&(OsStr::new("THEHERMES_TOKEN"), Some(OsStr::new("t")))));
    }
}
