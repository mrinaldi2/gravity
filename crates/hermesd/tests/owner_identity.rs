//! The owner without `client.token` (H-044 §5, tests 6 and 7). The desktop
//! app gets a one-time ticket because the code check says it's the pinned
//! app; a CLI owner command gets one only after the owner allows it on a
//! card. Bots get neither. The real signature check can't run in CI, so a
//! double stands in for it, as the spec asks.

mod common;

use std::sync::Arc;
use std::time::Duration;

use common::proxy::{is_launched_by, Proxy};
use common::*;
use hermesd::bus_auth::owner::{self, CodeCheck, Peer};
use serde_json::{json, Value};

/// The owner's app is the proxy with this launcher pid, and nothing else.
struct AppIs(u32);

impl CodeCheck for AppIs {
    fn is_owner_app(&self, peer: &Peer) -> bool {
        is_launched_by(self.0, peer.pid)
    }
}

async fn ask(proxy: &mut Proxy, method: &str, params: Value, within: u64) -> Value {
    proxy
        .answer(method, params, Duration::from_secs(within))
        .await
}

fn ticket_of(reply: &Value) -> String {
    reply["result"]["ticket"]
        .as_str()
        .unwrap_or_else(|| panic!("no ticket: {reply}"))
        .to_string()
}

/// Test 6: the app gets a ticket that makes one connection the owner's; any
/// other process, and any bot, gets none.
#[tokio::test]
async fn the_pinned_app_is_the_owner_and_nothing_else_is() {
    let d = spawn_daemon().await;
    let mut app = Proxy::spawn(&d);
    d.app.owner.set_check(Arc::new(AppIs(app.pid())));
    app.start().await;
    let ticket = ticket_of(&ask(&mut app, "hermes/owner_ticket", json!({}), 10).await);

    let hello = raw_hello(&d, &ticket).await;
    assert_eq!(hello["type"], "hello_ok", "{hello}");
    assert!(hello["grants"].to_string().contains("approve"), "{hello}");
    // One connection per ticket.
    let again = raw_hello(&d, &ticket).await;
    assert_eq!(again["code"], "auth_failed", "{again}");

    // Another process: a bot's tool, or a lookalike binary.
    let mut other = Proxy::spawn(&d);
    other.start().await;
    let refused = ask(&mut other, "hermes/owner_ticket", json!({}), 10).await;
    assert_eq!(
        refused["error"]["message"], "not the owner's app",
        "{refused}"
    );

    // A bot, even one the check would pass, never acts as the owner.
    let mut owner = WsClient::connect(&d).await;
    let project = common::peers::project(&mut owner, "p").await;
    let bot = create_bot(&mut owner, &project, "alice").await;
    let mut session = Proxy::spawn(&d);
    session.root_of(&d, bot["id"].as_str().expect("id"));
    d.app.owner.set_check(Arc::new(AppIs(session.pid())));
    session.start().await;
    let refused = ask(&mut session, "hermes/owner_ticket", json!({}), 10).await;
    assert_eq!(refused["error"]["message"], "a bot can't act as the owner");
}

