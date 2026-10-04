//! Each bot's own browser: the owner's Chrome is opt-in per bot, the session
//! is configured with a browser of its own, and the app can watch it live.
//! A fake DevTools endpoint stands in for Chrome, so no browser is needed.

mod common;

use std::cell::Cell;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use common::devtools::{
    fake_devtools, frame_number, recording_devtools, start_browser, streaming_devtools,
};
use common::peers::{project, wait_until};
use common::*;
use serde_json::{json, Value};

fn bot_root(d: &TestDaemon, bot_id: &str) -> std::path::PathBuf {
    let bot = d.app.db.get_bot(bot_id).expect("db").expect("bot");
    std::path::PathBuf::from(&bot.workspace_path)
        .parent()
        .expect("root")
        .to_path_buf()
}

#[tokio::test]
async fn bots_use_their_own_browser_unless_allowed_the_owners_chrome() {
    let d = spawn_daemon_with(|cfg| {
        cfg.user_home = cfg.home.join("user");
        cfg.browser.node_dir = Some(cfg.home.join("node"));
    })
    .await;
    let mut c = WsClient::connect(&d).await;
    let pid = project(&mut c, "web").await;
    let bot = create_bot(&mut c, &pid, "surfer").await;
    let id = bot["id"].as_str().expect("id").to_string();
    assert_eq!(bot["user_chrome"], false);

    wait_until("the bot starts without the owner's Chrome", || {
        terminal(&d, &id).contains("--no-chrome")
    })
    .await;
    let mcp: Value = serde_json::from_str(
        &std::fs::read_to_string(bot_root(&d, &id).join("mcp.json")).expect("mcp.json"),
    )
    .expect("json");
    let browser = &mcp["mcpServers"]["playwright"];
    assert_eq!(browser["type"], "stdio", "{mcp}");
    assert!(browser["args"]
        .as_array()
        .expect("args")
        .iter()
        .any(|a| a.as_str().is_some_and(|a| a.ends_with("playwright.json"))));

    let allowed = c
        .request(json!({"type": "set_bot_user_chrome", "bot_id": id, "enabled": true}))
        .await;
    assert_eq!(allowed["type"], "bot", "{allowed}");
    assert_eq!(allowed["bot"]["user_chrome"], true);
    wait_until("the restarted session may use the owner's Chrome", || {
        terminal(&d, &id).matches("--chrome ").count() >= 1
    })
    .await;
    let system = std::fs::read_to_string(bot_root(&d, &id).join("system.md")).expect("system");
    assert!(system.contains("claude-in-chrome"));
}

#[tokio::test]
async fn the_app_watches_a_bots_browser_live() {
    let d = spawn_daemon_with(|cfg| cfg.user_home = cfg.home.join("user")).await;
    let mut c = WsClient::connect(&d).await;
    let pid = project(&mut c, "web").await;
    let bot = create_bot(&mut c, &pid, "surfer").await;
    let id = bot["id"].as_str().expect("id").to_string();

    // No browser yet: the watch says so, and keeps watching.
    let ok = c
        .request(json!({"type": "watch_browser", "bot_id": id}))
        .await;
    assert_eq!(ok["type"], "ok", "{ok}");
    let closed = c.wait_for(|v| v["type"] == "browser_tabs").await;
    assert_eq!(closed["open"], false);

    // The bot's browser comes up and writes its DevTools port to its profile.
    let screencasts = Arc::new(AtomicUsize::new(0));
    let port = fake_devtools("Example", screencasts.clone()).await;
    start_browser(&d, &id, port);

    let tabs = c
        .wait_for(|v| v["type"] == "browser_tabs" && v["open"] == true)
        .await;
    assert_eq!(tabs["tabs"][0]["title"], "Example");
    assert_eq!(tabs["active"], "tab-1");
    assert_eq!(tabs["following"], true);
    let frame = c.wait_for(|v| v["type"] == "browser_frame").await;
    assert_eq!(frame["bot_id"], id.as_str());
    assert_eq!(frame["tab_id"], "tab-1");
    assert_eq!(frame["data"], "SlBFRw==");
    assert_eq!(
        (frame["width"].as_u64(), frame["height"].as_u64()),
        (Some(800), Some(600))
    );

    // A second viewer (the phone, say) shares the stream: it gets the
    // current screen at once, and Chrome still serves one screencast.
    let mut phone = WsClient::connect(&d).await;
    let ok = phone
        .request(json!({"type": "watch_browser", "bot_id": id}))
        .await;
    assert_eq!(ok["type"], "ok");
    let shared = phone.wait_for(|v| v["type"] == "browser_frame").await;
    assert_eq!(shared["data"], "SlBFRw==");
    assert_eq!(screencasts.load(Ordering::SeqCst), 1);
    assert_eq!(d.app.browsers.live(), 1);

    // The stream lives while anyone watches, and stops when nobody does.
    let stopped = c.request(json!({"type": "unwatch_browser"})).await;
    assert_eq!(stopped["type"], "ok");
    assert_eq!(d.app.browsers.live(), 1);
    phone.request(json!({"type": "unwatch_browser"})).await;
    let app = d.app.clone();
    wait_until("the shared stream stops", || app.browsers.live() == 0).await;
}

