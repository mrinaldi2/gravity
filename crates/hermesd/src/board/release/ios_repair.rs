//! A boot repair for iOS packages frozen before per-platform targets
//! (H-176). Those froze the desktop testers' computers as deploy targets,
//! which an iPhone build never reaches, so they could never close (iOS
//! 0.5.0, package 0eff48ae). A migration can't fix them: the targets are
//! part of the frozen hash, so `check_frozen` would refuse the deploy.

use serde_json::json;

use crate::db::Db;

use super::machines::{is_ios_package, IOS_DEVICE};
use super::model::{Release, ReleaseEvent, ReleaseTargets};
use super::{check_frozen, frozen_hash};

/// Whether `release` is one to repair: an iOS package submitted, not over,
/// not deploying yet, still as frozen, whose deploy targets nobody set and
/// aren't the iPhone.
pub(super) fn needs_repair(release: &Release) -> bool {
    let targets = &release.targets;
    is_ios_package(release)
        && !release.status.is_unsubmitted()
        && !release.status.is_closed()
        && release.deployments.is_empty()
        && targets.deploys_set_by.is_none()
        && !targets.deploys_to.is_empty()
        && targets.deploys_to != [IOS_DEVICE]
        && check_frozen(release).is_ok()
}

/// Re-freezes each such package to deploy to the owner's iPhone, its tests
/// as they were, and rehashes it so its deploy passes `check_frozen`. Each
/// repair is a `targets_repaired` event on the package, by the daemon, with
/// the targets it had and has; once repaired, a package no longer matches, so a
/// later boot does nothing. Returns the ids repaired.
pub fn repair_ios_deploy_targets(db: &Db) -> anyhow::Result<Vec<String>> {
    let projects = db.list_projects()?;
    db.board_tx(|t| {
        let mut repaired = Vec::new();
        for project in &projects {
            for release in t.releases(&project.id)? {
                if !needs_repair(&release) {
                    continue;
                }
                let targets = ReleaseTargets {
                    deploys_to: vec![IOS_DEVICE.to_string()],
                    deploys_set_by: None,
                    ..release.targets.clone()
                };
                t.set_release_targets(&release.id, &targets)?;
                let fixed = t.release(&release.id)?.expect("just updated");
                t.refreeze_release(&release.id, &frozen_hash(&fixed))?;
                let event = ReleaseEvent {
                    release_id: release.id.clone(),
                    release_name: release.name.clone(),
                    related_id: None,
                    kind: "targets_repaired".into(),
                    actor: "daemon".into(),
                    note: Some("frozen before per-platform targets (H-176)".into()),
                    detail: json!({
                        "from": release.targets.deploys_to,
                        "to": targets.deploys_to,
                    }),
                    at: bus::now(),
                };
                t.record_release_event(&event, &project.id)?;
                tracing::info!(
                    release_id = %release.id,
                    name = %release.name,
                    from = ?release.targets.deploys_to,
                    "an iOS package now deploys to the iPhone (H-176)"
                );
                repaired.push(release.id);
            }
        }
        Ok(repaired)
    })
}
