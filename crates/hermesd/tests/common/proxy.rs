//! A real `hermesd bus-proxy` talking to a test daemon's local endpoint
//! (H-044). `sh` waits for a line, then execs the proxy in its own process,
//! so a test can record that process (a session root, the owner's app)
//! before the proxy connects.

use std::process::Stdio;
use std::time::Duration;

use hermesd::bus_auth::os::OsProcessTable;
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};

use super::TestDaemon;

pub struct Proxy {
    child: Child,
    stdin: ChildStdin,
    stdout: Lines<BufReader<ChildStdout>>,
    next_id: u64,
}

impl Proxy {
    pub fn spawn(d: &TestDaemon) -> Self {
        let endpoint = hermesd::bus_auth::ipc::endpoint(&d.app.cfg);
        let script = format!(
            "read go; exec '{}' bus-proxy --endpoint '{}'",
            env!("CARGO_BIN_EXE_hermesd"),
            endpoint.display()
        );
        let mut child = Command::new("sh")
            .args(["-c", &script])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .expect("spawn sh");
        let stdin = child.stdin.take().expect("stdin");
        let stdout = BufReader::new(child.stdout.take().expect("stdout")).lines();
        Self {
            child,
            stdin,
            stdout,
            next_id: 1,
        }
    }

    pub fn pid(&self) -> u32 {
        self.child.id().expect("running")
    }

    /// Records this process as `bot_id`'s session root.
    pub fn root_of(&self, d: &TestDaemon, bot_id: &str) {
        d.app
            .supervisor
            .session_roots()
            .record_pid(&OsProcessTable, self.pid(), bot_id);
    }

    pub async fn start(&mut self) {
        self.stdin.write_all(b"go\n").await.expect("go");
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