#[tokio::test]
async fn the_owner_clicks_and_types_into_the_tab_on_show() {
    let d = spawn_daemon_with(|cfg| cfg.user_home = cfg.home.join("user")).await;
    let mut c = WsClient::connect(&d).await;
    let pid = project(&mut c, "web").await;
    let bot = create_bot(&mut c, &pid, "surfer").await;
    let id = bot["id"].as_str().expect("id").to_string();
    let (port, calls) = recording_devtools("Sign in").await;
    start_browser(&d, &id, port);

    // Input goes only to a tab someone is watching.
    c.send(
        json!({"type": "browser_input", "bot_id": id, "tab_id": "tab-1",
                  "event": {"kind": "text", "text": "early"}}),
    )
    .await;
    let refused = c.wait_for(|v| v["type"] == "error").await;
    assert!(
        refused["message"]
            .as_str()
            .is_some_and(|m| m.contains("not on show")),
        "{refused}"
    );

    c.request(json!({"type": "watch_browser", "bot_id": id}))
        .await;
    c.wait_for(|v| v["type"] == "browser_frame").await;
    for event in [
        json!({"kind": "mouse", "action": "down", "x": 40, "y": 80, "button": "left", "clicks": 1}),
        json!({"kind": "mouse", "action": "up", "x": 40, "y": 80, "button": "left", "clicks": 1}),
        json!({"kind": "key", "action": "down", "key": "a", "code": "KeyA", "key_code": 65, "text": "a"}),
        json!({"kind": "text", "text": "hunter2"}),
    ] {
        c.send(json!({"type": "browser_input", "bot_id": id, "tab_id": "tab-1", "event": event}))
            .await;
    }
    let seen = calls.clone();
    wait_until("the tab gets the owner's input", || {
        seen.lock().expect("calls").len() == 4
    })
    .await;
    let calls = calls.lock().expect("calls").clone();
    let methods: Vec<_> = calls.iter().map(|c| c["method"].clone()).collect();
    assert_eq!(
        methods,
        [
            "Input.dispatchMouseEvent",
            "Input.dispatchMouseEvent",
            "Input.dispatchKeyEvent",
            "Input.insertText"
        ]
    );
    assert_eq!(calls[0]["params"]["type"], "mousePressed");
    assert_eq!(calls[0]["params"]["x"], 40.0);
    assert_eq!(calls[2]["params"]["text"], "a");
    assert_eq!(calls[3]["params"]["text"], "hunter2");
}

