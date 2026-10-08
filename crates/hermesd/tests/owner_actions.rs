//! Owner actions end to end (H-117 R1): a bot proposes, only the owner's
//! client runs it, once, exactly as shown, and everyone gets the redacted
//! result.

mod common;

use std::time::Duration;

use common::peers::{project, wait_until};
use common::tasks::{drain_until, error_text};
use common::*;
use serde_json::{json, Value};

const FEATURES: &[&str] = &["permission_cards", "terminal_card", "owner_actions"];

struct Setup {
    d: TestDaemon,
    bot: McpClient,
    owner: WsClient,
    dir: tempfile::TempDir,
}

async fn setup() -> Setup {
    let d = spawn_daemon().await;
    let mut owner = WsClient::connect(&d).await;
    let pid = project(&mut owner, "app").await;
    let created = create_bot(&mut owner, &pid, "devops").await;
    let token = d
        .app
        .secrets
        .bot_token(created["id"].as_str().unwrap())
        .unwrap();
    let bot = McpClient::new(&d, &token);
    Setup {
        d,
        bot,
        owner,
        dir: tempfile::tempdir().unwrap(),
    }
}

impl Setup {
    async fn propose(&mut self, content: &str) -> Value {
        let cwd = self.dir.path().display().to_string();
        self.bot
            .call(
                "propose_owner_action",
                json!({"content": content, "reason": "stop colima for the install", "cwd": cwd}),
            )
            .await["owner_action"]
            .clone()
    }

    /// The owner's app: owner_actions shown, approve held, by device token
    /// (the test process can't be told apart from the daemon by its pid).
    async fn app(&mut self, caps: &[&str]) -> WsClient {
        let created = self
            .owner
            .request(json!({"type": "create_device", "name": "app", "capabilities": caps}))
            .await;
        WsClient::connect_with(&self.d, token_str(&created), FEATURES).await
    }

    async fn wait_state(&self, id: &str, state: &str) -> Value {
        let db = &self.d.app.db;
        wait_until("the action finishes", || {
            db.get_owner_action(id)
                .unwrap()
                .is_some_and(|a| a.state.as_str() == state)
        })
        .await;
        json!(db.get_owner_action(id).unwrap().unwrap().to_json())
    }
}

#[tokio::test]
async fn the_owner_runs_a_proposal_once_exactly_as_shown() {
    let mut s = setup().await;
    let proposed = s
        .propose("echo stopping; echo token=0123456789abcdef0123456789abcdef01; exit 0")
        .await;
    let id = proposed["id"].as_str().unwrap().to_string();
    let sha = proposed["sha256"].as_str().unwrap().to_string();
    assert_eq!(proposed["state"], "proposed");

    // Bots can't run one: there is no tool for it.
    let tools = s.bot.tools().await.to_string();
    assert!(tools.contains("propose_owner_action") && !tools.contains("run_owner_action"));
    let refused = s.bot.call_raw("run_owner_action", json!({"id": id})).await;
    assert_eq!(refused["isError"], true, "{refused}");

    // A client that doesn't show owner actions, or holds no approve, can't.
    let mut plain = WsClient::connect_with(&s.d, s.d.app.secrets.client_token(), &[]).await;
    let run = json!({"type": "owner_action_run", "id": id, "sha256": sha});
    let reply = plain.request(run.clone()).await;
    assert_eq!(reply["code"], "forbidden", "{reply}");
    let mut viewer = s.app(&["read", "control"]).await;
    assert_eq!(viewer.request(run.clone()).await["code"], "forbidden");
    // The owner token is refused from anywhere, even from a process that
    // isn't a bot's (here: this test's own), and can't reject either
    // (ARCH-R51 M1): it holds no approve at all (CE-030 N1). The app's
    // credential runs it.
    let mut token = WsClient::connect_with(&s.d, s.d.app.secrets.client_token(), FEATURES).await;
    let reply = token.request(run.clone()).await;
    assert_eq!(reply["code"], "forbidden", "{reply}");
    let reject = token
        .request(json!({"type": "owner_action_reject", "id": id}))
        .await;
    assert_eq!(reject["code"], "forbidden", "{reject}");

    let mut app = s.app(&["read", "control", "approve"]).await;
    // Something else than what was shown: refused, and audited.
    let wrong = app
        .request(json!({"type": "owner_action_run", "id": id, "sha256": "0".repeat(64)}))
        .await;
    assert_eq!(wrong["code"], "conflict", "{wrong}");
    let started = app.request(run.clone()).await;
    assert_eq!(started["action"]["state"], "running", "{started}");
    // A second tap runs nothing.
    assert_eq!(app.request(run).await["code"], "conflict");

    let done = s.wait_state(&id, "succeeded").await;
    assert_eq!(done["exit_code"], 0);
    let tail = done["output_tail"].as_str().unwrap();
    assert!(
        tail.contains("stopping") && tail.contains("token=[redacted]"),
        "{tail}"
    );
    assert!(!tail.contains("0123456789abcdef0123"), "{tail}");
    // The full output stays in a 0600 log.
    let log =
        s.d.app
            .db
            .get_owner_action(&id)
            .unwrap()
            .unwrap()
            .output_path
            .unwrap();
    assert!(std::fs::read_to_string(&log)
        .unwrap()
        .contains("0123456789abcdef0123"));
    // The bot hears how it went.
    let notes = drain_until(&mut s.bot, "succeeded").await;
    assert!(
        notes
            .iter()
            .any(|m| m["body"].as_str().unwrap().contains("exit 0")),
        "{notes:?}"
    );
    let events: Vec<String> =
        s.d.app
            .db
            .owner_action_audit(&id)
            .unwrap()
            .into_iter()
            .map(|(_, _, e)| e)
            .collect();
    // No feature, no approve, the owner token's run and reject, wrong hash:
    // each refusal is on record, the ones at the gate too.
    assert_eq!(
        events,
        ["proposed", "refused", "refused", "refused", "refused", "refused", "run", "finished"]
    );
}

