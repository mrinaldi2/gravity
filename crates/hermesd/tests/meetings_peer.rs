//! Meetings from a linked computer (H-102, ARCH-R48): the meeting tools are
//! board tools, so a bot whose board lives on a peer reaches them through
//! the B9 `board_call` path and acts there as its stand-in.

mod common;

use common::peer_board::board;
use common::tasks::error_text;
use hermesd::actor::Actor;
use hermesd::board::meetings::run::{self, AdHoc};
use hermesd::board::meetings::series::{self, SeriesEdit};
use hermesd::board::meetings::Party;
use serde_json::json;

#[tokio::test]
async fn a_linked_bot_contributes_to_a_meeting_on_the_boards_home() {
    let mut b = board().await;
    let mac = &b.p.mac.app;
    let project = mac.db.get_bot(&b.stand_in).unwrap().unwrap().project_id;
    let owner = Party::Owner(Actor::User);
    let attendees = ["tester".to_string()];
    let s = series::upsert(
        mac,
        &owner,
        &project,
        &SeriesEdit {
            meeting_type: Some("standup"),
            name: Some("Daily standup"),
            cron: Some("0 0 9 * * Mon-Fri"),
            facilitator: Some("lead"),
            attendees: &attendees,
            ..SeriesEdit::default()
        },
    )
    .unwrap();
    assert_eq!(s.attendees, vec![b.stand_in.clone()]);
    let none = AdHoc {
        name: "",
        attendees: &[],
    };
    let meeting = run::start(mac, &owner, &project, Some(&s.id), &none).unwrap();

    // The PC's tester contributes; the home records its stand-in.
    let got = b
        .tester
        .call(
            "meeting_contribute",
            json!({"meeting_id": meeting.id, "section": "today", "body": "Tested on win"}),
        )
        .await;
    assert_eq!(got["meeting"]["contributed"], 1, "{got}");
    let held = mac
        .db
        .board_read(|t| t.meeting(&meeting.id))
        .unwrap()
        .unwrap();
    assert_eq!(held.contributions.len(), 1);
    assert_eq!(held.contributions[0].author, b.stand_in);
    assert!(held.attendees[0].contributed_at.is_some());

    // And reads it there; the home's roles still decide what it may do.
    let read = b
        .tester
        .call("meeting_get", json!({"meeting_id": meeting.id}))
        .await;
    assert_eq!(read["meeting"]["contributions"][0]["body"], "Tested on win");
    let refused = b
        .tester
        .call_raw("action_promote", json!({"action_id": "nope"}))
        .await;
    assert!(error_text(&refused).contains("board role"), "{refused}");
}
