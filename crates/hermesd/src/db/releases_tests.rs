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
        5,
        "releases, release events, build commits, required machines and plans"
    );
    for sql in ours {
        db.lock().execute_batch(sql).unwrap();
    }
    let again = db.board_read(|t| t.release(&release.id)).unwrap().unwrap();
    assert_eq!(again.name, "0.16.0");
    assert_eq!(again.events, vec![event], "a successor's event shows on it");
}

/// H-176: an iOS package frozen with the desktop testers' computers as deploy
/// targets deploys to the owner's iPhone instead, once and again; a desktop
/// package, an owner's own list and a package already deploying stay.
#[test]
fn an_ios_package_frozen_on_desktop_computers_deploys_to_the_iphone() {
    use crate::board::release::model::{DeployAction, ReleaseBuild, ReleaseTargets};
    let db = Db::open_in_memory().unwrap();
    let p = db.create_project("The Hermes", "the-hermes").unwrap();
    db.ensure_board(&p.id, "d-mac", Some("H")).unwrap();
    let desktop = || ReleaseTargets {
        tested_on: vec!["mac".into()],
        tested_set_by: Some("lead".into()),
        deploys_to: vec!["imac".into(), "mac".into(), "win-pc".into()],
        deploys_set_by: None,
    };
    let package = |name: &str, platform: &str| {
        db.board_tx(|t| {
            let r = t.insert_release(&NewRelease {
                project_id: &p.id,
                name,
                display_version: None,
                changelog: "",
                how_to_test: &serde_json::json!([]),
                created_by: "ops",
                items: &[],
            })?;
            t.set_release_build(
                &r.id,
                &ReleaseBuild {
                    platform: platform.into(),
                    version: "1".into(),
                    artifact: "/b".into(),
                    url: None,
                    install_url: None,
                    sha256: "ab".into(),
                    built_at: bus::now(),
                    source_commit: None,
                },
            )?;
            t.set_release_targets(&r.id, &desktop())?;
            Ok(r.id)
        })
        .unwrap()
    };
    let ios = package("iOS 0.5.0", "ios");
    let mac = package("0.17.3", "desktop-mac");
    let deploying = package("iOS 0.4.0", "ios");
    db.board_tx(|t| t.start_deployment(&deploying, "mac", DeployAction::Deploy, "ops", None))
        .unwrap();
    let sql = bus::schema::MIGRATIONS
        .iter()
        .find(|m| m.contains("'iphone', 'deploy'"))
        .expect("the iOS deploy target migration");
    for _ in 0..2 {
        db.lock().execute_batch(sql).unwrap();
    }
    let deploys = |id: &str| {
        db.board_read(|t| t.release(id))
            .unwrap()
            .unwrap()
            .targets
            .deploys_to
    };
    assert_eq!(deploys(&ios), vec!["iphone".to_string()]);
    assert_eq!(deploys(&mac), desktop().deploys_to, "a desktop package");
    assert_eq!(
        deploys(&deploying),
        desktop().deploys_to,
        "already deploying"
    );
    let tested = db.board_read(|t| t.release(&ios)).unwrap().unwrap().targets;
    assert_eq!(
        tested.tested_on,
        vec!["mac".to_string()],
        "tests stay as frozen"
    );
}
