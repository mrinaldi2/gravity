//! A boot repair for iOS packages frozen before per-platform targets
//! (H-176). Those froze the desktop testers' computers as deploy targets,
//! which an iPhone build never reaches, so they could never close (iOS
//! 0.5.0, package 0eff48ae). A migration can't fix them: the targets are
//! part of the frozen hash, so `check_frozen` would refuse the deploy.
//!
//! And a boot settle (H-231): `ios` and `iphone` are one target now, so an
//! iOS package whose deploy on `ios` was confirmed while it waited for
//! `iphone` (iOS 0.6.1, 10835d40) is deployed. No new confirm would come to
//! say so.

use std::sync::Arc;

use serde_json::json;

use crate::actor::Actor;
use crate::app::AppState;
use crate::board::model::ColumnCategory;
use crate::db::Db;

use super::deploy::{all_done, post_install_checked};
use super::machines::{is_ios_package, is_ios_target, IOS_DEVICE};
use super::model::{
    DeployAction, DeployResult, Release, ReleaseEvent, ReleaseStatus, ReleaseTargets,
};
use super::{check_frozen, daemon_move, frozen_hash, publish_moves};

/// Whether `release` is one to repair: an iOS package submitted, not over,
/// not deploying yet, still as frozen, whose deploy targets nobody set and
/// aren't the iOS target already.
pub(super) fn needs_repair(release: &Release) -> bool {
    let targets = &release.targets;
    is_ios_package(release)
        && !release.status.is_unsubmitted()
        && !release.status.is_closed()
        && release.deployments.is_empty()
        && targets.deploys_set_by.is_none()
        && !targets.deploys_to.is_empty()
        && !targets.deploys_to.iter().all(|m| is_ios_target(m))
        && check_frozen(release).is_ok()
}

/// Marks deployed each iOS package still deploying whose targets are all
/// reached now that `ios` counts for `iphone`, its items to Done, under the
/// tester who confirmed it, with a `targets_settled` event. Leaves one whose
/// post-install criteria aren't ticked. Returns the ids settled.
pub fn settle_ios_deploys(app: &Arc<AppState>) -> anyhow::Result<Vec<String>> {
    let mut settled = Vec::new();
    for project in app.db.list_projects()? {
        let project = project.id.as_str();
        let mut feed = app.board.writer();
        let (ids, moved) = app.db.board_tx(|t| {
            let (mut ids, mut moved) = (Vec::new(), Vec::new());
            for release in t.releases(project)? {
                let deploying = matches!(
                    release.status,
                    ReleaseStatus::Deploying | ReleaseStatus::PartiallyDeployed
                );
                if !is_ios_package(&release) || !deploying || !all_done(t, &release)? {
                    continue;
                }
                if post_install_checked(t, &release).is_err() {
                    continue;
                }
                let executor = release
                    .deployments
                    .iter()
                    .find(|d| {
                        d.action == DeployAction::Deploy && d.result == Some(DeployResult::Ok)
                    })
                    .map(|d| d.executor.clone())
                    .unwrap_or_default();
                let actor = Actor::Bot {
                    id: &executor,
                    project_id: project,
                };
                let event = ReleaseEvent {
                    release_id: release.id.clone(),
                    release_name: release.name.clone(),
                    related_id: None,
                    kind: "targets_settled".into(),
                    actor: "daemon".into(),
                    note: Some("its deploy on ios counts for the iPhone (H-231)".into()),
                    detail: json!({ "deploys_to": release.targets.deploys_to }),
                    at: bus::now(),
                };
                t.record_release_event(&event, project)?;
                t.set_release_status(&release.id, ReleaseStatus::Deployed)?;
                let note = format!("release {} deployed", release.name);
                for ri in &release.items {
                    let from = daemon_move(
                        t,
                        project,
                        &ri.item_id,
                        ColumnCategory::Done,
                        &note,
                        false,
                        &actor,
                    )?;
                    moved.extend(from.map(|f| (ri.item_id.clone(), f)));
                }
                tracing::info!(release_id = %release.id, name = %release.name,
                    "an iOS package deployed on ios now counts as deployed (H-231)");
                ids.push(release.id);
            }
            Ok((ids, moved))
        })?;
        publish_moves(app, &mut feed, project, &moved);
        settled.extend(ids);
    }
    Ok(settled)
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
