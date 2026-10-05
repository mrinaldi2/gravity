//! Meetings' storage and the parts with no daemon around them; the flow
//! over MCP and the WebSocket is in tests/meetings.rs.

use chrono::{TimeZone, Utc};
use serde_json::json;

use super::actions::parse_due;
use super::model::{
    ActionItem, ActionStatus, Attendee, Contribution, Meeting, MeetingStatus, MeetingType, Series,
};
use super::next_at;
use super::series::routine_prompt;
use crate::db::Db;

fn series(project_id: &str) -> Series {
    let now = Utc::now();
    Series {
        id: "s1".into(),
        project_id: project_id.into(),
        meeting_type: MeetingType::Standup,
        name: "Daily standup".into(),
        cron: "0 0 9 * * Mon-Fri".into(),
        tz: "Europe/Rome".into(),
        facilitator: "sm".into(),
        attendees: vec!["dev".into(), "owner".into()],
        input_scope: "board".into(),
        enabled: true,
        routine_id: None,
        created_at: now,
        updated_at: now,
    }
}

fn meeting(project_id: &str, id: &str) -> Meeting {
    Meeting {
        id: id.into(),
        project_id: project_id.into(),
        series_id: Some("s1".into()),
        meeting_type: MeetingType::Standup,
        name: "Daily standup".into(),
        scheduled_at: Some(Utc::now()),
        started_at: Some(Utc::now()),
        closed_at: None,
        status: MeetingStatus::Collecting,
        skip_reason: None,
        facilitator: "sm".into(),
        inputs_snapshot: json!({ "board": null }),
        outputs: json!({}),
        summary: String::new(),
        attendees: ["dev", "owner"]
            .map(|who| Attendee {
                who: who.into(),
                required: true,
                contributed_at: None,
            })
            .to_vec(),
        contributions: Vec::new(),
        actions: Vec::new(),
    }
}

/// A downgrade and reinstall can rewind `schema_version`: the meetings
/// migration then runs again over its own tables, and keeps their rows.
#[test]
fn the_meetings_migration_runs_again_without_losing_meetings() {
    let db = Db::open_in_memory().unwrap();
    let p = db.create_project("The Hermes", "the-hermes").unwrap();
    db.board_tx(|t| {
        t.upsert_series(&series(&p.id))?;
        t.insert_meeting(&meeting(&p.id, "MTG-1"))
    })
    .unwrap();
    let ours: Vec<&str> = bus::schema::MIGRATIONS
        .iter()
        .copied()
        .filter(|m| m.contains("CREATE TABLE IF NOT EXISTS meeting_series"))
        .collect();
    assert_eq!(ours.len(), 1);
    db.rerun_sql(ours[0]).unwrap();
    let again = db.board_read(|t| t.meeting("MTG-1")).unwrap().unwrap();
    assert_eq!(again.attendees.len(), 2);
    assert_eq!(db.board_read(|t| t.series_list(&p.id)).unwrap().len(), 1);
}

#[test]
fn contributions_count_once_per_attendee_and_actions_carry_over() {
    let db = Db::open_in_memory().unwrap();
    let p = db.create_project("The Hermes", "the-hermes").unwrap();
    let now = Utc::now();
    let action = ActionItem {
        id: "a1".into(),
        project_id: p.id.clone(),
        meeting_id: "MTG-1".into(),
        series_id: Some("s1".into()),
        text: "Fix the flaky test".into(),
        owner: "dev".into(),
        due_at: None,
        status: ActionStatus::Open,
        item_id: None,
        created_at: now,
        updated_at: now,
    };
    db.board_tx(|t| {
        t.upsert_series(&series(&p.id))?;
        t.insert_meeting(&meeting(&p.id, "MTG-1"))?;
        t.insert_meeting(&meeting(&p.id, "MTG-2"))?;
        t.insert_action(&action)?;
        for section in ["yesterday", "today"] {
            let c = Contribution {
                id: bus::new_id(),
                author: "dev".into(),
                section: section.into(),
                body: "work".into(),
                item_refs: vec![],
                at: Utc::now(),
            };
            t.add_contribution("MTG-1", &c)?;
        }
        Ok(())
    })
    .unwrap();
    let first = db.board_read(|t| t.meeting("MTG-1")).unwrap().unwrap();
    assert_eq!(first.contributed(), (1, 2));
    assert_eq!(first.contributions.len(), 2);
    assert_eq!(first.actions.len(), 1);
    // Not carried over into its own meeting; carried into the next.
    assert!(db
        .board_read(|t| t.carried_over(&first))
        .unwrap()
        .is_empty());
    let next = db.board_read(|t| t.meeting("MTG-2")).unwrap().unwrap();
    let carried = db.board_read(|t| t.carried_over(&next)).unwrap();
    assert_eq!(carried.len(), 1);
    assert_eq!(carried[0].id, "a1");
}

#[test]
fn due_dates_take_rfc3339_or_a_day() {
    assert_eq!(parse_due("").unwrap(), None);
    assert_eq!(
        parse_due("2026-10-07").unwrap(),
        Some(Utc.with_ymd_and_hms(2026, 10, 7, 23, 59, 59).unwrap())
    );
    assert_eq!(
        parse_due("2026-10-07T17:00:00+02:00").unwrap(),
        Some(Utc.with_ymd_and_hms(2026, 10, 7, 15, 0, 0).unwrap())
    );
    assert!(parse_due("next week").is_err());
}

#[test]
fn a_series_next_time_follows_its_cron_while_enabled() {
    let mut s = series("p");
    // A Saturday: the next weekday 09:00 in Rome is Monday 07:00 UTC.
    let saturday = Utc.with_ymd_and_hms(2026, 10, 3, 12, 0, 0).unwrap();
    assert_eq!(
        next_at(&s, saturday),
        Some(Utc.with_ymd_and_hms(2026, 10, 5, 7, 0, 0).unwrap())
    );
    s.enabled = false;
    assert_eq!(next_at(&s, saturday), None);
    let prompt = routine_prompt(&s);
    assert!(
        prompt.contains("meeting_start with series_id \"s1\""),
        "{prompt}"
    );
}
