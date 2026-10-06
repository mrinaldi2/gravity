//! H-158: a deploy task forwarded to a tester on another computer counts as
//! on the board there (G4). That computer has no `release_deployment` row
//! for it; the task frame carries the release instead.

mod common;

use bus::BotState;
use chrono::{Duration, Utc};
use common::peer_board::{approved_release, board};
use common::peers::wait_until;
use serde_json::json;

#[tokio::test]
async fn a_deploy_task_forwarded_to_the_pcs_tester_is_on_the_board_there() {
    let mut b = board().await;
    let (mut ops, id) = approved_release(&mut b).await;
    let win = b.p.win.app.clone();
    let tester = b.tester_id.clone();

    // Working with no task, the PC's tester is flagged off the board.
    win.supervisor.set_state(&tester, BotState::Working, "test");
    let t0 = Utc::now();
    hermesd::offboard::sweep(&win, t0).unwrap();
    hermesd::offboard::sweep(&win, t0 + Duration::minutes(10)).unwrap();
    assert!(
        win.off_board.flagged_since(&tester).is_some(),
        "the PC watches its tester"
    );

    ops.call(
        "release_deploy",
        json!({"release_id": id, "machine": "win"}),
    )
    .await;
    wait_until("the PC holds the deploy task", || {
        win.db
            .open_tasks_for(&tester)
            .is_ok_and(|tasks| !tasks.is_empty())
    })
    .await;
    let task = win.db.open_tasks_for(&tester).unwrap().remove(0);
    assert_eq!(
        win.db.task_release(&task.id).unwrap().as_deref(),
        Some(id.as_str()),
        "the frame named the release, by the home's id"
    );
    assert_eq!(win.db.task_card(&task.id).unwrap(), None, "and no card");
    assert!(win.db.task_on_board(&task.id).unwrap());

    win.supervisor.set_state(&tester, BotState::Working, "test");
    hermesd::offboard::sweep(&win, t0 + Duration::minutes(11)).unwrap();
    hermesd::offboard::sweep(&win, t0 + Duration::minutes(25)).unwrap();
    assert!(
        win.off_board.flagged_since(&tester).is_none(),
        "a forwarded deploy task is on the board on the PC"
    );
}
