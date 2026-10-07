//! Comments on a card reach the team (H-201): `item_get` gives bots each
//! comment's body and author, and the owner's comment is delivered to the
//! card's assignee and the lead as the owner's note on the card, and a
//! dismissed question tells the bot that asked (H-211).

mod common;

use bus::contract::board::{self as c, board_request::Request, board_response::Response};
use bus::MessageKind;
use common::board::{call, new_item, response};
use common::tasks::project_with_bots;
use common::*;
use hermesd::actor::Actor;
use hermesd::board::model::Priority;
use hermesd::db::Db;
use serde_json::json;

const LEAD: usize = 0;
const ASSIGNEE: usize = 1;
const OTHER: usize = 2;

fn comment(id: &str, body: &str) -> Request {
    Request::ItemComment(c::ItemAddComment {
        id: id.to_string(),
        body: body.to_string(),
        reply_to: None,
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
async fn the_owners_comment_reaches_the_assignee_and_the_lead() {
    let (pair, mut bots) = project_with_bots(&["Team Lead", "Desktop Dev", "iOS Dev"]).await;
    let db = &pair.d.app.db;
    let project_id = db.get_bot(&pair.ids[LEAD]).unwrap().unwrap().project_id;
    db.ensure_board(&project_id, &db.daemon_id().unwrap(), Some("H"))
        .unwrap();
    db.set_project_lead(&project_id, Some(&pair.ids[LEAD]))
        .unwrap();
    let (card, version) = new_item(db, &project_id, "Comments", Priority::P1);
    db.assign_item(&card, version, Some(&pair.ids[ASSIGNEE]), &Actor::User)
        .unwrap();
    let before: Vec<usize> = pair.ids.iter().map(|id| told(db, id).len()).collect();

    // A bot's comment tells no one: the owner reads it on the card.
    bots[OTHER]
        .call("item_comment", json!({"id": card, "body": "Can I help?"}))
        .await;
    let mut owner = WsClient::connect(&pair.d).await;
    let Response::Edited(edited) =
        response(call(&mut owner, comment(&card, "Ship it today")).await)
    else {
        panic!("expected an edit result");
    };
    // The reply names who was told: the assignee and the lead.
    assert_eq!(
        edited.told,
        [pair.ids[ASSIGNEE].clone(), pair.ids[LEAD].clone()]
    );
    let line = format!("[card {card}] Owner commented on the card: Ship it today\n");
    for (i, id) in pair.ids.iter().enumerate() {
        let new = told(db, id).split_off(before[i]);
        if i == OTHER {
            assert!(new.is_empty(), "bot {i}: {new:?}");
        } else {
            assert_eq!(new.len(), 1, "bot {i}: {new:?}");
            assert!(new[0].starts_with(&line), "bot {i}: {new:?}");
            assert!(new[0].contains("item_comment with reply_to"), "{new:?}");
        }
    }

    // The lead is told once when it is the assignee too.
    let item = db.board_read(|t| t.item(&card)).unwrap().unwrap();
    db.assign_item(&card, item.version, Some(&pair.ids[LEAD]), &Actor::User)
        .unwrap();
    let lead_before = told(db, &pair.ids[LEAD]).len();
    response(call(&mut owner, comment(&card, "Thanks")).await);
    assert_eq!(told(db, &pair.ids[LEAD]).len(), lead_before + 1);

    // A note, not a chat (ARCH S2): the bot answers on the card, and H-192
    // never posts its turn to the owner's thread.
    let conv = db.dm_conversation(&pair.ids[LEAD]).unwrap().unwrap();
    let last = db
        .list_messages(&conv.id, None, 100)
        .unwrap()
        .pop()
        .unwrap();
    assert_eq!(last.kind, MessageKind::Note);
}

/// UX-042: dismissing a card question tells the bot that asked, so it
/// doesn't wait for an answer.
#[tokio::test]
async fn a_dismissed_question_tells_the_bot_that_asked() {
    let (pair, mut bots) = project_with_bots(&["Team Lead", "Desktop Dev"]).await;
    let db = &pair.d.app.db;
    let project_id = db.get_bot(&pair.ids[LEAD]).unwrap().unwrap().project_id;
    db.ensure_board(&project_id, &db.daemon_id().unwrap(), Some("H"))
        .unwrap();
    let (card, _) = new_item(db, &project_id, "Questions", Priority::P1);
    let asked = bots[ASSIGNEE]
        .call(
            "item_comment",
            json!({"id": card, "body": "Which colour?", "asks_owner": true}),
        )
        .await;
    let rows = hermesd::overview::attention_rows(&pair.d.app, &project_id).expect("rows");
    let row = rows
        .rows
        .iter()
        .find(|r| r.title.starts_with("Desktop Dev asks on"))
        .expect("the question's row");
    // The row names the comment that asked (H-211 c).
    assert_eq!(
        row.question_comment_id,
        asked["comment"]["id"].as_str().unwrap()
    );

    let before = told(db, &pair.ids[ASSIGNEE]).len();
    let mut owner = WsClient::connect(&pair.d).await;
    let dismissed = owner
        .request(json!({"type": "attention_dismiss", "id": row.id}))
        .await;
    assert_eq!(dismissed["type"], "attention_dismissed", "{dismissed}");
    let new = told(db, &pair.ids[ASSIGNEE]).split_off(before);
    assert_eq!(new.len(), 1, "{new:?}");
    assert!(
        new[0].starts_with(&format!(
            "[card {card}] The owner dismissed your question on the card"
        )),
        "{new:?}"
    );
}

#[tokio::test]
async fn item_get_gives_bots_each_comments_body_and_author() {
    let (pair, mut bots) = project_with_bots(&["Team Lead", "Desktop Dev"]).await;
    let db = &pair.d.app.db;
    let project_id = db.get_bot(&pair.ids[LEAD]).unwrap().unwrap().project_id;
    db.ensure_board(&project_id, &db.daemon_id().unwrap(), Some("H"))
        .unwrap();
    let (card, _) = new_item(db, &project_id, "Comments", Priority::P1);
    let mut owner = WsClient::connect(&pair.d).await;
    response(call(&mut owner, comment(&card, "Why is this slow?")).await);
    bots[ASSIGNEE]
        .call("item_comment", json!({"id": card, "body": "Looking now"}))
        .await;

    let got = bots[LEAD].call("item_get", json!({"id": card})).await;
    let comments: Vec<(String, String)> = got["comments"]
        .as_array()
        .expect("comments")
        .iter()
        .map(|c| {
            (
                c["author_name"].as_str().unwrap().to_string(),
                c["body"].as_str().unwrap().to_string(),
            )
        })
        .collect();
    assert_eq!(
        comments,
        [
            ("owner".to_string(), "Why is this slow?".to_string()),
            ("Desktop Dev".to_string(), "Looking now".to_string()),
        ],
        "{got}"
    );
    assert_eq!(
        got["comments"][1]["author"],
        format!("bot:{}", pair.ids[ASSIGNEE])
    );
}
