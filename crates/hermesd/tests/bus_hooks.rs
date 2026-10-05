//! Hooks by process identity (H-044 §5, tests 5 and 10). A real `hermesd
//! hook` reports to the daemon's local endpoint as the bot whose session it
//! runs in, with no token; and once bearers are refused, a session's
//! environment holds none.
#![cfg(unix)]

mod common;

use std::process::Stdio;
use std::time::Duration;

use common::*;
use hermesd::bus_auth::os::OsProcessTable;
use hermesd::bus_auth::BearerPolicy;
use serde_json::{json, Value};
use tokio::io::AsyncWriteExt;
use tokio::process::{Child, Command};

/// One bot in a fresh project, once its session runs; its id.
async fn bot(d: &TestDaemon, name: &str) -> String {
    let mut owner = WsClient::connect(d).await;
    let project = common::peers::project(&mut owner, "p").await;
    let bot = create_bot(&mut owner, &project, name).await;
    let id = bot["id"].as_str().expect("id").to_string();
    eventually(|| d.app.supervisor.msg_socket_path(&id).is_some()).await;
    id
}

async fn eventually(check: impl Fn() -> bool) {
    for _ in 0..200 {
        if check() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("condition never held");
}

/// Starts `hermesd hook <event>` as Claude Code would, with `payload` on
/// stdin and `socket` as the session's inbox. `sh` waits for a line and then
/// execs the hook in its own process, so `root_of` can record that process
/// as a bot's session root first.
async fn spawn_hook(
    d: &TestDaemon,
    event: &str,
    payload: &str,
    socket: &str,
    root_of: Option<&str>,
) -> Child {
    let endpoint = hermesd::bus_auth::ipc::endpoint(&d.app.cfg);
    let script = format!(
        "read go; exec '{}' hook {event} --endpoint '{}'",
        env!("CARGO_BIN_EXE_hermesd"),
        endpoint.display()
    );
    let mut child = Command::new("sh")
        .args(["-c", &script])
        .env("CLAUDE_CODE_MESSAGING_SOCKET", socket)
        .env("CLAUDE_CODE_MESSAGING_TOKEN", "msg-token")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .expect("spawn sh");
    if let Some(bot_id) = root_of {
        let pid = child.id().expect("running");
        d.app
            .supervisor
            .session_roots()
            .record_pid(&OsProcessTable, pid, bot_id);
    }
    let mut stdin = child.stdin.take().expect("stdin");
    stdin
        .write_all(format!("go\n{payload}").as_bytes())
        .await
        .expect("payload");
    child
}

/// Waits for a hook: its exit code, stdout and stderr.
async fn finish(child: Child) -> (Option<i32>, String, String) {
    let out = tokio::time::timeout(Duration::from_secs(10), child.wait_with_output())
        .await
        .expect("the hook ends")
        .expect("output");
    let text = |bytes: &[u8]| String::from_utf8_lossy(bytes).to_string();
    (out.status.code(), text(&out.stdout), text(&out.stderr))
}

async fn session_start(
    d: &TestDaemon,
    socket: &str,
    root_of: Option<&str>,
) -> (Option<i32>, String) {
    let payload = r#"{"hook_event_name":"SessionStart","source":"startup"}"#;
    let (code, _, stderr) =
        finish(spawn_hook(d, "SessionStart", payload, socket, root_of).await).await;
    (code, stderr)
}

/// Test 5: the hook registers its own bot's inbox socket, and a process in
/// no session is refused without registering anything.
#[tokio::test]
async fn session_start_registers_its_own_bots_socket() {
    let d = spawn_daemon().await;
    let alice = bot(&d, "alice").await;
    let before = d.app.supervisor.msg_socket_path(&alice);

    let (code, stderr) = session_start(&d, "/tmp/stray.sock", None).await;
    assert_eq!(code, Some(0), "a failed hook never blocks the session");
    assert!(stderr.contains("not a bot session"), "{stderr}");
    assert_eq!(d.app.supervisor.msg_socket_path(&alice), before);

    let (code, stderr) = session_start(&d, "/tmp/alice.sock", Some(&alice)).await;
    assert_eq!(code, Some(0), "{stderr}");
    assert_eq!(
        d.app.supervisor.msg_socket_path(&alice),
        Some("/tmp/alice.sock".into()),
        "{stderr}"
    );
}

/// The owner's answer to a prompt comes back over the endpoint as the
/// hook's output, for the bot whose session asked.
#[tokio::test]
async fn a_permission_prompt_is_answered_through_the_hook() {
    let d = spawn_daemon().await;
    let mut owner = WsClient::connect(&d).await;
    let project = common::peers::project(&mut owner, "p").await;
    let bot = create_bot(&mut owner, &project, "alice").await;
    let alice = bot["id"].as_str().expect("id").to_string();
    let prompt = json!({"tool_name": "Bash", "tool_input": {"command": "make clean"}});
    let hook = spawn_hook(
        &d,
        "PermissionRequest",
        &prompt.to_string(),
        "",
        Some(&alice),
    )
    .await;

    let card = owner.wait_for(|v| v["type"] == "permission_request").await;
    assert_eq!(card["request"]["bot_id"], alice.as_str(), "{card}");
    owner
        .request(json!({"type": "answer_permission", "request_id": card["request"]["id"], "decision": "allow_once"}))
        .await;
    let (code, stdout, stderr) = finish(hook).await;
    assert_eq!(code, Some(0), "{stderr}");
    let output: Value = serde_json::from_str(&stdout).expect("decision json");
    assert_eq!(
        output["hookSpecificOutput"]["decision"]["behavior"],
        "allow"
    );
}

/// H-038: SessionStart keeps trying a daemon that isn't listening yet.
#[tokio::test]
async fn session_start_waits_for_a_daemon_still_booting() {
    let dir = tempfile::tempdir().expect("tmp");
    let endpoint = dir.path().join("bus.sock");
    let mut hook = Command::new(env!("CARGO_BIN_EXE_hermesd"))
        .args(["hook", "SessionStart", "--endpoint"])
        .arg(&endpoint)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .expect("spawn hook");
    tokio::time::sleep(Duration::from_millis(1500)).await;
    assert!(hook.try_wait().expect("status").is_none(), "still retrying");
    // The daemon comes up: the hook connects and gets its answer.
    let listener = tokio::net::UnixListener::bind(&endpoint).expect("bind");
    let (stream, _) = tokio::time::timeout(Duration::from_secs(5), listener.accept())
        .await
        .expect("the hook connects")
        .expect("accept");
    let (reader, mut writer) = stream.into_split();
    let mut lines = tokio::io::AsyncBufReadExt::lines(tokio::io::BufReader::new(reader));
    let request: Value = serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
    assert_eq!(request["params"]["event"], "SessionStart");
    writer
        .write_all(b"{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{}}\n")
        .await
        .expect("reply");
    let status = tokio::time::timeout(Duration::from_secs(5), hook.wait())
        .await
        .expect("the hook ends")
        .expect("status");
    assert!(status.success());
}

/// The names of the variables the double runtime was started with.
fn session_env(d: &TestDaemon, bot_id: &str) -> String {
    let text = terminal(d, bot_id);
    let start = text.rfind("double runtime env [").expect("env line");
    let line = &text[start..];
    line[..line.find(']').expect("end")].to_string()
}

/// Test 10: once bearers are refused, no session gets a token to steal.
#[tokio::test]
async fn no_token_in_a_session_once_bearers_are_refused() {
    let d = spawn_daemon_with(|cfg| cfg.auth.bot_bearer = BearerPolicy::Refuse).await;
    let alice = bot(&d, "alice").await;
    let env = session_env(&d, &alice);
    assert!(!env.contains("THEHERMES_TOKEN"), "{env}");
    assert!(!env.contains("GRAVITY_TOKEN"), "{env}");
}

/// Phase 1 still sets it, for sessions on the HTTP rollback.
#[tokio::test]
async fn the_token_is_still_set_while_bearers_are_accepted() {
    let d = spawn_daemon().await;
    let alice = bot(&d, "alice").await;
    let env = session_env(&d, &alice);
    assert!(env.contains("THEHERMES_TOKEN"), "{env}");
    assert!(env.contains("GRAVITY_TOKEN"), "{env}");
}
