//! A board homed on the Mac and worked on from a linked PC (B9): the
//! fixture the two-daemon board tests share.

use super::peers::{bot_named, paired, project, wait_until, Paired};
use super::{create_bot, McpClient};
use hermesd::actor::Actor;
use hermesd::board::feed::{Change, ChangeKind};
use hermesd::board::model::{ItemType, Platform, Priority, ProjectRole, Role};
use hermesd::db::{MoveTo, NewItem};
use serde_json::json;

pub struct Board {
    pub p: Paired,
    pub mac_app: String,
    pub win_app: String,
    pub item: String,
    /// The tester on the PC, and its stand-in on the Mac.
    pub tester: McpClient,
    pub tester_id: String,
    pub stand_in: String,
}

/// The Mac's "app" holds the board with item H-1 assigned to the PC's
/// tester; the PC's "app" is linked to it and has mirrored the board.
pub async fn board() -> Board {
    let mut p = paired().await;
    let mac_app = project(&mut p.mac_client, "app").await;
    create_bot(&mut p.mac_client, &mac_app, "lead").await;
    let win_app = project(&mut p.win_client, "app").await;
    let tester = create_bot(&mut p.win_client, &win_app, "tester").await;
    let tester_id = tester["id"].as_str().expect("id").to_string();
    let linked = p
        .mac_client
        .request(json!({"type": "link_project", "project_id": mac_app,
                        "peer_id": p.mac_peer_id, "remote_project_id": win_app}))
        .await;
    assert_eq!(linked["type"], "project", "{linked}");
    let mac = &p.mac;
    wait_until("the tester stands in on the Mac", || {
        bot_named(mac, &mac_app, "tester").is_some()
    })
    .await;
    let stand_in = bot_named(mac, &mac_app, "tester").expect("stand-in").id;

    let db = &mac.app.db;
    db.ensure_board(&mac_app, &db.daemon_id().unwrap(), Some("H"))
        .unwrap();
    db.set_project_role(&ProjectRole {
        project_id: mac_app.clone(),
        role: Role::Tester,
        bot_id: stand_in.clone(),
        machine: Some("win".into()),
    })
    .unwrap();
    let item = db
        .create_item(
            &NewItem {
                project_id: &mac_app,
                item_type: ItemType::Feature,
                title: "Peer board",
                description: "",
                platforms: &[Platform::Daemon],
                size: None,
                priority: Priority::P1,
                labels: &[],
                parent_id: None,
                acceptance_criteria: &[],
            },
            &Actor::User,
        )
        .unwrap();
    db.assign_item(&item.id, item.version, Some(&stand_in), &Actor::User)
        .unwrap();
    // As starting the board does: the PC hears of it and mirrors it. Wait
    // for the snapshot taken after this change, not an earlier fetch (the
    // link's own) that found the board half made: a later one landing after
    // a test's watch would push a SettingsChanged and skew its seq.
    let started = mac.app.board.writer().publish(Change {
        project_id: &mac_app,
        kind: ChangeKind::SettingsChanged,
        item_id: "",
        card: None,
        from_column: None,
    });
    let win = &p.win;
    wait_until("the PC mirrors the board as of that change", || {
        win.app
            .board_mirror
            .get(&win_app)
            .is_some_and(|m| m.snapshot.seq >= started && !m.snapshot.cards.is_empty())
    })
    .await;
    let token = win.app.secrets.bot_token(&tester_id).expect("token");
    Board {
        tester: McpClient::new(win, &token),
        p,
        mac_app,
        win_app,
        item: item.id,
        tester_id,
        stand_in,
    }
}

/// A release of H-1, homed on the Mac, tested by the PC's tester and
/// approved by the owner: DevOps (a bot on the Mac) and the release's id.
pub async fn approved_release(b: &mut Board) -> (McpClient, String) {
    let (mac, mac_app) = (&b.p.mac, b.mac_app.clone());
    let devops = create_bot(&mut b.p.mac_client, &mac_app, "devops").await;
    let devops_id = devops["id"].as_str().expect("id").to_string();
    let db = &mac.app.db;
    db.set_project_role(&ProjectRole {
        project_id: mac_app.clone(),
        role: Role::Devops,
        bot_id: devops_id.clone(),
        machine: None,
    })
    .unwrap();
    let item = db.get_item(&b.item).unwrap().unwrap();
    let to = MoveTo {
        column: "verify",
        ..MoveTo::default()
    };
    db.move_item(&item.id, item.version, &to, &Actor::User)
        .unwrap();
    let mut ops = McpClient::new(mac, &mac.app.secrets.bot_token(&devops_id).expect("token"));

    let created = ops
        .call(
            "release_create",
            json!({"name": "0.16.0", "items": [b.item]}),
        )
        .await;
    let id = created["release"]["id"].as_str().expect("id").to_string();
    ops.call(
        "release_attach_build",
        json!({"release_id": id, "platform": "daemon", "version": "0.16.0",
               "artifact": "/builds/0.16.0", "url": "https://dl.example/0.16.0.tar.gz",
               "sha256": "a".repeat(64)}),
    )
    .await;
    b.tester
        .call(
            "release_test",
            json!({"release_id": id, "machine": "win", "build_sha256": "a".repeat(64),
                   "result": "pass"}),
        )
        .await;
    let release = ops.call("release_submit", json!({"release_id": id})).await["release"].clone();
    let ship = json!([{"item_id": b.item, "verdict": "ship"}]);
    let ruled =
        b.p.mac_client
            .request(
                json!({"type": "release_rule", "release_id": id, "verdicts": ship,
                        "expected_version": release["version"]}),
            )
            .await;
    assert_eq!(ruled["release"]["status"], "approved", "{ruled}");
    (ops, id)
}
