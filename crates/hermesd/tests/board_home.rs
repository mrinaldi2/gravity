//! One home per board (H-020 §1.3, H-037): an unlinked project's board is
//! enabled on its first read; a linked project's only when the owner starts
//! it on the computer that should be its home. Once it exists, only the
//! daemon named in `home_daemon_id` serves it, and two projects that each
//! have a board can't be linked.

mod common;

use bus::contract::board::{self as c, board_request::Request};
use bus::contract::wire::envelope::Body;
use common::board::*;
use common::*;
use hermesd::board::defaults::COLUMNS;
use serde_json::{json, Value};

fn board_get(project_id: &str) -> Request {
    Request::BoardGet(c::BoardGet {
        project_id: project_id.to_string(),
    })
}

fn board_enable(project_id: &str) -> Request {
    Request::BoardEnable(c::BoardEnable {
        project_id: project_id.to_string(),
    })
}

/// A project on a fresh daemon, linked through a peer that this daemon
/// dials (`url` set) or that dials it (`url` none), or not linked at all.
async fn project(link: Option<Option<&str>>) -> (TestDaemon, WsClient, String) {
    let d = spawn_daemon().await;
    let mut owner = WsClient::connect(&d).await;
    let project_id = common::peers::project(&mut owner, "Linked").await;
    if let Some(url) = link {
        let peer = d.app.db.create_peer("other", url).unwrap();
        d.app
            .db
            .create_project_link(&project_id, &peer.id, "remote-project", "Linked")
            .unwrap();
    }
    (d, owner, project_id)
}

#[tokio::test]
async fn an_unlinked_project_enables_its_board_here() {
    let (d, mut owner, project_id) = project(None).await;
    let board = snapshot(call(&mut owner, board_get(&project_id)).await);
    let home = board.settings.expect("settings").home_daemon_id;
    assert_eq!(home, d.app.db.daemon_id().unwrap());
}

/// Both sides dial each other on the owner's Mac (both peer rows have a
/// url), and listening is no sign of home either: a linked project gets a
/// board only when the owner starts it.
#[tokio::test]
async fn a_linked_project_gets_a_board_only_when_the_owner_starts_it() {
    for url in [Some("wss://pc.example:7316"), None] {
        let (d, mut owner, project_id) = project(Some(url)).await;
        let body = call(&mut owner, board_get(&project_id)).await;
        assert_eq!(error_code(body), "no_board");
        assert!(d.app.db.board_settings(&project_id).unwrap().is_none());

        let board = snapshot(call(&mut owner, board_enable(&project_id)).await);
        let home = board.settings.expect("settings").home_daemon_id;
        assert_eq!(home, d.app.db.daemon_id().unwrap());
        assert_eq!(board.columns.len(), COLUMNS.len());
        // Served from here from now on, and starting it again changes nothing.
        snapshot(call(&mut owner, board_get(&project_id)).await);
        let again = snapshot(call(&mut owner, board_enable(&project_id)).await);
        assert_eq!(again.settings.expect("settings").key, "L");
    }
}

#[tokio::test]
async fn only_the_owner_starts_a_board() {
    let (d, mut owner, project_id) = project(Some(Some("wss://pc.example:7316"))).await;
    let created = owner
        .request(json!({"type": "create_device", "name": "tablet",
                        "capabilities": ["read", "control"]}))
        .await;
    let mut device = connect_with(&d, token_str(&created)).await;
    let body = call(&mut device, board_enable(&project_id)).await;
    assert_eq!(error_code(body), "forbidden");
    assert!(d.app.db.board_settings(&project_id).unwrap().is_none());
}

