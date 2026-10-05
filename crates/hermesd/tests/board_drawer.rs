//! The owner's item drawer over WebSocket (U4, H-018 §3.6): a comment is
//! written as the owner, pushed to watchers, and shows in the item's detail.
//! Other edits stay refused until the drawer edits inline.

mod common;

use bus::contract::board::{self as c, board_request::Request, board_response::Response};
use common::board::*;
use common::*;
use hermesd::actor::Actor;
use hermesd::board::model::{ItemType, Platform, Priority};
use hermesd::db::NewItem;

fn comment(id: &str, body: &str) -> Request {
    Request::ItemComment(c::ItemAddComment {
        id: id.to_string(),
        body: body.to_string(),
        reply_to: None,
    })
}

#[tokio::test]
async fn the_owner_comments_from_the_drawer() {
    let d = spawn_daemon().await;
    let mut owner = WsClient::connect(&d).await;
    let mut watcher = WsClient::connect(&d).await;
    let project_id = common::peers::project(&mut owner, "The Hermes").await;
    snapshot(
        call(
            &mut owner,
            Request::BoardGet(c::BoardGet {
                project_id: project_id.clone(),
            }),
        )
        .await,
    );
    let item = d
        .app
        .db
        .create_item(
            &NewItem {
                project_id: &project_id,
                item_type: ItemType::Feature,
                title: "Drawer",
                description: "",
                platforms: &[Platform::Desktop],
                size: None,
                priority: Priority::P2,
                labels: &[],
                parent_id: None,
                acceptance_criteria: &[],
            },
            &Actor::User,
        )
        .expect("item");
    snapshot(
        call(
            &mut watcher,
            Request::BoardWatch(c::BoardWatch {
                project_id: project_id.clone(),
            }),
        )
        .await,
    );

    assert_eq!(
        error_code(call(&mut owner, comment(&item.id, "  ")).await),
        "invalid_request"
    );
    let Response::Edited(edited) =
        response(call(&mut owner, comment(&item.id, "Looks right")).await)
    else {
        panic!("expected an edit result");
    };
    assert!(matches!(
        edited.outcome,
        Some(c::edit_result::Outcome::Done(_))
    ));
    let push = next_push(&mut watcher).await.expect("a push");
    assert_eq!(push.item_id, item.id);

    let Response::Item(detail) = response(
        call(
            &mut owner,
            Request::ItemGet(c::ItemGet {
                id: item.id.clone(),
            }),
        )
        .await,
    ) else {
        panic!("expected the item");
    };
    let posted = detail.comments.last().expect("the comment");
    assert_eq!(
        (posted.author.as_str(), posted.body.as_str()),
        ("user", "Looks right")
    );

    assert_eq!(
        error_code(call(&mut owner, comment("H-999", "x")).await),
        "not_found"
    );
    // A device without the control grant only reads.
    let created = owner
        .request(serde_json::json!({"type": "create_device", "name": "phone",
                                    "capabilities": ["read"]}))
        .await;
    let mut phone = connect_with(&d, token_str(&created)).await;
    assert_eq!(
        error_code(call(&mut phone, comment(&item.id, "x")).await),
        "forbidden"
    );
}
