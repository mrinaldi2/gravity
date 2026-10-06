//! The owner's message on a card (H-128 D5): `send_user_message` with an
//! `item_id` carries the card to the bot and comments on the card; a card
//! of another project, or a closed one, is refused; without one nothing
//! changes.

mod common;

use common::board::{new_item, walk};
use common::peers::project;
use common::tasks::project_with_bots;
use common::*;
use hermesd::board::model::Priority;
use serde_json::json;

#[tokio::test]
async fn a_message_on_a_card_carries_it_and_comments_on_it() {
    let (pair, _bots) = project_with_bots(&["Desktop Dev"]).await;
    let db = &pair.d.app.db;
    let bot = pair.ids[0].clone();
    let project_id = db.get_bot(&bot).unwrap().unwrap().project_id;
    let me = db.daemon_id().unwrap();
    db.ensure_board(&project_id, &me, Some("H")).unwrap();
    let (card, _) = new_item(db, &project_id, "Projects home", Priority::P1);
    let mut owner = WsClient::connect(&pair.d).await;

    let send = |item: Option<&str>, body: &str| {
        let mut req = json!({"type": "send_user_message", "to_bot_id": bot, "body": body});
        if let Some(item) = item {
            req["item_id"] = json!(item);
        }
        req
    };
    let reply = owner.request(send(Some(&card), "Build the overview")).await;
    assert_eq!(reply["type"], "message", "{reply}");
    assert_eq!(reply["item_id"], card);
    assert_eq!(reply["commented"], true);
    assert_eq!(
        reply["message"]["body"],
        format!("[card {card}] Build the overview")
    );
    assert_eq!(reply["message"]["kind"], "chat");
    let comments = db.board_read(|t| t.item_comments(&card)).unwrap();
    assert_eq!(comments.len(), 1);
    assert_eq!(
        comments[0].body,
        "Owner asked Desktop Dev: Build the overview"
    );

    // Without a card, a chat as before.
    let plain = owner.request(send(None, "How is it going?")).await;
    assert_eq!(plain["message"]["body"], "How is it going?");
    assert!(plain.get("item_id").is_none(), "{plain}");

    // Another project's card is refused, and nothing is sent.
    let other = project(&mut owner, "other").await;
    db.ensure_board(&other, &me, Some("O")).unwrap();
    let (theirs, _) = new_item(db, &other, "Not yours", Priority::P1);
    let refused = owner.request(send(Some(&theirs), "Do this")).await;
    assert_eq!(refused["type"], "error", "{refused}");
    assert_eq!(refused["code"], "invalid_request");
    assert!(db
        .board_read(|t| t.item_comments(&theirs))
        .unwrap()
        .is_empty());

    // A closed card is refused too.
    walk(
        db,
        &card,
        &["ready", "doing", "review", "verify", "done"],
        None,
    );
    let closed = owner.request(send(Some(&card), "One more thing")).await;
    assert_eq!(closed["code"], "invalid_request", "{closed}");
    let missing = owner.request(send(Some("H-999"), "Hm")).await;
    assert_eq!(missing["code"], "not_found", "{missing}");
}
