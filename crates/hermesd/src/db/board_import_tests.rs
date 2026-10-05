//! The backlog import's storage (B6): key adoption, kept and new ids,
//! idempotent re-runs, dry runs, and the home-only rule.

use crate::board::import::{
    is_imported, parse, AWAITING_OWNER_LABEL, DEPLOYING_LABEL, NOT_IMPORTED_SQL,
};
use crate::board::model::*;
use crate::board::moves::{item_move, MoveRequest, Moved, CLOSED_WITHOUT_RELEASE};

use super::board_tests::new_item;
use super::{Actor, Db};

const FIXTURE: &str = include_str!("../../tests/fixtures/board_import_backlog.md");
const OWNER: Actor<'static> = Actor::User;

/// A board as a fresh project gets it: keyed by its name, empty.
fn board(key: &str) -> (Db, String) {
    let db = Db::open_in_memory().unwrap();
    let p = db.create_project("gravity", "gravity").unwrap();
    for (name, dir) in [("Desktop Dev", "/tmp/a"), ("Architect", "/tmp/b")] {
        db.create_bot(&p.id, name, "", "", "", dir, name, None)
            .unwrap();
    }
    db.ensure_board(&p.id, "d-home", Some(key)).unwrap();
    (db, p.id)
}

#[test]
fn an_empty_board_takes_the_key_and_every_entry_keeps_or_gets_an_id() {
    let (db, p) = board("G");
    let parsed = parse(FIXTURE);
    let report = db
        .board_import(&p, "d-home", &parsed, false, &OWNER)
        .unwrap();

    assert_eq!(
        (report.key_before.as_str(), report.key.as_str()),
        ("G", "H")
    );
    assert_eq!(db.board_settings(&p).unwrap().unwrap().key, "H");
    assert_eq!(report.created.len(), 14);
    assert_eq!(report.skipped.len(), 4);
    let ids: Vec<(&str, &str)> = report
        .created
        .iter()
        .map(|c| (c.source_id.as_str(), c.id.as_str()))
        .collect();
    // Kept ids keep theirs; the others follow the highest kept one, in order.
    assert!(ids.contains(&("H-040", "H-040")));
    assert!(ids.contains(&("R1-I2", "H-041")));
    assert!(ids.contains(&("B1", "H-042")));
    assert!(ids.contains(&("REL-D-1", "H-043")));
    assert!(ids.contains(&("U2", "H-044")));
    assert_eq!(report.next_seq, 45);

    let h1 = db.get_item("H-001").unwrap().unwrap();
    assert_eq!(
        (h1.column_key.as_str(), h1.category),
        ("done", ColumnCategory::Done)
    );
    assert!(h1.done_at.is_some());
    let architect = db.list_bots(Some(&p)).unwrap();
    let architect = architect.iter().find(|b| b.name == "Architect").unwrap();
    assert_eq!(h1.assignee.as_deref(), Some(architect.id.as_str()));
    let links = db.item_links("H-001").unwrap();
    assert_eq!(links.len(), 2);
    assert!(links.iter().all(|l| l.kind == LinkKind::Decision));

    let renamed = db.get_item("H-041").unwrap().unwrap();
    assert_eq!(renamed.labels, ["rename", "was:R1-I2"]);
    assert_eq!(renamed.category, ColumnCategory::Review);
    let events = db.item_events("H-041").unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].kind, ItemEventKind::Created);
    assert_eq!(events[0].note.as_deref(), Some("imported from backlog.md"));

    // The next item made on the board follows the imported ones.
    let next = db.create_item(&new_item(&p, "After"), &OWNER).unwrap();
    assert_eq!(next.id, "H-045");
}

#[test]
fn a_rerun_creates_nothing_and_a_dry_run_writes_nothing() {
    let (db, p) = board("G");
    let parsed = parse(FIXTURE);
    let dry = db
        .board_import(&p, "d-home", &parsed, true, &OWNER)
        .unwrap();
    assert!(dry.dry_run);
    assert_eq!(dry.created.len(), 14);
    assert_eq!(dry.key, "H");
    assert!(db.board_cards(&p).unwrap().is_empty(), "nothing kept");
    let settings = db.board_settings(&p).unwrap().unwrap();
    assert_eq!((settings.key.as_str(), settings.next_seq), ("G", 1));

    let first = db
        .board_import(&p, "d-home", &parsed, false, &OWNER)
        .unwrap();
    let again = db
        .board_import(&p, "d-home", &parsed, false, &OWNER)
        .unwrap();
    assert!(again.created.is_empty());
    assert_eq!(again.existing.len(), 14);
    assert_eq!(again.next_seq, first.next_seq);
    assert_eq!(db.board_cards(&p).unwrap().len(), 14);
}

