//! Owner actions on a linked computer (H-117 R3): a bot on the Mac proposes
//! a command for the PC; the PC keeps its own copy, and the owner's run on
//! the Mac executes that copy there, once, by its hash, with the output
//! back on the Mac's card. A link that is down refuses at once.

mod common;

use common::peers::{paired, project, wait_until, Paired};
use common::tasks::{drain_until, error_text};
use common::*;
use serde_json::{json, Value};

const FEATURES: &[&str] = &["permission_cards", "terminal_card", "owner_actions"];

struct Setup {
    p: Paired,
    win_app: String,
    bot: McpClient,
    dir: tempfile::TempDir,
}

/// The Mac's "app" is linked with the PC's, and has a devops bot.
async fn setup() -> Setup {
    let mut p = paired().await;
    let mac_app = project(&mut p.mac_client, "app").await;
    let win_app = project(&mut p.win_client, "app").await;
    let linked = p
        .mac_client
        .request(json!({"type": "link_project", "project_id": mac_app,
                        "peer_id": p.mac_peer_id, "remote_project_id": win_app}))
        .await;
    assert_eq!(linked["type"], "project", "{linked}");
    let win = &p.win;
    let peer = p.win_peer_id.clone();
    wait_until("the PC links the project too", || {
        win.app
            .db
            .project_link_by_remote(&peer, &mac_app)
            .unwrap()
            .is_some()
    })
    .await;
    let created = create_bot(&mut p.mac_client, &mac_app, "devops").await;
    let token = p
        .mac
        .app
        .secrets
        .bot_token(created["id"].as_str().unwrap())
        .unwrap();
    let bot = McpClient::new(&p.mac, &token);
    Setup {
        p,
        win_app,
        bot,
        dir: tempfile::tempdir().unwrap(),
    }
}

/// A shell the "PC" (this same test machine) runs: the target refuses a
/// shell its OS doesn't have.
const SHELL: &str = if cfg!(windows) { "powershell" } else { "zsh" };

impl Setup {
    async fn propose_raw(&mut self, content: &str) -> Value {
        let cwd = self.dir.path().display().to_string();
        self.bot
            .call_raw(
                "propose_owner_action",
                json!({"content": content, "reason": "stop the VM on the PC", "cwd": cwd,
                       "target_machine": "win", "shell": SHELL}),
            )
            .await
    }

    async fn propose(&mut self, content: &str) -> Value {
        let raw = self.propose_raw(content).await;
        let text = raw["content"][0]["text"].as_str().expect("text");
        let parsed: Value =
            serde_json::from_str(text).unwrap_or_else(|e| panic!("not a proposal ({e}): {raw}"));
        parsed["owner_action"].clone()
    }

    /// The owner's app on the Mac, by device credential with approve.
    async fn mac_app(&mut self) -> WsClient {
        let created = self
            .p
            .mac_client
            .request(json!({"type": "create_device", "name": "app",
                            "capabilities": ["read", "control", "approve"]}))
            .await;
        WsClient::connect_with(&self.p.mac, token_str(&created), FEATURES).await
    }

    fn state_on(d: &TestDaemon, id: &str) -> String {
        d.app
            .db
            .get_owner_action(id)
            .unwrap()
            .map(|a| a.state.as_str().to_string())
            .unwrap_or_default()
    }

    fn audit_on(d: &TestDaemon, id: &str) -> Vec<String> {
        d.app
            .db
            .owner_action_audit(id)
            .unwrap()
            .into_iter()
            .map(|(_, _, e)| e)
            .collect()
    }
}

