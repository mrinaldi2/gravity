//! The dashboard on a computer linked to the board's home (H-112): Needs you
//! adds the home's rows, marked with where to act on them, beside this
//! computer's own decisions; with the home away it says where to look.

mod common;

use common::peer_board::board;
use common::peers::{bot_named, wait_until};
use common::McpClient;
use serde_json::{json, Value};

fn titles(d: &Value) -> Vec<(String, Value)> {
    d["needs_you"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|row| row["kind"] == "decision")
        .map(|row| {
            (
                row["title"].as_str().unwrap().to_string(),
                row["elsewhere"].clone(),
            )
        })
        .collect()
}

#[tokio::test]
async fn needs_you_off_home_shows_the_homes_rows_or_where_to_look() {
    let mut b = board().await;
    let lead = bot_named(&b.p.mac, &b.mac_app, "lead").expect("lead");
    let token = b.p.mac.app.secrets.bot_token(&lead.id).expect("token");
    let mut mac_lead = McpClient::new(&b.p.mac, &token);
    mac_lead
        .call(
            "raise_decision",
            json!({"title": "Freeze on Friday?", "body": "Yes or no."}),
        )
        .await;
    b.tester
        .call(
            "raise_decision",
            json!({"title": "Which PC test plan?", "body": "Pick one."}),
        )
        .await;

    let reply =
        b.p.win_client
            .request(json!({"type": "dashboard_get", "project_id": b.win_app}))
            .await;
    assert_eq!(reply["type"], "dashboard", "{reply}");
    let d = &reply["dashboard"];
    let rows = titles(d);
    let home = d["home"].clone();
    assert!(home.is_string(), "{d}");
    assert!(
        rows.contains(&("Freeze on Friday?".into(), home)),
        "{rows:?}"
    );
    assert!(
        rows.contains(&("Which PC test plan?".into(), Value::Null)),
        "{rows:?}"
    );
    assert_eq!(d["needs_you_note"], Value::Null);

    // The Mac goes away: the PC's own rows stay, and it says where to look.
    let mac_peer_id = b.p.mac_peer_id.clone();
    b.p.mac.app.db.revoke_peer(&mac_peer_id).unwrap();
    b.p.mac.app.peers.disconnect(&mac_peer_id);
    let (win, win_peer_id) = (&b.p.win, b.p.win_peer_id.clone());
    wait_until("the PC loses the Mac", || {
        !win.app.peers.is_online(&win_peer_id)
    })
    .await;
    let reply =
        b.p.win_client
            .request(json!({"type": "dashboard_get", "project_id": b.win_app}))
            .await;
    let d = &reply["dashboard"];
    assert_eq!(
        titles(d),
        vec![("Which PC test plan?".to_string(), Value::Null)]
    );
    assert!(
        d["needs_you_note"]
            .as_str()
            .is_some_and(|n| n.contains("can't be reached")),
        "{d}"
    );
}
