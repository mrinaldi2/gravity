//! One conversation with each bot (H-192, UX-033): when a turn the owner
//! started from chat ends without `message_owner`, its final text joins the
//! owner thread, once. Other turns, and turns that already wrote to the
//! owner, add nothing.

mod common;

use std::io::Write;
use std::path::{Path, PathBuf};

use common::*;
use serde_json::{json, Value};

struct Setup {
    d: TestDaemon,
    c: WsClient,
    bot_id: String,
    transcript: PathBuf,
}

async fn setup() -> Setup {
    let d = spawn_daemon_with(|cfg| cfg.user_home = cfg.home.join("user")).await;
    let mut c = WsClient::connect(&d).await;
    let created = c
        .request(json!({"type": "create_project", "name": "app"}))
        .await;
    let project_id = created["project"]["id"].as_str().expect("pid").to_string();
    let bot = create_bot(&mut c, &project_id, "dev").await;
    let bot_id = bot["id"].as_str().expect("id").to_string();
    let mangled: String = bot["workspace_path"]
        .as_str()
        .expect("workspace")
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '-' })
        .collect();
    let dir = d.app.cfg.user_home.join(".claude/projects").join(mangled);
    std::fs::create_dir_all(&dir).expect("transcript dir");
    Setup {
        transcript: dir.join("session.jsonl"),
        d,
        c,
        bot_id,
    }
}

fn append(path: &Path, records: &[Value]) {
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .expect("open transcript");
    for record in records {
        writeln!(file, "{record}").expect("write");
    }
}

fn now() -> String {
    chrono::Utc::now().to_rfc3339()
}

