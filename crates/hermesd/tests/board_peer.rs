//! A board homed on one daemon, worked on from a linked one (B9, H-020
//! §1.3): the Mac holds it, a tester on the PC calls the board tools through
//! its own daemon, which forwards them to run as its stand-in on the Mac.
//! The PC mirrors the board, its clients watch it move, and while the Mac is
//! unreachable the board there is read-only.

mod common;

use bus::contract::board::{self as c, board_request::Request, board_response::Response};
use bus::contract::wire::envelope::Body;
use common::board::*;
use common::peer_board::board;
use common::peers::wait_until;
use common::*;
use hermesd::actor::Actor;
use hermesd::board::model::{ProjectRole, Role};
use hermesd::db::MoveTo;
use serde_json::json;

fn watch(project_id: &str) -> Request {
    Request::BoardWatch(c::BoardWatch {
        project_id: project_id.to_string(),
    })
}

/// The PC's bot reads and writes the Mac's board as its stand-in there, and
/// sees itself, not its stand-in, as the assignee.
#[tokio::test]
async fn a_linked_bot_works_on_the_board_through_its_home() {
    let mut b = board().await;
    let got = b.tester.call("board_get", json!({})).await;
    assert_eq!(got["key"], "H", "{got}");
    let item = b.tester.call("item_get", json!({"id": b.item})).await;
    assert_eq!(item["item"]["assignee"], json!(b.tester_id), "{item}");

    b.tester
        .call(
            "item_comment",
            json!({"id": b.item, "body": "passes on win"}),
        )
        .await;
    let events = b.p.mac.app.db.item_events(&b.item).unwrap();
    let comment = events.last().expect("the comment");
    assert_eq!(comment.actor, format!("bot:{}", b.stand_in), "{comment:?}");

    // Its roles on the home decide: a tester doesn't assign work.
    let refused = b
        .tester
        .call_raw("item_assign", json!({"id": b.item, "assignee": "lead"}))
        .await;
    assert_eq!(refused["isError"], true, "{refused}");
    assert!(refused.to_string().contains("board role"), "{refused}");
}

/// A client on the PC watches the mirrored board, in its own ids, and sees
/// the home's changes arrive as pushes; item details and moves stay on the
/// home, and so do releases.
#[tokio::test]
async fn the_home_relays_its_changes_to_a_linked_computers_clients() {
    let mut b = board().await;
    let board = snapshot(call(&mut b.p.win_client, watch(&b.win_app)).await);
    let settings = board.settings.expect("settings");
    assert_eq!(settings.home_daemon_id, b.p.mac.app.db.daemon_id().unwrap());
    assert_eq!(settings.project_id, b.win_app);
    assert!(!board.can_rule);
    assert_eq!(
        board.cards[0].assignee.as_deref(),
        Some(b.tester_id.as_str())
    );

    b.tester
        .call("item_comment", json!({"id": b.item, "body": "relayed"}))
        .await;
    // A snapshot fetch started before the fixture's (by a relay that came
    // before the mirror existed) can still land after the watch and push a
    // settings change (H-042). The client's seq still has no gap.
    let mut seen = board.seq;
    let push = loop {
        let push = next_push(&mut b.p.win_client).await.expect("a push");
        if push.item_id == b.item {
            break push;
        }
        assert_eq!(push.seq, seen + 1, "no gap before the item's push");
        seen = push.seq;
    };
    assert_eq!(push.project_id, b.win_app);
    assert_eq!(push.seq, seen + 1, "continues the client's seq");
    assert_eq!(
        push.card.expect("card").assignee.as_deref(),
        Some(b.tester_id.as_str())
    );

    // The item drawer: details come from the home, in the PC's ids; moves
    // stay there (ARCH-R28 c).
    let body = call(
        &mut b.p.win_client,
        Request::ItemGet(c::ItemGet { id: b.item.clone() }),
    )
    .await;
    let Response::Item(detail) = response(body) else {
        panic!("expected the item");
    };
    let item = detail.item.expect("item");
    assert_eq!(item.assignee.as_deref(), Some(b.tester_id.as_str()));
    assert!(
        detail
            .comments
            .iter()
            .any(|c| c.author == format!("bot:{}", b.tester_id) || c.author == b.tester_id),
        "{:?}",
        detail.comments
    );
    let body = call(
        &mut b.p.win_client,
        Request::ItemMoveCheck(c::ItemMoveCheck { id: b.item.clone() }),
    )
    .await;
    let Response::MoveCheck(check) = response(body) else {
        panic!("expected a move check");
    };
    assert!(check
        .columns
        .iter()
        .all(|col| col.unmet.iter().any(|u| u.code == "board.elsewhere"
            && u.text == format!("The board lives on mac. Move {} from there.", b.item))));
    let body = call(
        &mut b.p.win_client,
        move_to(&b.item, "doing", item.version, None),
    )
    .await;
    let Body::Error(e) = body else {
        panic!("expected a refusal");
    };
    assert_eq!(e.code, "no_board");
    assert_eq!(
        e.message,
        format!("The board lives on mac. Move {} from there.", b.item)
    );
    let body = call(
        &mut b.p.win_client,
        Request::ItemComment(c::ItemAddComment {
            id: b.item.clone(),
            body: "hi".into(),
            reply_to: None,
        }),
    )
    .await;
    let Body::Error(e) = body else {
        panic!("expected a refusal");
    };
    assert_eq!(
        e.message,
        format!(
            "The board for {} lives on mac. Comment on it from there.",
            b.item
        )
    );
    let releases =
        b.p.win_client
            .request(json!({"type": "list_releases", "project_id": b.win_app}))
            .await;
    assert_eq!(releases["type"], "error", "{releases}");
    assert!(
        releases["message"]
            .as_str()
            .is_some_and(|m| m.contains("rule on them there")),
        "{releases}"
    );
}