/// A board recorded with another daemon as its home is not served here.
#[tokio::test]
async fn a_board_homed_elsewhere_is_refused_naming_its_home() {
    let (d, mut owner, project_id) = project(None).await;
    let imac = d.app.db.create_peer("imac", None).unwrap();
    d.app.db.bind_peer_daemon(&imac.id, "d-imac").unwrap();
    d.app.db.ensure_board(&project_id, "d-imac", None).unwrap();
    for request in [board_get(&project_id), board_enable(&project_id)] {
        let Body::Error(e) = call(&mut owner, request).await else {
            panic!("expected a refusal");
        };
        assert_eq!(e.code, "no_board");
        assert!(e.message.contains("imac"), "{}", e.message);
    }
}

/// A board enabled before a link stays home on its side.
#[tokio::test]
async fn an_existing_board_keeps_its_home_after_a_link() {
    let d = spawn_daemon().await;
    let mut owner = WsClient::connect(&d).await;
    let project_id = common::peers::project(&mut owner, "Linked").await;
    snapshot(call(&mut owner, board_get(&project_id)).await);
    let peer = d
        .app
        .db
        .create_peer("imac", Some("wss://imac.example:7316"))
        .unwrap();
    d.app
        .db
        .create_project_link(&project_id, &peer.id, "remote-project", "Linked")
        .unwrap();
    let board = snapshot(call(&mut owner, board_get(&project_id)).await);
    let home = board.settings.expect("settings").home_daemon_id;
    assert_eq!(home, d.app.db.daemon_id().unwrap());
}

fn refused_for_two_homes(reply: &Value) {
    assert_eq!(reply["type"], "error", "{reply}");
    assert_eq!(reply["code"], "conflict");
    let message = reply["message"].as_str().expect("message");
    assert!(message.contains("one board home"), "{message}");
}

/// Two projects that each have a board can't be linked, from either side.
#[tokio::test]
async fn linking_two_boards_is_refused() {
    let mut p = common::peers::paired().await;
    let mac_app = common::peers::project(&mut p.mac_client, "app").await;
    snapshot(call(&mut p.mac_client, board_get(&mac_app)).await);
    let win_app = common::peers::project(&mut p.win_client, "pc-app").await;
    snapshot(call(&mut p.win_client, board_get(&win_app)).await);

    let from_mac = p
        .mac_client
        .request(json!({"type": "link_project", "project_id": mac_app,
                        "peer_id": p.mac_peer_id, "remote_project_id": win_app}))
        .await;
    refused_for_two_homes(&from_mac);
    let from_pc = p
        .win_client
        .request(json!({"type": "link_project", "project_id": win_app,
                        "peer_id": p.win_peer_id, "remote_project_id": mac_app}))
        .await;
    refused_for_two_homes(&from_pc);
    assert!(p.mac.app.db.project_links(&mac_app).unwrap().is_empty());
    assert!(p.win.app.db.project_links(&win_app).unwrap().is_empty());
}

/// When only one side has a board the link goes through, from either side,
/// and that side is the home: the other one starts none on its own.
#[tokio::test]
async fn the_side_with_the_board_is_the_home() {
    let mut p = common::peers::paired().await;
    let mac_app = common::peers::project(&mut p.mac_client, "app").await;
    snapshot(call(&mut p.mac_client, board_get(&mac_app)).await);
    let win_app = common::peers::project(&mut p.win_client, "pc-app").await;
    let linked = p
        .win_client
        .request(json!({"type": "link_project", "project_id": win_app,
                        "peer_id": p.win_peer_id, "remote_project_id": mac_app}))
        .await;
    assert_eq!(linked["type"], "project", "{linked}");
    snapshot(call(&mut p.mac_client, board_get(&mac_app)).await);
    let body = call(&mut p.win_client, board_get(&win_app)).await;
    assert_eq!(error_code(body), "no_board");

    // The dialing side's board links too (the Mac dials the PC here).
    let mac_two = common::peers::project(&mut p.mac_client, "two").await;
    snapshot(call(&mut p.mac_client, board_get(&mac_two)).await);
    let linked = p
        .mac_client
        .request(json!({"type": "link_project", "project_id": mac_two, "peer_id": p.mac_peer_id}))
        .await;
    assert_eq!(linked["type"], "project", "{linked}");
}
