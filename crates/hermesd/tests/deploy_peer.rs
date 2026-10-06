//! A release installing on a tester's own computer, the board on another:
//! - H-158: the deploy task forwarded there counts as on the board (G4).
//!   That computer has no `release_deployment` row for it; the task frame
//!   carries the release instead.
//! - H-166: `install_quiesce` pauses the tester's computer, never the
//!   board's home, and `install_release` says how to run the install there.

mod common;

use bus::{BotState, PermissionExtra};
use chrono::{Duration, Utc};
use common::peer_board::{approved_release, board};
use common::peers::wait_until;
use serde_json::json;

#[tokio::test]
async fn the_pcs_tester_pauses_the_pc_not_the_boards_home() {
    let mut b = board().await;
    let (mut ops, id) = approved_release(&mut b).await;
    ops.call(
        "release_deploy",
        json!({"release_id": id, "machine": "win"}),
    )
    .await;
    let (win, mac) = (b.p.win.app.clone(), b.p.mac.app.clone());
    let tester = b.tester_id.clone();
    wait_until("the PC holds the deploy task", || {
        win.db
            .open_tasks_for(&tester)
            .is_ok_and(|tasks| !tasks.is_empty())
    })
    .await;
    // Its own computer's extras, as the owner grants them there.
    win.db
        .set_bot_permission_extras(&tester, &[PermissionExtra::Install, PermissionExtra::Quiesce])
        .unwrap();

    let builds = b.tester.call("install_release", json!({"release_id": id})).await;
    let command = builds["command"].as_str().unwrap_or_default();
    assert!(command.ends_with(&format!(" release install {id}")), "{builds}");
    let started = b
        .tester
        .call("install_quiesce", json!({"release_id": id, "action": "start"}))
        .await;
    assert_eq!(started["quiesce"]["exempt_bot"], json!(tester), "{started}");
    assert_eq!(started["quiesce"]["reason"], "install of 0.16.0");
    assert!(win.db.open_quiesce().unwrap().is_some(), "the PC is paused");
    assert!(mac.db.open_quiesce().unwrap().is_none(), "the home is not");

    // An older PC that still forwards the tool gets a refusal, not a pause.
    let link = &win.db.project_links(&b.win_app).unwrap()[0];
    let frame = json!({"type": "board_call", "project_id": link.project_id,
                       "bot_id": tester, "tool": "install_quiesce",
                       "args": {"release_id": id, "action": "start"}});
    let refused = win.peers.request(&link.peer_id, frame).await;
    assert!(format!("{refused:?}").contains("doesn't pause itself"), "{refused:?}");
    assert!(mac.db.open_quiesce().unwrap().is_none());

    let resumed = b
        .tester
        .call("install_quiesce", json!({"release_id": id, "action": "resume"}))
        .await;
    assert_eq!(resumed["resumed"]["outcome"], "aborted", "{resumed}");
}

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