/// Test 7: a CLI owner command waits for the owner's card and runs only
/// when allowed; a denial, silence or no app to ask refuses it.
#[tokio::test]
async fn a_cli_owner_command_runs_only_when_the_owner_allows_it() {
    // The default card timeout: the owner's answer never races the clock,
    // however loaded the machine (WIN-CHK-3). Expiry has its own daemon.
    let d = spawn_daemon().await;
    let request = json!({ "command": "hermesd board import --dry-run" });

    // No app open to ask: refused at once.
    let mut alone = Proxy::spawn(&d);
    alone.start().await;
    let refused = ask(&mut alone, "hermes/owner_request", request.clone(), 10).await;
    assert_eq!(refused["error"]["message"], owner::NO_APP);

    // The owner's app is open and answers the card.
    let mut owner = WsClient::connect(&d).await;
    for (decision, allowed) in [("allow_once", true), ("deny", false)] {
        let mut cli = Proxy::spawn(&d);
        cli.start().await;
        let asking = tokio::spawn(async move {
            let reply = ask(
                &mut cli,
                "hermes/owner_request",
                json!({ "command": "hermesd board import" }),
                20,
            )
            .await;
            (cli, reply)
        });
        let card = owner
            .wait_for(|v| v["type"] == "permission_request" && v["request"]["bot_id"] == "terminal")
            .await;
        assert_eq!(
            card["request"]["origin"]["command"], "hermesd board import",
            "{card}"
        );
        let answered = owner
            .request(json!({"type": "answer_permission",
                            "request_id": card["request"]["id"], "decision": decision}))
            .await;
        assert_eq!(answered["type"], "permission", "{answered}");
        let (_cli, reply) = asking.await.expect("cli");
        if allowed {
            let hello = raw_hello(&d, &ticket_of(&reply)).await;
            assert_eq!(hello["type"], "hello_ok", "{hello}");
        } else {
            assert_eq!(reply["error"]["message"], owner::DENIED, "{reply}");
        }
    }

    // Left unanswered, the card closes and the command is refused.
    let d = spawn_daemon_with(|cfg| cfg.permission_timeout_seconds = 2).await;
    let _owner = WsClient::connect(&d).await;
    let mut late = Proxy::spawn(&d);
    late.start().await;
    let refused = ask(&mut late, "hermes/owner_request", request, 20).await;
    assert_eq!(refused["error"]["message"], owner::EXPIRED, "{refused}");
}

/// UX-014: the card gets the command and where it came from as separate
/// fields, and names the bot when the command runs inside its workspace.
#[tokio::test]
async fn a_terminal_card_says_where_the_command_came_from() {
    let d = spawn_daemon().await;
    let mut owner = WsClient::connect(&d).await;
    let project = common::peers::project(&mut owner, "p").await;
    let bot = create_bot(&mut owner, &project, "Desktop Dev").await;
    let workspace = std::path::PathBuf::from(bot["workspace_path"].as_str().expect("workspace"));
    std::fs::create_dir_all(workspace.join("sub")).expect("workspace");
    let real = |p: &std::path::Path| std::fs::canonicalize(p).expect("real path");

    for (dir, bot_name) in [
        (workspace.join("sub"), Some("Desktop Dev")),
        (d._home.path().to_path_buf(), None),
    ] {
        let mut cli = Proxy::spawn_in(&d, &dir);
        let launcher = cli.pid();
        cli.start().await;
        // The client reports its directory, as the real CLI does; Windows
        // can't read another process's, so there the card relies on it.
        let request = json!({ "command": "hermesd board import --dry-run", "cwd": dir });
        let asking = tokio::spawn(async move {
            let reply = ask(&mut cli, "hermes/owner_request", request, 20).await;
            (cli, reply)
        });
        let card = owner
            .wait_for(|v| v["type"] == "permission_request" && v["request"]["bot_id"] == "terminal")
            .await;
        let origin = &card["request"]["origin"];
        assert_eq!(
            origin["command"], "hermesd board import --dry-run",
            "{origin}"
        );
        // Windows has no exec: the launcher runs the proxy as its child.
        let asker = origin["pid"].as_u64().expect("pid") as u32;
        assert!(is_launched_by(launcher, asker), "{origin}");
        assert_eq!(origin["process"], "hermesd", "{origin}");
        let cwd = std::path::PathBuf::from(origin["cwd"].as_str().expect("cwd"));
        assert_eq!(real(&cwd), real(&dir), "{origin}");
        match bot_name {
            Some(name) => assert_eq!(origin["bot"], name, "{origin}"),
            None => assert!(origin.get("bot").is_none(), "{origin}"),
        }
        owner
            .request(json!({"type": "answer_permission",
                            "request_id": card["request"]["id"], "decision": "deny"}))
            .await;
        let (_cli, reply) = asking.await.expect("cli");
        assert_eq!(reply["error"]["message"], owner::DENIED, "{reply}");
    }
}
