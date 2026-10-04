//! One home per board (H-020 §1.3, ARCH-R8 M1): until boards forward (B9), a
//! linked project's board is enabled only on the side that accepted the
//! link, so imac and win-pc never grow a second board for the Mac's project.

mod common;

use bus::contract::board::{self as c, board_request::Request};
use common::board::*;
use common::*;
use serde_json::{json, Value};

fn board_get(project_id: &str) -> Request {
    Request::BoardGet(c::BoardGet {
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

#[tokio::test]
async fn the_side_that_accepted_the_link_is_the_home() {
    let (d, mut owner, project_id) = project(Some(None)).await;
    let board = snapshot(call(&mut owner, board_get(&project_id)).await);
    let home = board.settings.expect("settings").home_daemon_id;
    assert_eq!(home, d.app.db.daemon_id().unwrap());
}

#[tokio::test]
async fn the_dialing_side_gets_no_second_board() {
    let (d, mut owner, project_id) = project(Some(Some("wss://mac.example:7316"))).await;
    let body = call(&mut owner, board_get(&project_id)).await;
    assert_eq!(error_code(body), "no_board");
    assert!(d.app.db.board_settings(&project_id).unwrap().is_none());
}

/// A board enabled before a link stays home on the side the other one
/// dials. Linking it as the dialing side is refused (the two tests below).
#[tokio::test]
async fn an_existing_board_keeps_its_home_after_a_link() {
    let d = spawn_daemon().await;
    let mut owner = WsClient::connect(&d).await;
    let project_id = common::peers::project(&mut owner, "Linked").await;
    snapshot(call(&mut owner, board_get(&project_id)).await);
    let peer = d.app.db.create_peer("imac", None).unwrap();
    d.app
        .db
        .create_project_link(&project_id, &peer.id, "remote-project", "Linked")
        .unwrap();
    let board = snapshot(call(&mut owner, board_get(&project_id)).await);
    let home = board.settings.expect("settings").home_daemon_id;
    assert_eq!(home, d.app.db.daemon_id().unwrap());
}

fn refused_for_its_board(reply: &Value) {
    assert_eq!(reply["type"], "error", "{reply}");
    assert_eq!(reply["code"], "conflict");
    let message = reply["message"].as_str().expect("message");
    assert!(message.contains("board lives here"), "{message}");
}

/// In `paired()` the Mac dials the PC. A Mac project with a board can't be
/// linked from the Mac: the PC it dials would become a second home.
#[tokio::test]
async fn the_dialing_side_cannot_link_a_project_whose_board_is_here() {
    let mut p = common::peers::paired().await;
    let mac_app = common::peers::project(&mut p.mac_client, "app").await;
    snapshot(call(&mut p.mac_client, board_get(&mac_app)).await);
    let reply = p
        .mac_client
        .request(json!({"type": "link_project", "project_id": mac_app, "peer_id": p.mac_peer_id}))
        .await;
    refused_for_its_board(&reply);
    assert!(p.mac.app.db.project_links(&mac_app).unwrap().is_empty());
}

/// Nor from the other side: the PC asking to link its project with the
/// Mac's is refused by the Mac, which dials it (the re-pairing case).
#[tokio::test]
async fn a_link_into_a_board_on_the_dialing_side_is_refused_there() {
    let mut p = common::peers::paired().await;
    let mac_app = common::peers::project(&mut p.mac_client, "app").await;
    snapshot(call(&mut p.mac_client, board_get(&mac_app)).await);
    let win_app = common::peers::project(&mut p.win_client, "pc-app").await;
    let reply = p
        .win_client
        .request(json!({"type": "link_project", "project_id": win_app,
                        "peer_id": p.win_peer_id, "remote_project_id": mac_app}))
        .await;
    refused_for_its_board(&reply);
    assert!(p.win.app.db.project_links(&win_app).unwrap().is_empty());
    assert!(p.mac.app.db.project_links(&mac_app).unwrap().is_empty());

    // The listener's own board links fine: it stays home here.
    snapshot(call(&mut p.win_client, board_get(&win_app)).await);
    let linked = p
        .win_client
        .request(json!({"type": "link_project", "project_id": win_app, "peer_id": p.win_peer_id}))
        .await;
    assert_eq!(linked["type"], "project", "{linked}");
}
