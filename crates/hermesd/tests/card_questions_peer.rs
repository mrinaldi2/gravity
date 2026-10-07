//! Card questions across linked computers (H-211): a bot on the PC asks on
//! the Mac's card; the Mac keeps the question, so the owner's reply reaches
//! the bot on the PC, and the PC closes the question once the card it
//! mirrors shows the owner's comment. The attention row names the comment
//! that asked.

mod common;

use bus::contract::board::{self as c, board_request::Request, board_response::Response};
use common::board::{call, response};
use common::peer_board::board;
use common::peers::wait_until;
use hermesd::actor::Actor;
use hermesd::db::Db;
use serde_json::json;

fn owner_comment(id: &str, body: &str, reply_to: Option<&str>) -> Request {
    Request::ItemComment(c::ItemAddComment {
        id: id.to_string(),
        body: body.to_string(),
        reply_to: reply_to.map(str::to_string),
        asks_owner: None,
    })
}

/// The bodies of the messages in `bot`'s conversation with the owner.
fn told(db: &Db, bot: &str) -> Vec<String> {
    let conv = db.dm_conversation(bot).unwrap().expect("a DM conversation");
    db.list_messages(&conv.id, None, 100)
        .unwrap()
        .into_iter()
        .map(|m| m.body)
        .collect()
}

#[tokio::test]
async fn the_owners_reply_reaches_the_pc_bot_and_closes_its_question() {
    let mut b = board().await;
    // Only an asker, not the assignee: the reply must reach it as one.
    let mac_db = &b.p.mac.app.db;
    let item = mac_db.board_read(|t| t.item(&b.item)).unwrap().unwrap();
    mac_db
        .assign_item(&b.item, item.version, None, &Actor::User)
        .unwrap();

    let asked = b
        .tester
        .call(
            "item_comment",
            json!({"id": b.item, "body": "Ship the PC build too?", "asks_owner": true}),
        )
        .await;
    let question_comment = asked["comment"]["id"]
        .as_str()
        .expect("comment id")
        .to_string();
    assert!(asked["owner_question"].is_string(), "{asked}");

    // (c) The PC's row names the comment that asked.
    let rows = hermesd::overview::attention_rows(&b.p.win.app, &b.win_app).expect("rows");
    let row = rows
        .rows
        .iter()
        .find(|r| r.title.starts_with("tester asks on"))
        .expect("the question is the PC's row");
    assert_eq!(row.question_comment_id, question_comment);

    // The owner replies on the Mac, the board's home.
    let Response::Edited(edited) = response(
        call(
            &mut b.p.mac_client,
            owner_comment(&b.item, "Yes, ship it", Some(&question_comment)),
        )
        .await,
    ) else {
        panic!("expected an edit result");
    };
    assert!(edited.told.contains(&b.stand_in), "{:?}", edited.told);

    // (b) The PC's tester gets the reply, through its stand-in.
    let (win, tester) = (&b.p.win.app, b.tester_id.clone());
    let reply = format!(
        "[card {}] Owner replied to your question on the card: Yes, ship it",
        b.item
    );
    wait_until("the reply reaches the PC's tester", || {
        told(&win.db, &tester).iter().any(|m| m.starts_with(&reply))
    })
    .await;

    // (a) The PC closes the question once its mirror shows the comment.
    let win_app = b.win_app.clone();
    wait_until("the PC closes the question", || {
        hermesd::overview::attention_rows(win, &win_app)
            .expect("rows")
            .rows
            .iter()
            .all(|r| !r.title.starts_with("tester asks on"))
    })
    .await;
}

#[tokio::test]
async fn a_later_comment_doesnt_reach_an_answered_asker() {
    let mut b = board().await;
    let mac_db = &b.p.mac.app.db;
    let item = mac_db.board_read(|t| t.item(&b.item)).unwrap().unwrap();
    mac_db
        .assign_item(&b.item, item.version, None, &Actor::User)
        .unwrap();
    b.tester
        .call(
            "item_comment",
            json!({"id": b.item, "body": "Which colour?", "asks_owner": true}),
        )
        .await;
    for body in ["Blue", "And a note for the team"] {
        let Response::Edited(edited) =
            response(call(&mut b.p.mac_client, owner_comment(&b.item, body, None)).await)
        else {
            panic!("expected an edit result");
        };
        // The first comment answers it; the second is for whoever holds the card.
        assert_eq!(edited.told.contains(&b.stand_in), body == "Blue", "{body}");
    }
}