#[test]
fn ids_made_on_the_board_are_not_taken_for_imported_ones() {
    let (db, p) = board("H");
    let made = db.create_item(&new_item(&p, "Made here"), &OWNER).unwrap();
    assert_eq!(made.id, "H-001");
    let report = db
        .board_import(&p, "d-home", &parse(FIXTURE), false, &OWNER)
        .unwrap();
    let clash = report.skipped.iter().find(|s| s.id == "H-001").unwrap();
    assert!(clash.reason.contains("made there"), "{}", clash.reason);
    assert_eq!(db.get_item("H-001").unwrap().unwrap().title, "Made here");
    assert_eq!(report.created.len(), 13);
}

#[test]
fn it_runs_only_on_the_home_and_never_renames_items() {
    let (db, p) = board("G");
    let parsed = parse(FIXTURE);
    let err = db
        .board_import(&p, "d-elsewhere", &parsed, false, &OWNER)
        .unwrap_err();
    assert!(err.to_string().contains("home"), "{err}");

    db.create_item(&new_item(&p, "Already here"), &OWNER)
        .unwrap();
    let err = db
        .board_import(&p, "d-home", &parsed, false, &OWNER)
        .unwrap_err();
    assert!(err.to_string().contains("already has items"), "{err}");
    assert_eq!(db.board_settings(&p).unwrap().unwrap().key, "G");

    let none = Db::open_in_memory().unwrap();
    let q = none.create_project("q", "q").unwrap();
    let err = none
        .board_import(&q.id, "d-home", &parsed, false, &OWNER)
        .unwrap_err();
    assert!(err.to_string().contains("no board"), "{err}");
}

#[test]
fn nothing_lands_where_only_the_daemon_moves_it() {
    let (db, p) = board("G");
    db.board_import(&p, "d-home", &parse(FIXTURE), false, &OWNER)
        .unwrap();
    let cards = db.board_cards(&p).unwrap();
    assert!(cards
        .iter()
        .all(|c| !matches!(c.column_key.as_str(), "approval" | "deploying")));
    let rel = db.get_item("H-043").unwrap().unwrap();
    assert_eq!(rel.category, ColumnCategory::Verify);
    assert_eq!(rel.labels, ["was:REL-D-1", DEPLOYING_LABEL]);
    let h11 = db.get_item("H-011").unwrap().unwrap();
    assert_eq!(h11.category, ColumnCategory::Verify);
    assert_eq!(h11.labels, [AWAITING_OWNER_LABEL]);
}

fn close(db: &Db, id: &str, actor: &Actor<'_>) -> Moved {
    let item = db.get_item(id).unwrap().unwrap();
    let req = MoveRequest {
        id,
        to: "done",
        expected_version: item.version,
        reason: Some("shipped in 0.14.0 before the board"),
        override_reason: None,
    };
    item_move(db, &req, actor).unwrap()
}

#[test]
fn the_owner_closes_an_imported_item_out_of_verify_and_it_is_logged() {
    let (db, p) = board("G");
    db.board_import(&p, "d-home", &parse(FIXTURE), false, &OWNER)
        .unwrap();
    let bots = db.list_bots(Some(&p)).unwrap();
    let dev = Actor::Bot {
        id: &bots.iter().find(|b| b.name == "Desktop Dev").unwrap().id,
        project_id: &p,
    };
    // H-043 (was REL-D-1): a feature in Verify, in no release.
    assert!(matches!(close(&db, "H-043", &dev), Moved::Refused(_)));
    let Moved::Done(item) = close(&db, "H-043", &OWNER) else {
        panic!("the owner may close it");
    };
    assert_eq!(item.category, ColumnCategory::Done);
    let event = db.item_events("H-043").unwrap().pop().unwrap();
    assert_eq!(event.kind, ItemEventKind::Moved);
    assert_eq!(
        event.note.as_deref(),
        Some(format!("{CLOSED_WITHOUT_RELEASE} · shipped in 0.14.0 before the board").as_str())
    );

    // In a release, it is the release's to move: refused for the owner too.
    db.conn
        .lock()
        .unwrap()
        .execute("UPDATE item SET release_id = 'R-1' WHERE id = 'H-003'", [])
        .unwrap();
    for actor in [&OWNER, &dev] {
        assert!(matches!(close(&db, "H-003", actor), Moved::Refused(_)));
    }
}

#[test]
fn flow_metrics_can_leave_imported_items_out() {
    let (db, p) = board("G");
    db.board_import(&p, "d-home", &parse(FIXTURE), false, &OWNER)
        .unwrap();
    let made = db.create_item(&new_item(&p, "Made here"), &OWNER).unwrap();
    let sql = format!("SELECT id FROM item WHERE project_id = ?1 AND {NOT_IMPORTED_SQL}");
    let counted: Vec<String> = {
        let conn = db.conn.lock().unwrap();
        let mut stmt = conn.prepare(&sql).unwrap();
        let rows = stmt.query_map([&p], |r| r.get(0)).unwrap();
        rows.map(Result::unwrap).collect()
    };
    assert_eq!(counted, [made.id.as_str()]);
    assert!(is_imported(&db.item_events("H-001").unwrap()));
    assert!(!is_imported(&db.item_events(&made.id).unwrap()));
}