#[tokio::test]
async fn a_stalled_viewer_skips_frames_instead_of_queuing_them() {
    const FRAMES: usize = 60;
    let d = spawn_daemon_with(|cfg| cfg.user_home = cfg.home.join("user")).await;
    let mut c = WsClient::connect(&d).await;
    let pid = project(&mut c, "web").await;
    let bot = create_bot(&mut c, &pid, "surfer").await;
    let id = bot["id"].as_str().expect("id").to_string();
    // A quarter megabyte of screen every 5 ms: far more, in all, than the
    // link holds while the phone is not reading.
    let (port, sent) = streaming_devtools(FRAMES, 256 * 1024, Duration::from_millis(5)).await;
    start_browser(&d, &id, port);
    let ok = c
        .request(json!({"type": "watch_browser", "bot_id": id}))
        .await;
    assert_eq!(ok["type"], "ok", "{ok}");

    // The phone stops reading (a dead zone) for as long as the screen keeps
    // changing, then asks for something once it is back. A debug build on a
    // busy machine takes its time over megabytes of frames, so the waits
    // here go by what arrives, not by the clock.
    let slow = Duration::from_secs(60);
    let deadline = tokio::time::Instant::now() + slow;
    while sent.load(Ordering::SeqCst) < FRAMES {
        assert!(
            tokio::time::Instant::now() < deadline,
            "the screencast stalled"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    tokio::time::sleep(Duration::from_millis(500)).await;
    let req_id = c.send(json!({"type": "list_projects"})).await;
    let frames_first = Cell::new(0);
    let reply = c
        .wait_for_within(slow, |v| {
            if v["type"] == "browser_frame" {
                frames_first.set(frames_first.get() + 1);
            }
            v["req_id"] == req_id.as_str()
        })
        .await;
    assert_eq!(reply["type"], "projects", "{reply}");
    // Only what was already on the wire comes first, not every frame.
    assert!(
        frames_first.get() < FRAMES / 2,
        "{} of {FRAMES} frames queued ahead of the reply",
        frames_first.get()
    );
    // The newest screen still arrives.
    c.wait_for_within(slow, |v| {
        v["type"] == "browser_frame" && frame_number(v) == FRAMES - 1
    })
    .await;
}

#[tokio::test]
async fn the_browser_log_says_what_each_step_did_and_why() {
    let d = spawn_daemon_with(|cfg| cfg.user_home = cfg.home.join("user")).await;
    let mut c = WsClient::connect(&d).await;
    let pid = project(&mut c, "web").await;
    let bot = create_bot(&mut c, &pid, "surfer").await;
    let id = bot["id"].as_str().expect("id").to_string();
    let workspace = bot["workspace_path"].as_str().expect("workspace");
    let mangled: String = workspace
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '-' })
        .collect();
    let dir = d.app.cfg.user_home.join(".claude/projects").join(mangled);
    std::fs::create_dir_all(&dir).expect("transcript dir");
    let records = [
        json!({"type": "user", "uuid": "turn-1", "timestamp": "2026-10-01T10:00:00Z",
               "isMeta": true, "origin": {"kind": "peer"},
               "message": {"content":
                   "Another Claude session sent a message:\n[msg #7 from USER · chat] check the pricing page"}}),
        json!({"type": "assistant", "uuid": "a1", "timestamp": "2026-10-01T10:00:02Z",
               "message": {"content": [
                   {"type": "tool_use", "id": "nav", "name": "mcp__playwright__browser_navigate",
                    "input": {"url": "https://example.com/pricing"}},
                   {"type": "tool_use", "id": "click", "name": "mcp__playwright__browser_click",
                    "input": {"element": "Plans tab", "ref": "e12"}},
                   {"type": "tool_use", "id": "own", "name": "mcp__claude-in-chrome__navigate",
                    "input": {"url": "https://mail.example.com/"}},
                   {"type": "tool_use", "id": "other", "name": "Bash",
                    "input": {"command": "ls"}}]}}),
    ];
    let lines: Vec<String> = records.iter().map(Value::to_string).collect();
    std::fs::write(dir.join("session.jsonl"), lines.join("\n") + "\n").expect("transcript");

    let log = c
        .request(json!({"type": "list_browser_activity", "bot_id": id}))
        .await;
    assert_eq!(log["type"], "browser_activity", "{log}");
    let activity = log["activity"].as_array().expect("activity");
    assert_eq!(activity.len(), 3, "{log}");
    // Newest first, each with its turn and what started it.
    assert_eq!(activity[0]["browser"], "owners_chrome");
    assert_eq!(activity[0]["title"], "Opened mail.example.com");
    assert_eq!(activity[1]["title"], "Clicked Plans tab");
    assert_eq!(activity[2]["browser"], "own");
    assert_eq!(activity[2]["subtitle"], "https://example.com/pricing");
    assert_eq!(activity[2]["turn_id"], "turn-1");
    assert_eq!(activity[2]["trigger"]["kind"], "owner");
}
