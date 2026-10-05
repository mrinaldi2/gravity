#![cfg(windows)]
use std::time::Duration;

use hermesd::runtime::double::DoubleAdapter;
use hermesd::runtime::{BotSpec, RuntimeAdapter, SessionEvent};

#[tokio::test]
async fn windows_inbox_delivers_authenticated_unicode_messages() {
    let spec = BotSpec {
        codex: None,
        bot_id: "windows-inbox".into(),
        bot_name: "Inbox".into(),
        workspace: std::env::temp_dir(),
        claude_bin: "unused".into(),
        claude_args: Vec::new(),
        env: Vec::new(),
        cols: 80,
        rows: 24,
    };
    let mut started = DoubleAdapter.start(&spec).expect("start");
    started.events.recv().await.expect("greeting");
    // The double's second startup line names the env it was given; drain it
    // so the next event is the inbox payload.
    let env_line = started.events.recv().await.expect("env line");
    assert!(
        matches!(&env_line, SessionEvent::Output(bytes) if bytes.starts_with(b"double runtime env ["))
    );
    let socket = started.msg_socket.clone().expect("inbox");
    assert!(socket.path.to_string_lossy().starts_with(r"\\.\pipe\"));
    let text = "Hello — 世界\n".repeat(10_000);
    let delivered = text.clone();
    tokio::task::spawn_blocking(move || hermesd::channel::send(&socket, &delivered))
        .await
        .expect("join")
        .expect("deliver");
    let event = tokio::time::timeout(Duration::from_secs(5), started.events.recv())
        .await
        .expect("timeout")
        .expect("output");
    assert!(
        matches!(event, SessionEvent::Output(bytes) if bytes == format!("{text}\r\n").as_bytes())
    );
    let mut socket = started.msg_socket.expect("inbox");
    socket.token = None;
    assert!(hermesd::channel::send(&socket, "unauthenticated").is_err());
    started.session.kill().expect("kill");
}