/// What starts the turn, as the transcript records it.
enum Start<'a> {
    Chat(&'a str),
    Terminal(&'a str),
    Task(&'a str),
}

/// One whole turn: its start, the bot's records, and the transcript's own
/// end-of-turn record. Then the `Stop` hook, as Claude Code runs it.
fn turn(s: &Setup, id: &str, start: Start<'_>, records: &[Value]) {
    let opening = match start {
        Start::Chat(text) => json!({"type": "user", "uuid": id, "timestamp": now(),
            "isMeta": true, "origin": {"kind": "peer"}, "message": {"content": format!(
                "Another Claude session sent a message:\n[msg #7 from USER · chat] {text}")}}),
        Start::Terminal(text) => json!({"type": "user", "uuid": id, "timestamp": now(),
            "origin": {"kind": "human"}, "message": {"content": text}}),
        Start::Task(text) => json!({"type": "user", "uuid": id, "timestamp": now(),
            "isMeta": true, "origin": {"kind": "peer"}, "message": {"content": format!(
                "Another Claude session sent a message:\n[msg #8 from Lead · task] {text}")}}),
    };
    let mut all = vec![opening];
    all.extend_from_slice(records);
    all.push(
        json!({"type": "system", "subtype": "turn_duration", "durationMs": 900,
                    "uuid": format!("{id}-end"), "timestamp": now()}),
    );
    append(&s.transcript, &all);
    s.d.app
        .supervisor
        .on_hook(&s.bot_id, "UserPromptSubmit", None);
    s.d.app.supervisor.on_hook(&s.bot_id, "Stop", None);
}

fn says(id: &str, text: &str) -> Value {
    json!({"type": "assistant", "uuid": id, "timestamp": now(),
           "message": {"content": [{"type": "text", "text": text}]}})
}

/// The bot's own messages in its owner thread, oldest first.
async fn answers(s: &mut Setup) -> Vec<Value> {
    let page =
        s.c.request(json!({"type": "owner_thread_get", "bot_id": s.bot_id}))
            .await;
    assert_eq!(page["type"], "owner_thread", "{page}");
    // An empty thread leaves `messages` out (proto3 JSON).
    page["owner_thread"]["messages"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|m| m["from_owner"] != true)
        .cloned()
        .collect()
}

/// Waits until the bot has `n` messages in its thread.
async fn wait_answers(s: &mut Setup, n: usize) -> Vec<Value> {
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        let got = answers(s).await;
        if got.len() >= n || tokio::time::Instant::now() > deadline {
            return got;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
}

#[tokio::test]
async fn a_chat_answer_printed_in_the_session_joins_the_owner_thread_once() {
    let mut s = setup().await;
    turn(
        &s,
        "t1",
        Start::Chat("is the build green?"),
        &[
            says("a1", "Checking."),
            json!({"type": "assistant", "uuid": "a1b", "timestamp": now(),
                   "message": {"content": [{"type": "tool_use", "id": "run",
                       "name": "Bash", "input": {"command": "cargo test"}}]}}),
            json!({"type": "user", "uuid": "r1", "timestamp": now(),
                   "message": {"content": [{"type": "tool_result", "tool_use_id": "run",
                       "content": "test result: ok. 597 passed"}]}}),
            says("a2", "Green: 597 tests."),
        ],
    );
    let got = wait_answers(&mut s, 1).await;
    assert_eq!(got.len(), 1, "{got:?}");
    // The final text, after the work: not the "Checking." before it.
    assert_eq!(got[0]["text"], "Green: 597 tests.");

    // A later turn's hook re-reads the transcript: still once.
    turn(
        &s,
        "t2",
        Start::Chat("thanks"),
        &[says("a3", "You're welcome.")],
    );
    let got = wait_answers(&mut s, 2).await;
    let texts: Vec<&str> = got.iter().filter_map(|m| m["text"].as_str()).collect();
    assert_eq!(texts, ["Green: 597 tests.", "You're welcome."]);

    // Activity tags the turn with the message it became.
    let chat =
        s.c.request(json!({"type": "list_chat", "bot_id": s.bot_id}))
            .await;
    let first = &chat["turns"][0];
    assert_eq!(first["id"], "t1");
    // The thread spells message numbers as strings (proto u64).
    assert_eq!(
        first["answer_num"].to_string(),
        got[0]["num"].as_str().unwrap_or_default(),
        "{chat}"
    );
}

#[tokio::test]
async fn a_turn_that_called_message_owner_adds_nothing_more() {
    let mut s = setup().await;
    let note = json!({"type": "assistant", "uuid": "a1", "timestamp": now(),
        "message": {"content": [
            {"type": "tool_use", "id": "mo", "name": "mcp__hermes-bus__message_owner",
             "input": {"body": "Green, see the release."}}]}});
    turn(
        &s,
        "t1",
        Start::Chat("is the build green?"),
        &[note, says("a2", "Sent it to your Chat.")],
    );
    // A chat turn after it is answered, so the first one has been looked at.
    turn(&s, "t2", Start::Chat("ok"), &[says("a3", "Noted.")]);
    let got = wait_answers(&mut s, 1).await;
    let texts: Vec<&str> = got.iter().filter_map(|m| m["text"].as_str()).collect();
    assert_eq!(texts, ["Noted."], "{got:?}");

    // The note shows in Activity as sent to the owner.
    let chat =
        s.c.request(json!({"type": "list_chat", "bot_id": s.bot_id}))
            .await;
    let sent = &chat["turns"][0]["items"][0];
    assert_eq!(sent["type"], "sent", "{chat}");
    assert_eq!(sent["msg_kind"], "owner");
    assert_eq!(sent["body"], "Green, see the release.");
}

#[tokio::test]
async fn turns_from_the_terminal_or_a_task_are_not_posted() {
    let mut s = setup().await;
    turn(
        &s,
        "t1",
        Start::Terminal("ls please"),
        &[says("a1", "Here.")],
    );
    turn(
        &s,
        "t2",
        Start::Task("fix H-1"),
        &[says("a2", "Fixed H-1.")],
    );
    turn(&s, "t3", Start::Chat("done?"), &[says("a3", "Yes, done.")]);
    let got = wait_answers(&mut s, 1).await;
    let texts: Vec<&str> = got.iter().filter_map(|m| m["text"].as_str()).collect();
    assert_eq!(texts, ["Yes, done."], "{got:?}");
}

#[tokio::test]
async fn the_owner_answers_capability_is_advertised() {
    let d = spawn_daemon().await;
    let hello = raw_hello(&d, d.app.secrets.client_token()).await;
    let caps = hello["capabilities"].as_array().expect("capabilities");
    assert!(caps.contains(&json!("owner_answers")));
}
