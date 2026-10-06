//! The projects home across linked computers (H-128 D3): each computer
//! reports only the rows it owns, a peer's part arrives in the background
//! after its `project_attention_changed`, a peer answers only for projects
//! linked with the caller, and a peer away leaves its last good part, marked.

mod common;

use bus::contract::home::{source::State, ProjectAttention, ProjectRow, ProjectsOverview};
use common::peer_board::board;
use common::peers::{project, wait_until};
use hermesd::app::AppState;
use serde_json::json;

fn overview(app: &AppState) -> ProjectsOverview {
    hermesd::overview::overview(app, &[]).expect("overview")
}

fn row(o: &ProjectsOverview, project_id: &str) -> ProjectRow {
    o.rows
        .iter()
        .find(|r| r.project_id == project_id)
        .cloned()
        .unwrap_or_else(|| panic!("no row for {project_id}"))
}

fn decisions(r: &ProjectRow) -> u32 {
    r.attention
        .as_ref()
        .and_then(|a| a.by_kind.get("decision").copied())
        .unwrap_or(0)
}

#[tokio::test]
async fn the_mac_counts_the_pcs_rows_once_and_keeps_them_while_it_is_away() {
    let mut b = board().await;
    let (mac, win) = (&b.p.mac.app, &b.p.win.app);
    let win_daemon = win.db.daemon_id().unwrap();
    b.tester
        .call(
            "raise_decision",
            json!({"title": "Which PC test plan?", "body": "Pick one."}),
        )
        .await;
    // The PC tells the Mac its part changed; the Mac asks and counts it.
    let mac_app = b.mac_app.clone();
    wait_until("the Mac counts the PC's decision", || {
        decisions(&row(&overview(mac), &mac_app)) == 1
    })
    .await;
    let mac_row = row(&overview(mac), &b.mac_app);
    assert!(!mac_row.partial, "{mac_row:?}");
    assert!(mac_row
        .members
        .iter()
        .any(|m| m.daemon_id == win_daemon && m.project_id == b.win_app));
    let o = overview(mac);
    let source = o
        .sources
        .iter()
        .find(|s| s.daemon_id == win_daemon)
        .expect("source");
    assert_eq!(source.state(), State::Ok);
    assert!(source.as_of.is_some());

    // Its row says to act on the PC, as the PC numbers it.
    let rows = hermesd::overview::attention_rows(mac, &b.mac_app).expect("rows");
    let decision = rows
        .rows
        .iter()
        .find(|r| r.title == "Which PC test plan?")
        .expect("the PC's decision");
    assert_eq!(decision.daemon_id, win_daemon);
    assert_eq!(decision.project_id, b.win_app);

    // Each side counts the same rows: none twice (D3 e).
    let win_app = b.win_app.clone();
    let mac_count = || row(&overview(mac), &mac_app).attention.unwrap().count;
    wait_until("the PC counts the Mac's part", || {
        row(&overview(win), &win_app).attention.unwrap().count == mac_count()
    })
    .await;
    assert_eq!(
        row(&overview(win), &b.win_app).board_home,
        mac.db.daemon_id().unwrap()
    );

    // A stand-in says where its bot runs (R2.3).
    let bots =
        b.p.mac_client
            .request(json!({"type": "list_bots", "project_id": b.mac_app}))
            .await;
    let stand_in = bots["bots"]
        .as_array()
        .unwrap()
        .iter()
        .find(|bot| bot["id"] == b.stand_in)
        .expect("stand-in");
    assert_eq!(
        stand_in["origin"],
        json!({"daemon_id": win_daemon, "bot_id": b.tester_id})
    );

    // The PC goes away: the row keeps the last good count, marked (D3 b).
    let win_peer_id = b.p.win_peer_id.clone();
    win.db.revoke_peer(&win_peer_id).unwrap();
    win.peers.disconnect(&win_peer_id);
    let mac_peer_id = b.p.mac_peer_id.clone();
    wait_until("the Mac marks the PC away", || {
        !mac.peers.is_online(&mac_peer_id) && row(&overview(mac), &mac_app).partial
    })
    .await;
    let away = row(&overview(mac), &b.mac_app);
    assert_eq!(decisions(&away), 1, "last good is kept: {away:?}");
    assert_eq!(away.stale_sources, vec![win_daemon.clone()]);
    let o = overview(mac);
    let source = o
        .sources
        .iter()
        .find(|s| s.daemon_id == win_daemon)
        .expect("source");
    assert_eq!(source.state(), State::Offline);
    assert!(source.as_of.is_some(), "the data's age");
}

#[tokio::test]
async fn a_peer_answers_only_for_projects_linked_with_the_caller() {
    let mut b = board().await;
    let unlinked = project(&mut b.p.win_client, "private").await;
    let answer =
        b.p.mac
            .app
            .peers
            .request(
                &b.p.mac_peer_id,
                json!({"type": "project_attention", "project_ids": [b.win_app, unlinked]}),
            )
            .await
            .expect("an answer");
    let answer: ProjectAttention = serde_json::from_value(answer).expect("proto3 JSON");
    assert_eq!(answer.parts.len(), 1, "{answer:?}");
    // In the caller's ids.
    assert_eq!(answer.parts[0].project_id, b.mac_app);
    assert!(!answer.parts[0].is_home, "the PC doesn't hold the board");
}
