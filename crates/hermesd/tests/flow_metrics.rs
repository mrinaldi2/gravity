//! `metrics_get` (B11): the Flow widget's numbers from the board's history,
//! without the items the backlog import brought in; and on a linked
//! computer, the home's numbers or where to look.

mod common;

use common::board::{new_item, walk};
use common::peer_board::board;
use common::peers::wait_until;
use common::tasks::project_with_bots;
use common::*;
use hermesd::board::model::Priority;
use serde_json::{json, Value};

const BACKLOG: &str = include_str!("fixtures/board_import_backlog.md");

async fn metrics(c: &mut WsClient, project: &str, range: &str) -> Value {
    let reply = c
        .request(json!({"type": "metrics_get", "project_id": project, "range": range}))
        .await;
    assert_eq!(reply["type"], "metrics", "{reply}");
    reply
}

#[tokio::test]
async fn flow_counts_this_ranges_work_without_the_import() {
    let (pair, _bots) = project_with_bots(&["Team Lead"]).await;
    let db = &pair.d.app.db;
    let project = db.get_bot(&pair.ids[0]).unwrap().unwrap().project_id;
    db.ensure_board(&project, &db.daemon_id().unwrap(), None)
        .unwrap();
    let mut owner = WsClient::connect(&pair.d).await;
    let imported = owner
        .request(json!({"type": "board_import", "markdown": BACKLOG, "project": "p"}))
        .await;
    assert_eq!(imported["type"], "board_imported", "{imported}");
    walk(db, "H-002", &["doing", "review", "verify", "done"], None);

    let (shipped, _) = new_item(db, &project, "Shipped", Priority::P2);
    walk(
        db,
        &shipped,
        &["ready", "doing", "review", "verify", "done"],
        None,
    );
    let (returned, _) = new_item(db, &project, "Returned", Priority::P2);
    walk(db, &returned, &["ready", "doing", "review", "doing"], None);

    let m = metrics(&mut owner, &project, "week").await["metrics"].clone();
    assert_eq!(m["days"], 7);
    assert_eq!(m["throughput"], 1, "the import's H-002 doesn't count: {m}");
    assert_eq!(m["weekly"].as_array().unwrap().len(), 8);
    assert_eq!(m["cycle"]["count"], 1);
    assert_eq!(m["reworked"], 1);
    assert_eq!(m["past_doing"], 2);
    assert_eq!(m["daily"].as_array().unwrap().len(), 7);
    assert_eq!(m["daily"][6]["wip"], 1, "Returned is back in Doing");
    assert_eq!(m["aging"][0]["id"], json!(returned));
    assert!(m["by_column"]
        .as_array()
        .unwrap()
        .iter()
        .any(|c| c["key"] == "review"));

    let four = metrics(&mut owner, &project, "4w").await;
    assert_eq!(four["metrics"]["daily"].as_array().unwrap().len(), 28);
    let bad = owner
        .request(json!({"type": "metrics_get", "project_id": project, "range": "year"}))
        .await;
    assert_eq!(bad["type"], "error", "{bad}");
}

#[tokio::test]
async fn a_linked_computer_shows_the_homes_flow_or_where_to_look() {
    let mut b = board().await;
    let reply = metrics(&mut b.p.win_client, &b.win_app, "week").await;
    assert!(reply["metrics"]["throughput"].is_number(), "{reply}");
    assert_eq!(reply["note"], Value::Null);

    let mac_peer_id = b.p.mac_peer_id.clone();
    b.p.mac.app.db.revoke_peer(&mac_peer_id).unwrap();
    b.p.mac.app.peers.disconnect(&mac_peer_id);
    let (win, win_peer_id) = (&b.p.win, b.p.win_peer_id.clone());
    wait_until("the PC loses the Mac", || {
        !win.app.peers.is_online(&win_peer_id)
    })
    .await;
    let reply = metrics(&mut b.p.win_client, &b.win_app, "week").await;
    assert_eq!(reply["metrics"], Value::Null);
    assert!(
        reply["note"]
            .as_str()
            .is_some_and(|n| n.contains("can't be reached")),
        "{reply}"
    );
}
