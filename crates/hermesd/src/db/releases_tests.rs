//! The releases migration and storage.

use super::releases::NewRelease;
use super::Db;

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
    let last = bus::schema::MIGRATIONS.last().unwrap();
    assert!(last.contains("CREATE TABLE IF NOT EXISTS release ("));
    db.lock().execute_batch(last).unwrap();
    let again = db.board_read(|t| t.release(&release.id)).unwrap();
    assert_eq!(again.map(|r| r.name), Some("0.16.0".to_string()));
}
