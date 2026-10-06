//! Owner rulings that grant extras to the linked Windev (H-163), and reading
//! what the decision says about them.

#![allow(dead_code)]

use bus::PermissionExtra;
use serde_json::{json, Value};

use super::peers::{wait_until, Team};
use super::{TestDaemon, WsClient};

pub async fn desktop(d: &TestDaemon) -> WsClient {
    WsClient::connect_with(d, d.app.secrets.client_token(), &["decision_grants"]).await
}

/// The lead asks for `extras` for the linked Windev; the owner grants them,
/// after `before_publish`. Returns the decision's id once it is published.
pub async fn grant(t: &mut Team, extras: &[&str], before_publish: impl FnOnce(&Team)) -> String {
    let grants: Vec<Value> = extras
        .iter()
        .map(|e| json!({"bot": t.linked_windev, "extra": e}))
        .collect();
    let raised = t
        .lead
        .call(
            "raise_decision",
            json!({"title": "Let Windev ship?", "body": "b",
                   "options": [{"key": "yes", "label": "Yes", "grants": grants}]}),
        )
        .await;
    let id = raised["decision"]["id"].as_str().expect("id").to_string();
    let mut owner = desktop(&t.mac).await;
    let got = owner
        .request(json!({"type": "get_decision", "decision_id": id}))
        .await;
    let sha = got["decision"]["options"][0]["grants_sha"].clone();
    before_publish(t);
    owner
        .request(
            json!({"type": "publish_decisions", "items": [{"decision_id": id,
                        "ruling_option": "yes", "ruling_text": "Yes.", "grants_sha": sha}]}),
        )
        .await;
    id
}

/// The Mac drops its link to the PC, as a restart there does; it redials no
/// sooner than two seconds on.
pub fn drop_link(t: &Team) {
    for p in t.mac.app.db.list_peers().unwrap() {
        t.mac.app.peers.disconnect(&p.id);
    }
}

pub fn comments(d: &TestDaemon, id: &str) -> Vec<String> {
    d.app
        .db
        .list_decision_comments(id)
        .unwrap()
        .into_iter()
        .map(|c| c.body)
        .collect()
}

pub async fn said(d: &TestDaemon, id: &str, needle: &str) {
    wait_until(&format!("the decision says '{needle}'"), || {
        comments(d, id).iter().any(|c| c.contains(needle))
    })
    .await;
}

/// What the PC holds for Windev.
pub fn extras(t: &Team) -> Vec<PermissionExtra> {
    t.win.app.db.bot_permission_extras(&t.windev_id).unwrap()
}

/// Waits until nothing is left waiting on the Mac.
pub async fn settled(t: &Team) {
    let mac = &t.mac;
    wait_until("no grant is left waiting", || {
        mac.app.db.pending_peer_grants(None).unwrap().is_empty()
    })
    .await;
}