/// With the home unreachable the PC serves the board as last seen, and
/// refuses changes naming the home.
#[tokio::test]
async fn the_board_is_read_only_while_its_home_is_down() {
    let mut b = board().await;
    // The Mac forgets the PC, so nothing redials: the PC sees it go away.
    let mac_peer_id = b.p.mac_peer_id.clone();
    b.p.mac.app.db.revoke_peer(&mac_peer_id).unwrap();
    b.p.mac.app.peers.disconnect(&mac_peer_id);
    let win = &b.p.win;
    let win_peer_id = b.p.win_peer_id.clone();
    wait_until("the PC loses the Mac", || {
        !win.app.peers.is_online(&win_peer_id)
    })
    .await;

    let got = b.tester.call("board_get", json!({})).await;
    assert_eq!(got["cards"][0]["id"], json!(b.item), "{got}");
    assert!(
        got["offline"]
            .as_str()
            .is_some_and(|n| n.contains("read-only")),
        "{got}"
    );
    let found = b
        .tester
        .call("item_query", json!({"text": "peer", "assignee": "tester"}))
        .await;
    assert_eq!(found["cards"].as_array().map(Vec::len), Some(1), "{found}");

    let refused = b
        .tester
        .call_raw("item_comment", json!({"id": b.item, "body": "late"}))
        .await;
    assert_eq!(refused["isError"], true, "{refused}");
    let text = refused["content"][0]["text"].as_str().unwrap_or_default();
    assert_eq!(
        text,
        "The board lives on mac, which is unreachable. Try again when it's back."
    );
    let body = call(
        &mut b.p.win_client,
        Request::ItemGet(c::ItemGet { id: b.item.clone() }),
    )
    .await;
    assert_eq!(error_code(body), "unavailable");
    // And the PC's own clients still see it.
    let board = snapshot(call(&mut b.p.win_client, watch(&b.win_app)).await);
    assert_eq!(board.cards.len(), 1);
}

/// ARCH-R23 F1: the PC's tester takes part in a release homed on the Mac.
/// Its test result, `install_release` and `deploy_confirm` go to the home.
#[tokio::test]
async fn a_remote_tester_tests_installs_and_confirms_a_release() {
    let mut b = board().await;
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
    ops.call(
        "release_deploy",
        json!({"release_id": id, "machine": "win"}),
    )
    .await;

    let builds = b
        .tester
        .call("install_release", json!({"release_id": id}))
        .await;
    assert_eq!(builds["machine"], "win", "{builds}");
    assert_eq!(builds["builds"][0]["sha256"], "a".repeat(64));
    // Never the home's local path (ARCH-R28 d).
    assert_eq!(
        builds["builds"][0]["url"],
        "https://dl.example/0.16.0.tar.gz"
    );
    assert!(builds["builds"][0].get("artifact").is_none(), "{builds}");
    let done = b
        .tester
        .call(
            "deploy_confirm",
            json!({"release_id": id, "machine": "win", "result": "ok", "smoke": "pass"}),
        )
        .await;
    assert_eq!(done["release"]["status"], "deployed", "{done}");
    let deployment = &done["release"]["deployments"][0];
    assert_eq!(
        deployment["executor"],
        json!(b.tester_id),
        "in the PC's ids: {done}"
    );
}
