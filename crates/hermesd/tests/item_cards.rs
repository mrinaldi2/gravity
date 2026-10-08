//! Card ids as links (H-203, UX-035 §9): `item_cards_get` answers each id
//! with its card and where it is kept, or why it can't; projects carry their
//! item prefix so a client knows `H-` from `UTF-`.

mod common;

use common::board::new_item;
use common::peer_board::board;
use common::peers::{project, wait_until};
use common::WsClient;
use hermesd::board::model::Priority;
use serde_json::{json, Value};

fn entry<'a>(reply: &'a Value, id: &str) -> &'a Value {
    reply["cards"]
        .as_array()
        .expect("cards")
        .iter()
        .find(|c| c["id"] == id)
        .unwrap_or_else(|| panic!("no entry for {id}: {reply}"))
}

#[tokio::test]
async fn cards_on_this_computer_come_back_with_where_they_are_kept() {
    let d = common::spawn_daemon().await;
    let mut owner = WsClient::connect(&d).await;
    assert!(hermesd::app::CAPABILITIES.contains(&"item_cards"));
    let project_id = project(&mut owner, "The Hermes").await;
    let db = &d.app.db;
    db.ensure_board(&project_id, &db.daemon_id().unwrap(), Some("H"))
        .unwrap();
    let (card, _) = new_item(db, &project_id, "Card links", Priority::P1);

    let listed = owner.request(json!({"type": "list_projects"})).await;
    let mine = listed["projects"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["id"] == project_id.as_str())
        .cloned()
        .unwrap();
    assert_eq!(mine["item_prefix"], "H", "{mine}");

    let reply = owner
        .request(json!({"type": "item_cards_get", "ids": [card, "H-999", "UTF-8"]}))
        .await;
    assert_eq!(reply["type"], "item_cards", "{reply}");
    let found = entry(&reply, &card);
    assert_eq!(found["project_id"], project_id.as_str());
    assert_eq!(found["project_name"], "The Hermes");
    assert_eq!(found["card"]["title"], "Card links");
    assert_eq!(found["column_name"], "Inbox", "{found}");
    assert!(found["computer"].is_string());
    assert!(found.get("missing").is_none_or(Value::is_null), "{found}");
    // A known prefix but no such card: missing, on that project.
    let gone = entry(&reply, "H-999");
    assert_eq!(gone["missing"], true);
    assert_eq!(gone["project_id"], project_id.as_str());
    // No project uses the prefix: missing, with no project.
    let other = entry(&reply, "UTF-8");
    assert_eq!(other["missing"], true);
    assert!(other["project_id"].is_null(), "{other}");
}

/// A card mirrored from the board's home comes from the mirror, and with the
/// home offline it is unreachable, with the last copy.
#[tokio::test]
async fn a_card_kept_on_another_computer_is_read_from_its_mirror() {
    let b = board().await;
    let mut win = WsClient::connect(&b.p.win).await;
    let reply = win
        .request(json!({"type": "item_cards_get", "ids": [b.item]}))
        .await;
    let card = entry(&reply, &b.item);
    assert_eq!(card["project_id"], b.win_app.as_str());
    assert_eq!(card["card"]["title"], "Peer board");
    assert!(card.get("unreachable").is_none_or(Value::is_null), "{card}");
    let home = card["computer"].as_str().unwrap().to_string();

    let mac_peer_id = b.p.mac_peer_id.clone();
    b.p.mac.app.db.revoke_peer(&mac_peer_id).unwrap();
    b.p.mac.app.peers.disconnect(&mac_peer_id);
    let (winapp, win_peer) = (&b.p.win.app, b.p.win_peer_id.clone());
    wait_until("the PC loses the Mac", || {
        !winapp.peers.is_online(&win_peer)
    })
    .await;

    let reply = win
        .request(json!({"type": "item_cards_get", "ids": [b.item, "H-999"]}))
        .await;
    let card = entry(&reply, &b.item);
    assert_eq!(card["unreachable"]["computer"], home.as_str(), "{card}");
    assert_eq!(card["card"]["title"], "Peer board", "the last copy");
    let gone = entry(&reply, "H-999");
    assert_eq!(gone["unreachable"]["computer"], home.as_str(), "{gone}");
}
