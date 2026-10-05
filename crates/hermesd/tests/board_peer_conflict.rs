//! A stale write from a linked computer comes back in that computer's ids
//! (H-113), and every write at the home that moves an item's version is
//! pushed, so the mirror never keeps a stale version.

mod common;

use common::peer_board::board;
use common::peers::wait_until;
use hermesd::actor::Actor;
use serde_json::json;

#[tokio::test]
async fn a_conflict_from_the_home_names_the_linked_computers_bots() {
    let mut b = board().await;
    let stale = b
        .tester
        .call_raw(
            "item_move",
            json!({"id": b.item, "to": "doing", "expected_version": 1}),
        )
        .await;
    assert_eq!(stale["isError"], true, "{stale}");
    let text = stale["content"][0]["text"].as_str().expect("text");
    assert!(text.starts_with("conflict: the item changed"), "{text}");
    assert!(text.contains(&b.tester_id), "the PC's own id: {text}");
    assert!(
        !text.contains(&b.stand_in),
        "not the Mac's stand-in: {text}"
    );
}

#[tokio::test]
async fn the_mirror_follows_every_version_the_home_writes() {
    let b = board().await;
    let (mac, win) = (&b.p.mac, &b.p.win);
    let item = mac.app.db.get_item(&b.item).unwrap().expect("item");
    // A write outside the tools, as a retiring bot's handback does.
    let bot = mac.app.db.get_bot(&b.stand_in).unwrap().expect("stand-in");
    hermesd::board::handback::items_to_lead(&mac.app, &bot, &Actor::User).expect("handback");
    let now = mac.app.db.get_item(&b.item).unwrap().expect("item").version;
    assert!(now > item.version);
    let win_app = b.win_app.clone();
    wait_until("the PC mirrors the new version", || {
        win.app.board_mirror.get(&win_app).is_some_and(|m| {
            m.snapshot
                .cards
                .iter()
                .any(|c| c.id == b.item && c.version == now)
        })
    })
    .await;
}
