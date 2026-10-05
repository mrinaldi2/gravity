//! A release that changes an item's package without moving it still pushes
//! its card (H-113): its version moved, and a mirror or open board that
//! kept the old one would see its next write conflict.

mod common;

use bus::contract::board::{self as c, board_request::Request};
use common::board::{call, next_push};
use common::releases::{releases, rule};
use common::WsClient;
use serde_json::json;

#[tokio::test]
async fn a_successor_pushes_the_shipped_item_it_takes_over() {
    let mut r = releases(2).await;
    let (a, b) = (r.items[0].clone(), r.items[1].clone());
    let first = r.submitted("0.16.0").await;
    let mut owner = WsClient::connect(&r.pair.d).await;
    rule(
        &mut owner,
        &first,
        json!([{"item_id": a, "verdict": "ship"}, {"item_id": b, "verdict": "rework"}]),
    )
    .await;

    let mut watcher = WsClient::connect(&r.pair.d).await;
    call(
        &mut watcher,
        Request::BoardWatch(c::BoardWatch {
            project_id: r.project.clone(),
        }),
    )
    .await;
    let column = r.column(&a);
    r.package(
        "0.16.1",
        json!({"name": "0.16.1", "items": [a], "from": first["id"]}),
    )
    .await;
    assert_eq!(r.column(&a), column, "it stayed in Owner testing");

    let now = r.pair.d.app.db.get_item(&a).unwrap().unwrap().version;
    loop {
        let push = next_push(&mut watcher).await.expect("a push for the item");
        if push.item_id == a && push.card.as_ref().is_some_and(|card| card.version == now) {
            break;
        }
    }
}
