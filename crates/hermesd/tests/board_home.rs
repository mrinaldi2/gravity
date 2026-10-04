//! One home per board (H-020 §1.3, ARCH-R8 M1): until boards forward (B9), a
//! linked project's board is enabled only on the side that accepted the
//! link, so imac and win-pc never grow a second board for the Mac's project.

mod common;

use bus::contract::board::{self as c, board_request::Request};
use common::board::*;
use common::*;

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

#[tokio::test]
async fn an_existing_board_keeps_its_home_after_a_link() {
    let d = spawn_daemon().await;
    let mut owner = WsClient::connect(&d).await;
    let project_id = common::peers::project(&mut owner, "Linked").await;
    snapshot(call(&mut owner, board_get(&project_id)).await);
    let peer = d
        .app
        .db
        .create_peer("mac", Some("wss://mac.example:7316"))
        .unwrap();
    d.app
        .db
        .create_project_link(&project_id, &peer.id, "remote-project", "Linked")
        .unwrap();
    let board = snapshot(call(&mut owner, board_get(&project_id)).await);
    let home = board.settings.expect("settings").home_daemon_id;
    assert_eq!(home, d.app.db.daemon_id().unwrap());
}
