//! The releases migration and storage.

use super::releases::NewRelease;
use super::Db;
use crate::board::release::model::ReleaseEvent;

/// A downgrade and reinstall can rewind `schema_version`: the releases
/// migration then runs again over its own tables, and keeps their rows.
#[test]
fn the_releases_migration_runs_again_without_losing_packages() {
    let db = Db::open_in_memory().unwrap();
    let p = db.create_project("The Hermes", "the-hermes").unwrap();
    db.ensure_board(&p.id, "d-mac", Some("H")).unwrap();
    let release = db
        .board_tx(|t| {
            t.insert_release(&NewRelease {
                project_id: &p.id,
                name: "0.16.0",
                display_version: None,
                changelog: "",
                how_to_test: &serde_json::json!([]),
                created_by: "ops",
                items: &[],
            })
        })
        .unwrap();
    let event = ReleaseEvent {
        release_id: "gone".into(),
        release_name: "0.16.1".into(),
        related_id: Some(release.id.clone()),
        kind: "cancelled".into(),
        actor: "ops".into(),
        note: None,
        detail: serde_json::json!({}),
        at: bus::now(),
    };
    db.board_tx(|t| t.record_release_event(&event, &p.id))
        .unwrap();
    let ours: Vec<&str> = bus::schema::MIGRATIONS
        .iter()
        .copied()
        .filter(|m| m.contains("CREATE TABLE IF NOT EXISTS release"))
        .collect();
    assert_eq!(
        ours.len(),
        6,
        "releases, release events, build commits, required machines, plans and work cards"
    );
    for sql in ours {
        db.lock().execute_batch(sql).unwrap();
    }
    let again = db.board_read(|t| t.release(&release.id)).unwrap().unwrap();
    assert_eq!(again.name, "0.16.0");
    assert_eq!(again.events, vec![event], "a successor's event shows on it");
}
