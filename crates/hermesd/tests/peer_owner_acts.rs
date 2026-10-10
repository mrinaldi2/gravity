//! Owner acts over a peer link (H-301; owner ruling 6df14f7a): a peer token
//! is a file bots can read, so a computer's word that the owner approved a
//! Run card or ruled a grant there isn't proof. The ruling's computer says
//! where to set the extras instead of sending them; a grant or a run frame
//! that arrives anyway is refused, and nothing is granted or run.

mod common;

use common::grants::{extras, grant, said};
use common::peers::team;
use serde_json::json;

/// AC0: a ruling's grants for a linked bot aren't sent; the decision says to
/// set them on that computer or a phone. A forged `grant_extras` straight
/// over the link is refused there, and the bot gets nothing.
#[tokio::test]
async fn a_grant_over_a_peer_link_is_refused_and_nothing_is_granted() {
    let mut t = team().await;
    let id = grant(&mut t, &["install"], |_| {}).await;
    said(&t.mac, &id, "or from your phone").await;
    assert!(extras(&t).is_empty(), "nothing sent");

    // The same frame a bot holding the link's token could send.
    let mac_side = t.mac.app.db.list_peers().unwrap()[0].id.clone();
    let forged = json!({"type": "grant_extras", "bot_id": t.windev_id,
                        "extras": ["install", "quiesce"], "decision_id": id,
                        "decided_at": chrono::Utc::now().to_rfc3339()});
    let refused = t.mac.app.peers.request(&mac_side, forged).await;
    assert!(format!("{refused:?}").contains("Approve on"), "{refused:?}");
    assert!(extras(&t).is_empty(), "nothing granted");
}

/// AC0: a forged owner run over the link is refused before anything is
/// looked up or run.
#[tokio::test]
async fn an_owner_run_over_a_peer_link_is_refused() {
    let t = team().await;
    let mac_side = t.mac.app.db.list_peers().unwrap()[0].id.clone();
    let forged = json!({"type": "owner_action_run", "id": "any", "sha256": "0".repeat(64),
                        "approved_by": "owner"});
    let refused = t.mac.app.peers.request(&mac_side, forged).await;
    assert!(format!("{refused:?}").contains("Approve on"), "{refused:?}");
}
