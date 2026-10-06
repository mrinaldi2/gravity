//! Owner threads and pins across linked computers (H-128 D6 d, D7 a): a
//! linked bot's thread and its questions live on its own computer and read
//! through its stand-in; its updates reach the other computer's clients; a
//! pin is forwarded to the computers the project is linked with.

mod common;

use common::peer_board::board;
use common::peers::wait_until;
use serde_json::{json, Value};

#[tokio::test]
async fn a_linked_bots_thread_reads_through_its_stand_in() {
    let mut b = board().await;
    let win_daemon = b.p.win.app.db.daemon_id().unwrap();
    // The PC's tester asks on the Mac's card: commented at the home, asked
    // on the PC.
    let sent = b
        .tester
        .call(
            "message_owner",
            json!({"body": "Is the PC build in scope?", "asks": true, "item": b.item}),
        )
        .await;
    assert_eq!(sent["asks"], true, "{sent}");
    let comments =
        b.p.mac
            .app
            .db
            .board_read(|t| t.item_comments(&b.item))
            .unwrap();
    assert!(
        comments
            .iter()
            .any(|c| c.body == "To the owner: Is the PC build in scope?"),
        "{comments:?}"
    );
    let item = b.item.clone();
    let asked = hermesd::overview::attention_rows(&b.p.win.app, &b.win_app).expect("rows");
    let question = asked
        .rows
        .iter()
        .find(|r| r.title.starts_with("tester asks:"))
        .expect("the question is the PC's row");
    assert_eq!(question.daemon_id, win_daemon);

    // The Mac's client hears of it, as the stand-in.
    let mac = &mut b.p.mac_client;
    let pushed = mac.wait_for(|v| v["type"] == "owner_thread_updated").await;
    assert_eq!(pushed["bot"]["daemon_id"], win_daemon.as_str());
    assert_eq!(pushed["bot"]["bot_id"], b.tester_id.as_str());
    assert_eq!(pushed["project_id"], b.mac_app.as_str());

    // owner_threads on the Mac lists the stand-in, as the PC answers for it.
    let listed = mac.request(json!({"type": "owner_threads"})).await;
    let entry = listed["owner_threads"]["threads"]
        .as_array()
        .expect("threads")
        .iter()
        .find(|t| t["bot"]["bot_id"] == b.tester_id.as_str())
        .cloned()
        .unwrap_or_else(|| panic!("no stand-in thread: {listed}"));
    assert_eq!(entry["project_id"], b.mac_app.as_str());
    assert_eq!(entry["unread"], 1, "{entry}");
    assert_eq!(entry["open_question"], true);
    assert_eq!(
        entry["last"]["text"],
        format!("[card {item}] Is the PC build in scope?")
    );

    // The page and the read mark go to the PC, by either id.
    for bot_id in [b.stand_in.as_str(), b.tester_id.as_str()] {
        let page = mac
            .request(json!({"type": "owner_thread_get", "bot_id": bot_id}))
            .await;
        assert_eq!(page["type"], "owner_thread", "{page}");
        assert_eq!(page["owner_thread"]["project_id"], b.mac_app.as_str());
        assert_eq!(page["owner_thread"]["messages"][0]["asks"], true);
    }
    let page = mac
        .request(json!({"type": "owner_thread_get", "bot_id": b.stand_in}))
        .await;
    let num: Value = page["owner_thread"]["messages"][0]["num"].clone();
    let marked = mac
        .request(json!({"type": "owner_thread_read", "bot_id": b.stand_in, "up_to_num": num}))
        .await;
    assert_eq!(marked["type"], "owner_thread_marked", "{marked}");
    assert_eq!(marked["owner_thread_marked"]["bot_id"], b.stand_in.as_str());
    assert_eq!(
        b.p.win.app.db.owner_unread(&b.tester_id).unwrap(),
        0,
        "read on the PC"
    );
}

#[tokio::test]
async fn a_pin_is_forwarded_to_the_linked_computer() {
    let mut b = board().await;
    let pinned =
        b.p.mac_client
            .request(json!({"type": "project_pin", "project_id": b.mac_app, "pinned": true}))
            .await;
    assert_eq!(pinned["type"], "project_pinned", "{pinned}");
    let (win, win_app) = (&b.p.win.app, b.win_app.clone());
    wait_until("the PC pins it too", || {
        win.db.project_pinned(&win_app).unwrap()
    })
    .await;
    let o = hermesd::overview::overview(win, &[]).expect("overview");
    assert_eq!(o.rows[0].project_id, b.win_app);
    assert!(o.rows[0].pinned);
    assert_eq!(o.rows[0].rank, 0);
}
