//! The daemon's own merge-queue ticker (H-271; ARCH should-fix on H-308).
//! Every other PR test stops it and steps the queue on its own clock; this
//! one leaves it running on the real clock, and checks that a merge window
//! that has expired is handed to DevOps by the ticker alone.
//!
//! Not a race: whatever the ticker did first (queue the PR, or open its
//! window), the test then makes the row an expired window; the ticker's next
//! pass hands it on. Nominally that is within one `EVERY`; the bound is
//! generous for a loaded computer.

mod common;

use std::time::{Duration, Instant};

use bus::PermissionExtra;
use chrono::Utc;
use common::prs::{approve, give, opened, setup_with};
use common::WsClient;
use hermesd::board::model::Role;
use hermesd::prs::queue::{EVERY, WINDOW};
use serde_json::json;

const DEVOPS: usize = 0;

#[tokio::test]
async fn the_ticker_hands_an_expired_window_to_devops() {
    let mut r = setup_with(|cfg| cfg.merge_queue_by_hand_for_tests(false)).await;
    give(&r, DEVOPS, Role::Devops);
    let db = r.pair.d.app.db.clone();
    db.set_bot_permission_extras(&r.pair.ids[DEVOPS], &[PermissionExtra::PrMerge])
        .unwrap();
    let (_, head) = opened(&mut r).await;
    approve(&mut r, &head).await;
    let mut owner = WsClient::connect(&r.pair.d).await;
    let ok = owner
        .request(json!({"type": "pr_review_submit", "project_id": r.project,
                        "number": 1, "sha": head, "verdict": "approved"}))
        .await;
    assert_ne!(ok["type"], "error", "{ok}");
    let pr = db.board_read(|t| t.pr(&r.project, 1)).unwrap().unwrap();

    // The ticker queues the mergeable PR by itself.
    let generous = EVERY * 3 + Duration::from_secs(20);
    let row = |db: &hermesd::db::Db| db.board_read(|t| t.queue_row(&pr.id)).unwrap();
    let start = Instant::now();
    while row(&db).is_none() {
        assert!(start.elapsed() < generous, "the ticker never queued PR #1");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    // Its window ended a while ago; nothing but the ticker moves it now.
    let patch = row(&db).unwrap().patch_id;
    let past = Utc::now() - WINDOW - chrono::Duration::seconds(5);
    db.board_tx(|t| t.set_queue_state(&pr.id, "window", Some(past), None, &patch, past))
        .unwrap();
    let start = Instant::now();
    while row(&db).map(|r| r.state) != Some("handed".to_string()) {
        assert!(
            start.elapsed() < generous,
            "an expired window wasn't handed on: {:?}",
            row(&db)
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let handed = row(&db).unwrap();
    let task = handed.task_id.expect("the pr_merge task");
    let task = db.get_task(&task).unwrap().unwrap();
    assert_eq!(task.to_bot_id, r.pair.ids[DEVOPS], "it goes to DevOps");
}