#[tokio::test]
async fn an_action_for_the_pc_runs_there_once_as_shown() {
    let mut s = setup().await;
    let proposed = s.propose("echo on-the-pc; exit 0").await;
    let id = proposed["id"].as_str().unwrap().to_string();
    let sha = proposed["sha256"].as_str().unwrap().to_string();
    assert_eq!(proposed["target_name"], "win", "{proposed}");
    // The PC holds its own copy, the same hash, in its own project.
    let copy = s.p.win.app.db.get_owner_action(&id).unwrap().expect("copy");
    assert_eq!(copy.sha256, sha);
    assert_eq!(copy.project_here(), s.win_app);
    assert_eq!(copy.origin, format!("peer:{}", s.p.win_peer_id));

    let mut app = s.mac_app().await;
    let wrong = app
        .request(json!({"type": "owner_action_run", "id": id, "sha256": "0".repeat(64)}))
        .await;
    assert_eq!(wrong["code"], "conflict", "{wrong}");
    assert!(Setup::audit_on(&s.p.win, &id).contains(&"refused".to_string()));

    let run = json!({"type": "owner_action_run", "id": id, "sha256": sha});
    let started = app.request(run.clone()).await;
    assert_eq!(started["action"]["state"], "running", "{started}");
    // Output streams to the Mac's clients as it runs on the PC, in as many
    // chunks as the shell writes (WIN-CHK-7): all of them until it's done.
    let mut streamed = String::new();
    loop {
        let m = app
            .wait_for(|m| {
                (m["type"] == "owner_action_output" && m["id"] == json!(id))
                    || (m["type"] == "owner_action_update"
                        && m["action"]["id"] == json!(id)
                        && m["action"]["state"] != "running")
            })
            .await;
        if m["type"] == "owner_action_update" {
            break;
        }
        streamed.push_str(m["chunk"].as_str().unwrap_or_default());
    }
    assert!(streamed.contains("on-the-pc"), "streamed: {streamed:?}");
    let (mac, win) = (&s.p.mac, &s.p.win);
    wait_until("both copies finish", || {
        Setup::state_on(win, &id) == "succeeded" && Setup::state_on(mac, &id) == "succeeded"
    })
    .await;
    let mine = mac.app.db.get_owner_action(&id).unwrap().unwrap();
    assert_eq!(mine.exit_code, Some(0));
    assert!(mine.output_tail.unwrap().contains("on-the-pc"));
    // The PC keeps the full log and its audit.
    assert!(win
        .app
        .db
        .get_owner_action(&id)
        .unwrap()
        .unwrap()
        .output_path
        .is_some());
    assert_eq!(
        Setup::audit_on(win, &id),
        ["proposed", "refused", "run", "finished"]
    );
    // A second tap runs nothing.
    assert_eq!(app.request(run).await["code"], "conflict");
    // The bot on the Mac hears how it ended, once.
    let notes = drain_until(&mut s.bot, "succeeded").await;
    let told = notes
        .iter()
        .filter(|m| m["body"].as_str().unwrap().contains("succeeded"))
        .count();
    assert_eq!(told, 1, "{notes:?}");
}

#[tokio::test]
async fn the_pc_runs_only_what_the_mac_offered() {
    let s = setup().await;
    // An action the PC's own bot proposed there: the Mac never offered it.
    let win = &s.p.win.app;
    let proposal = hermesd::owner_action::model::Proposal {
        project_id: s.win_app.clone(),
        proposed_by: "bot:someone".into(),
        item_id: None,
        decision_id: None,
        target_machine: win.db.daemon_id().unwrap(),
        shell: hermesd::owner_action::model::Shell::Zsh,
        cwd: s.dir.path().display().to_string(),
        content: "echo never".into(),
        pinned_files: vec![],
        reason: "r".into(),
        timeout_s: 60,
    };
    let local = hermesd::owner_action::store(win, proposal, "bot", vec![]).unwrap();
    let forged =
        s.p.mac
            .app
            .peers
            .request(
                &s.p.mac_peer_id,
                json!({"type": "owner_action_run", "id": local.id, "sha256": local.sha256}),
            )
            .await
            .unwrap_err();
    assert_eq!(forged.code(), "not_found", "{forged}");
    assert_eq!(Setup::state_on(&s.p.win, &local.id), "proposed");
    assert_eq!(
        Setup::audit_on(&s.p.win, &local.id),
        ["proposed", "refused"]
    );
}

#[tokio::test]
async fn nothing_waits_for_a_pc_that_is_away() {
    let mut s = setup().await;
    // Rejected on the Mac: closed on the PC too, and the bot is told.
    let first = s.propose("echo one").await;
    let first_id = first["id"].as_str().unwrap().to_string();
    let mut app = s.mac_app().await;
    let rejected = app
        .request(json!({"type": "owner_action_reject", "id": first_id, "reason": "not now"}))
        .await;
    assert_eq!(rejected["action"]["state"], "rejected", "{rejected}");
    assert_eq!(Setup::state_on(&s.p.win, &first_id), "rejected");
    drain_until(&mut s.bot, "not now").await;

    let second = s.propose("echo two").await;
    let id = second["id"].as_str().unwrap().to_string();
    // The PC forgets the Mac: the link goes down and stays down.
    let (win, win_peer_id) = (&s.p.win, s.p.win_peer_id.clone());
    win.app.db.revoke_peer(&win_peer_id).unwrap();
    win.app.peers.disconnect(&win_peer_id);
    let (mac, mac_peer_id) = (&s.p.mac, s.p.mac_peer_id.clone());
    wait_until("the Mac loses the PC", || {
        !mac.app.peers.is_online(&mac_peer_id)
    })
    .await;

    let run = app
        .request(json!({"type": "owner_action_run", "id": id, "sha256": second["sha256"]}))
        .await;
    assert_eq!(run["code"], "unavailable", "{run}");
    assert!(
        run["message"].as_str().unwrap().contains("offline"),
        "{run}"
    );
    // Nothing was queued: both copies still wait, and the try is on record.
    assert_eq!(Setup::state_on(mac, &id), "proposed");
    assert_eq!(Setup::state_on(win, &id), "proposed");
    assert!(Setup::audit_on(mac, &id).contains(&"refused".to_string()));
    let withdrawn = s
        .bot
        .call_raw("withdraw_owner_action", json!({"id": id}))
        .await;
    assert!(error_text(&withdrawn).contains("offline"), "{withdrawn}");
    // And a new proposal is refused, not left waiting for it.
    let third = s.propose_raw("echo three").await;
    assert!(error_text(&third).contains("offline"), "{third}");
}
