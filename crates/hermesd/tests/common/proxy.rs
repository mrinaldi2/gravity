//! A real `hermesd bus-proxy` talking to a test daemon's local endpoint
//! (H-044). A launcher process waits, then runs the proxy, so a test can
//! record the launcher (a session root, the owner's app) before the proxy
//! connects:
//! - unix: `sh` reads a line, then execs the proxy in its own process;
//! - Windows: a batch file waits for a flag file, then runs the proxy as its
//!   child (there is no exec), so the launcher is the proxy's parent.

use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use hermesd::bus_auth::os::OsProcessTable;
use hermesd::bus_auth::session::ProcessTable;
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};

use super::TestDaemon;

pub struct Proxy {
    child: Child,
    stdin: ChildStdin,
    stdout: Lines<BufReader<ChildStdout>>,
    next_id: u64,
    /// Windows: created by `start` to let the launcher run the proxy.
    #[cfg_attr(unix, allow(dead_code))]
    go: PathBuf,
}

#[cfg(unix)]
fn launcher(d: &TestDaemon, _go: &std::path::Path) -> Command {
    let endpoint = hermesd::bus_auth::ipc::endpoint(&d.app.cfg);
    let script = format!(
        "read go; exec '{}' bus-proxy --endpoint '{}'",
        env!("CARGO_BIN_EXE_hermesd"),
        endpoint.display()
    );
    let mut command = Command::new("sh");
    command.args(["-c", &script]);
    command
}

#[cfg(windows)]
fn launcher(d: &TestDaemon, go: &std::path::Path) -> Command {
    let endpoint = hermesd::bus_auth::ipc::endpoint(&d.app.cfg);
    let script = go.with_extension("cmd");
    std::fs::write(
        &script,
        "@echo off\r\n:wait\r\nif not exist \"%~1\" (ping -n 1 -w 20 127.0.0.1 >nul & goto wait)\r\n\"%~2\" bus-proxy --endpoint \"%~3\"\r\n",
    )
    .expect("launcher script");
    let mut command = Command::new("cmd");
    command
        .args(["/d", "/q", "/c"])
        .arg(&script)
        .arg(go)
        .arg(env!("CARGO_BIN_EXE_hermesd"))
        .arg(endpoint);
    command
}

impl Proxy {
    pub fn spawn(d: &TestDaemon) -> Self {
        let go = std::env::temp_dir().join(format!("hermes-proxy-{}.go", uuid::Uuid::new_v4()));
        let mut child = launcher(d, &go)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .expect("spawn the launcher");
        let stdin = child.stdin.take().expect("stdin");
        let stdout = BufReader::new(child.stdout.take().expect("stdout")).lines();
        Self {
            child,
            stdin,
            stdout,
            next_id: 1,
            go,
        }
    }

    /// The launcher: the proxy itself on unix, its parent on Windows.
    pub fn pid(&self) -> u32 {
        self.child.id().expect("running")
    }

    /// Whether `pid` is this proxy: the launcher or its direct child.
    pub fn is(&self, pid: u32) -> bool {
        is_launched_by(self.pid(), pid)
    }

    /// Records this process as `bot_id`'s session root.
    pub fn root_of(&self, d: &TestDaemon, bot_id: &str) {
        d.app
            .supervisor
            .session_roots()
            .record_pid(&OsProcessTable, self.pid(), bot_id);
    }

    pub async fn start(&mut self) {
        #[cfg(unix)]
        self.stdin.write_all(b"go\n").await.expect("go");
        #[cfg(windows)]
        std::fs::write(&self.go, b"go").expect("go");
    }

    /// One JSON-RPC request; the reply, or `None` once the daemon has closed.
    pub async fn request(&mut self, method: &str, params: Value) -> Option<Value> {
        self.request_within(method, params, Duration::from_secs(10))
            .await
    }

    pub async fn request_within(
        &mut self,
        method: &str,
        params: Value,
        within: Duration,
    ) -> Option<Value> {
        let id = self.next_id;
        self.next_id += 1;
        let line = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
        self.stdin
            .write_all(format!("{line}\n").as_bytes())
            .await
            .ok()?;
        let reply = tokio::time::timeout(within, self.stdout.next_line())
            .await
            .expect("a reply in time")
            .ok()??;
        Some(serde_json::from_str(&reply).expect("json"))
    }

    /// `get_self` as whoever the daemon decides this process is.
    pub async fn whoami(&mut self) -> Result<String, Value> {
        let reply = self
            .request("tools/call", json!({"name": "get_self", "arguments": {}}))
            .await
            .ok_or(Value::Null)?;
        if reply.get("error").is_some() {
            return Err(reply);
        }
        let text = reply["result"]["content"][0]["text"]
            .as_str()
            .expect("text");
        let me: Value = serde_json::from_str(text).expect("payload");
        Ok(me["name"].as_str().expect("name").to_string())
    }
}

/// `pid` is `launcher` itself or a process it started.
pub fn is_launched_by(launcher: u32, pid: u32) -> bool {
    pid == launcher || OsProcessTable.info(pid).is_some_and(|p| p.ppid == launcher)
}

impl Drop for Proxy {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.go);
        let _ = std::fs::remove_file(self.go.with_extension("cmd"));
    }
}
