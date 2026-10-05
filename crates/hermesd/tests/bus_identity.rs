//! Bus identity by process (H-044 §5, tests 1, 2, 4 and 8). A real
//! `hermesd bus-proxy` relays JSON-RPC to the daemon's local endpoint, and
//! the daemon decides who it is from its place under a recorded session
//! root, never from anything it sends. A stolen bearer token is worthless
//! once bearers are refused.
#![cfg(unix)]

mod common;

use std::process::Stdio;
use std::time::Duration;

use common::*;
use hermesd::bus_auth::os::OsProcessTable;
use hermesd::bus_auth::BearerPolicy;
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};

/// A `bus-proxy` started as a session would start it. `sh` waits for a line,
/// then execs the proxy in its own process, so the test can record that
/// process as a session root before the proxy connects.
struct Proxy {
    child: Child,
    stdin: ChildStdin,
    stdout: Lines<BufReader<ChildStdout>>,
    next_id: u64,
}

impl Proxy {
    fn spawn(d: &TestDaemon) -> Self {
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

    fn pid(&self) -> u32 {
        self.child.id().expect("running")
    }

    /// Records this process as `bot_id`'s session root.
    fn root_of(&self, d: &TestDaemon, bot_id: &str) {
        d.app
            .supervisor
            .session_roots()
            .record_pid(&OsProcessTable, self.pid(), bot_id);
    }

    async fn start(&mut self) {
        self.stdin.write_all(b"go\n").await.expect("go");
    }

    /// One JSON-RPC request; the reply, or `None` once the daemon has closed.
    async fn request(&mut self, method: &str, params: Value) -> Option<Value> {
        let id = self.next_id;
        self.next_id += 1;
        let line = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
        self.stdin
            .write_all(format!("{line}\n").as_bytes())
            .await
            .ok()?;
        let reply = tokio::time::timeout(Duration::from_secs(10), self.stdout.next_line())
            .await
            .expect("a reply in time")
            .ok()??;
        Some(serde_json::from_str(&reply).expect("json"))
    }

    /// `get_self` as whoever the daemon decides this process is.
    async fn whoami(&mut self) -> Result<String, Value> {
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

/// Two bots, "alice" and "bob", in one project; their ids and tokens.
async fn two_bots(d: &TestDaemon) -> [(String, String); 2] {
    let mut owner = WsClient::connect(d).await;
    let project = common::peers::project(&mut owner, "p").await;
    let mut out = Vec::new();
    for name in ["alice", "bob"] {
        let bot = create_bot(&mut owner, &project, name).await;
        let id = bot["id"].as_str().expect("id").to_string();
        let token = d.app.secrets.bot_token(&id).expect("token");
        out.push((id, token));
    }
    [out.remove(0), out.remove(0)]
}

#[tokio::test]
async fn each_proxy_is_the_bot_whose_session_it_runs_in() {
    let d = spawn_daemon().await;
    let [(alice, _), (bob, _)] = two_bots(&d).await;

    let mut a = Proxy::spawn(&d);
    a.root_of(&d, &alice);
    a.start().await;
    let mut b = Proxy::spawn(&d);
    b.root_of(&d, &bob);
    b.start().await;
    assert_eq!(a.whoami().await.expect("alice"), "alice");
    assert_eq!(b.whoami().await.expect("bob"), "bob");
    // Whatever B sends, it stays B: nothing in a request names a bot.
    let init = b.request("initialize", json!({"bot_id": alice})).await;
    assert!(init.expect("reply")["result"]["serverInfo"].is_object());
    assert_eq!(b.whoami().await.expect("still bob"), "bob");
}

#[tokio::test]
async fn a_process_in_no_session_is_refused() {
    let d = spawn_daemon().await;
    two_bots(&d).await;
    // Not under any recorded root: a terminal, or a process that detached.
    let mut stray = Proxy::spawn(&d);
    stray.start().await;
    let refused = stray.whoami().await.expect_err("refused");
    assert_eq!(
        refused["error"]["message"], "not a bot session",
        "{refused}"
    );
    // And the daemon hung up on it.
    assert!(stray.request("ping", json!({})).await.is_none());
}

#[tokio::test]
async fn a_restarted_session_cuts_off_the_old_one() {
    let d = spawn_daemon().await;
    let [(alice, _), _] = two_bots(&d).await;
    let mut old = Proxy::spawn(&d);
    old.root_of(&d, &alice);
    old.start().await;
    assert_eq!(old.whoami().await.expect("alice"), "alice");

    // The session restarts: a new root replaces the old one.
    let mut new = Proxy::spawn(&d);
    new.root_of(&d, &alice);
    new.start().await;
    let refused = old.whoami().await.expect_err("cut off");
    assert_eq!(refused["error"]["message"], "not a bot session");
    assert_eq!(new.whoami().await.expect("alice again"), "alice");
}

/// Phase 2 (tests 1 and 8): with bearers refused, a stolen token gets
/// nowhere, from any process, and the refusal names the bot it belonged to;
/// the bot's own proxy still works.
#[tokio::test]
async fn a_stolen_token_is_refused_once_bearers_are() {
    let d = spawn_daemon_with(|cfg| cfg.auth.bot_bearer = BearerPolicy::Refuse).await;
    let [(alice, alice_token), (bob, _)] = two_bots(&d).await;

    let status = reqwest::Client::new()
        .post(format!("http://{}/mcp", d.addr))
        .bearer_auth(&alice_token)
        .json(&json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}))
        .send()
        .await
        .expect("request")
        .status();
    assert_eq!(status, reqwest::StatusCode::UNAUTHORIZED);
    assert!(
        d.app.bearers.last_seen(&alice).is_some(),
        "the refusal names alice"
    );

    let mut a = Proxy::spawn(&d);
    a.root_of(&d, &alice);
    a.start().await;
    let mut b = Proxy::spawn(&d);
    b.root_of(&d, &bob);
    b.start().await;
    assert_eq!(a.whoami().await.expect("alice"), "alice");
    assert_eq!(b.whoami().await.expect("bob, never alice"), "bob");
}

/// Phase 1: a session that hasn't restarted still uses its bearer token.
#[tokio::test]
async fn bearers_still_work_while_both_are_accepted() {
    let d = spawn_daemon().await;
    let [(alice, alice_token), _] = two_bots(&d).await;
    let mut http = McpClient::new(&d, &alice_token);
    assert_eq!(http.call("get_self", json!({})).await["name"], "alice");
    assert!(d.app.bearers.last_seen(&alice).is_some());
}