#[tokio::test]
async fn a_pinned_file_changed_since_the_proposal_is_refused_at_run() {
    let mut s = setup().await;
    let script = s.dir.path().join("stop.sh");
    std::fs::write(&script, "echo old\n").unwrap();
    let cwd = s.dir.path().display().to_string();
    let path = script.display().to_string();
    let proposed = s
        .bot
        .call(
            "propose_owner_action",
            json!({"content": format!("sh {path}"), "reason": "r", "cwd": cwd,
                   "pinned_files": [path]}),
        )
        .await["owner_action"]
        .clone();
    assert_eq!(proposed["flags"], json!([]), "pinned paths aren't flagged");
    std::fs::write(&script, "echo swapped\n").unwrap();
    let mut app = s.app(&["read", "control", "approve"]).await;
    let id = proposed["id"].as_str().unwrap();
    app.request(json!({"type": "owner_action_run", "id": id, "sha256": proposed["sha256"]}))
        .await;
    let done = s.wait_state(id, "failed").await;
    let tail = done["output_tail"].as_str().unwrap();
    assert!(
        tail.contains("pinned files changed") && tail.contains("stop.sh"),
        "{tail}"
    );
    assert!(!tail.contains("swapped"), "it never ran: {tail}");
}

#[tokio::test]
async fn proposals_hide_nothing_and_the_owner_can_reject() {
    let mut s = setup().await;
    let cwd = s.dir.path().display().to_string();
    let hidden = s
        .bot
        .call_raw(
            "propose_owner_action",
            json!({"content": "echo safe\u{202e}hs.lla", "reason": "r", "cwd": cwd}),
        )
        .await;
    assert!(error_text(&hidden).contains("bidirectional"), "{hidden}");
    let proposed = s.propose("echo hi").await;
    let id = proposed["id"].as_str().unwrap();
    let mut app = s.app(&["read", "control", "approve"]).await;
    let rejected = app
        .request(json!({"type": "owner_action_reject", "id": id, "reason": "not now"}))
        .await;
    assert_eq!(rejected["action"]["state"], "rejected", "{rejected}");
    let notes = drain_until(&mut s.bot, "rejected").await;
    assert!(notes
        .iter()
        .any(|m| m["body"].as_str().unwrap().contains("not now")));
    tokio::time::sleep(Duration::from_millis(10)).await;
}

/// The app's one-time ticket runs one: the owner's own app, not the token
/// file a bot could read (ARCH-R51 M1).
#[tokio::test]
async fn the_apps_ticket_runs_it() {
    let mut s = setup().await;
    let proposed = s.propose("echo by-ticket").await;
    let id = proposed["id"].as_str().unwrap().to_string();
    let ticket = s.d.app.owner.mint();
    let mut app = WsClient::connect_with(&s.d, &ticket, FEATURES).await;
    let started = app
        .request(json!({"type": "owner_action_run", "id": id, "sha256": proposed["sha256"]}))
        .await;
    assert_eq!(started["action"]["state"], "running", "{started}");
    s.wait_state(&id, "succeeded").await;
}
