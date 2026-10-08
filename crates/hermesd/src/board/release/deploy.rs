//! Rolling an approved package out (H-020 §2.4–2.5): DevOps deploys it to a
//! machine, which opens a task to that machine's tester; the tester fetches
//! the builds with `install_release` and reports with `deploy_confirm`. The
//! gate is checked again at every deploy, never remembered.

use std::sync::Arc;

use bus::{DecisionState, MessageKind, Sender, SenderKind, TaskState, DEFAULT_TASK_DEADLINE_HOURS};
use serde_json::{json, Value};

use crate::app::AppState;
use crate::board::model::{ColumnCategory, Role};
use crate::db::BoardTx;
use crate::decisions::authority::is_relayed;
use crate::decisions::{conflict, forbidden, invalid};
use crate::messaging::{self, Dm};

use super::machines;
use super::model::{DeployAction, DeployResult, Release, ReleaseStatus, Smoke, Verdict};
use super::post_install::post_install_checked;
use super::{check_frozen, daemon_move, load, publish_moves, publish_touched, testers_on, Caller};

/// The gate, checked at call time (H-020 §2.4): a settled, unrelayed owner
/// or device ruling, an unchanged package, and only shipped items.
pub fn gate_open(app: &Arc<AppState>, release: &Release) -> anyhow::Result<()> {
    let shut = |why: String| {
        Err(forbidden(format!(
            "release {} can't deploy: {why}",
            release.name
        )))
    };
    if let Some(reason) = release
        .paused_reason
        .as_deref()
        .filter(|_| release.status == ReleaseStatus::Paused)
    {
        return shut(format!("the rollout is paused: {reason}"));
    }
    if !matches!(
        release.status,
        ReleaseStatus::Approved | ReleaseStatus::Deploying | ReleaseStatus::PartiallyDeployed
    ) {
        return shut(format!("it is {}, not approved", release.status.as_str()));
    }
    let Some(decision) = release
        .decision_id
        .as_deref()
        .map(|id| app.db.get_decision(id))
        .transpose()?
        .flatten()
    else {
        return shut("it has no owner decision".into());
    };
    if decision.state != DecisionState::Settled {
        return shut(format!("its decision is {}", decision.state.as_str()));
    }
    let by = decision
        .ruling
        .as_ref()
        .map(|r| r.answered_by.as_str())
        .unwrap_or("");
    if is_relayed(&decision) || !(by == "owner" || by.starts_with("device:")) {
        return shut("its ruling did not come from the owner directly".into());
    }
    check_frozen(release)?;
    if let Some(i) = release.items.iter().find(|i| i.verdict != Verdict::Ship) {
        return shut(format!("{} is not marked ship", i.item_id));
    }
    Ok(())
}

fn sender(bot: &bus::Bot) -> Sender {
    Sender {
        kind: SenderKind::Bot,
        bot_id: Some(bot.id.clone()),
        name: bot.name.clone(),
    }
}

/// A delegated task from `from` to `to`, as `send_message(kind=task)` opens one.
fn open_task(app: &Arc<AppState>, from: &bus::Bot, to: &str, body: &str) -> anyhow::Result<String> {
    let s = sender(from);
    let msg = messaging::send_dm(
        &app.db,
        &app.events,
        Dm::new(to, &s, MessageKind::Task, body),
    )?;
    let deadline = chrono::Utc::now() + chrono::Duration::hours(DEFAULT_TASK_DEADLINE_HOURS);
    let task = app
        .db
        .create_task(&msg.id, Some(&from.id), to, Some(deadline), 1, &from.id)?;
    Ok(task.id)
}

/// Send an approved package to one machine, through its tester.
pub fn deploy(
    app: &Arc<AppState>,
    me: &Caller<'_>,
    release_id: &str,
    machine: &str,
) -> anyhow::Result<Release> {
    me.require(Role::Devops, "deploy a release")?;
    roll(app, me, release_id, machine, DeployAction::Deploy)
}

/// Take a package back off one machine, through its tester.
pub fn rollback(
    app: &Arc<AppState>,
    me: &Caller<'_>,
    release_id: &str,
    machine: &str,
) -> anyhow::Result<Release> {
    me.require(Role::Devops, "roll a release back")?;
    roll(app, me, release_id, machine, DeployAction::Rollback)
}

fn roll(
    app: &Arc<AppState>,
    me: &Caller<'_>,
    release_id: &str,
    machine: &str,
    action: DeployAction,
) -> anyhow::Result<Release> {
    let project = me.bot.project_id.as_str();
    let release = app.db.board_read(|t| load(t, project, release_id))?;
    match action {
        DeployAction::Deploy => gate_open(app, &release)?,
        DeployAction::Rollback => {
            if !release.deployments.iter().any(|d| {
                machines::same_target(&d.machine, machine) && d.action == DeployAction::Deploy
            }) {
                return Err(conflict(format!(
                    "release {} was never deployed to {machine}",
                    release.name
                )));
            }
        }
    }
    let tester = testers_on(app, project, machine)?
        .into_iter()
        .next()
        .ok_or_else(|| {
            invalid(format!(
                "no tester is assigned to {machine}; the lead sets one"
            ))
        })?;
    let builds: Vec<String> = release
        .builds
        .iter()
        .map(|b| {
            format!(
                "- {} {}: {} (sha256 {})",
                b.platform, b.version, b.artifact, b.sha256
            )
        })
        .collect();
    let body = match action {
        DeployAction::Deploy => format!(
            "Deploy release {} to {machine} (release_id {}). The owner approved it. Call \
             install_release with the release_id for the verified builds, install them, restart and \
             smoke-check, then report with deploy_confirm (release_id, machine, result, smoke).\n{}",
            release.name, release.id, builds.join("\n")
        ),
        DeployAction::Rollback => format!(
            "Roll release {} back on {machine} (release_id {}){}. Reinstall the previous version, \
             restart and smoke-check, then report with deploy_confirm and result rolled_back.",
            release.name,
            release.id,
            release.rollback_to.as_deref().map(|r| format!(" to release {r}")).unwrap_or_default()
        ),
    };
    let task_id = open_task(app, me.bot, &tester, &body)?;
    app.db.set_task_release(&task_id, &release.id)?;
    app.db.board_tx(|t| {
        t.start_deployment(&release.id, machine, action, &tester, Some(&task_id))?;
        if action == DeployAction::Deploy && release.status == ReleaseStatus::Approved {
            t.set_release_status(&release.id, ReleaseStatus::Deploying)?;
        }
        Ok(t.release(&release.id)?.expect("loaded"))
    })
}

/// A tester holding the deploy task for this release gets its verified builds.
pub fn install(app: &Arc<AppState>, me: &Caller<'_>, release_id: &str) -> anyhow::Result<Value> {
    let release = app
        .db
        .board_read(|t| load(t, &me.bot.project_id, release_id))?;
    let mine = release.deployments.iter().find(|d| {
        d.executor == me.bot.id
            && d.result.is_none()
            && d.task_id
                .as_deref()
                .and_then(|id| app.db.get_task(id).ok().flatten())
                .is_some_and(|task| task.state == TaskState::Open)
    });
    let Some(deployment) = mine else {
        return Err(forbidden(format!(
            "you hold no open deploy task for release {}; install only when DevOps tasks you with it",
            release.name
        )));
    };
    if deployment.action == DeployAction::Deploy {
        gate_open(app, &release)?;
    }
    let verified = super::publish::verify_served(app, &release)?;
    Ok(json!({
        "release_id": release.id,
        "name": release.name,
        "machine": deployment.machine,
        "action": deployment.action.as_str(),
        "install_mode": release.install_mode,
        "builds": release.builds.iter().zip(verified).map(|(b, verified)| json!({
            "platform": b.platform, "version": b.version, "artifact": b.artifact,
            "url": b.url, "install_url": b.install_url, "sha256": b.sha256,
            "verified": verified,
        })).collect::<Vec<_>>(),
    }))
}

/// How a machine's deployment or rollback went. Every required machine ok
/// closes the items; a failure sends them back to Verify and asks DevOps to
/// roll back.
pub fn confirm(
    app: &Arc<AppState>,
    me: &Caller<'_>,
    release_id: &str,
    machine: &str,
    result: DeployResult,
    smoke: Option<Smoke>,
    log_artifact: Option<&str>,
) -> anyhow::Result<Release> {
    let project = me.bot.project_id.as_str();
    let actor = me.actor();
    let mut feed = app.board.writer();
    let (release, moved, touched, failed) = app.db.board_tx(|t| {
        let release = load(t, project, release_id)?;
        let action = if result == DeployResult::RolledBack {
            DeployAction::Rollback
        } else {
            DeployAction::Deploy
        };
        let row = release
            .deployments
            .iter()
            .find(|d| machines::same_target(&d.machine, machine) && d.action == action);
        let Some(row) = row else {
            return Err(conflict(format!(
                "release {} has no {} open on {machine}",
                release.name,
                action.as_str()
            )));
        };
        if row.executor != me.bot.id && !me.has(Role::Devops) {
            return Err(forbidden(
                "only the tester carrying it out or DevOps can confirm it",
            ));
        }
        let machine = row.machine.clone();
        // Called off when a later package closed this one (H-191).
        if row.result == Some(DeployResult::Superseded) {
            return Err(conflict(format!(
                "the {} on {machine} was called off: release {} was closed through a later one",
                action.as_str(),
                release.name
            )));
        }
        t.finish_deployment(&release.id, &machine, action, result, smoke, log_artifact)?;
        let release = t.release(&release.id)?.expect("loaded");
        let (status, to, note) = match result {
            DeployResult::Failed => (
                ReleaseStatus::PartiallyDeployed,
                Some(ColumnCategory::Verify),
                format!("deploy of release {} failed on {machine}", release.name),
            ),
            DeployResult::Ok if all_done(t, &release)? => (
                ReleaseStatus::Deployed,
                Some(ColumnCategory::Done),
                format!("release {} deployed", release.name),
            ),
            DeployResult::RolledBack if all_rolled_back(&release) => {
                (ReleaseStatus::RolledBack, None, String::new())
            }
            _ => (release.status, None, String::new()),
        };
        if to == Some(ColumnCategory::Done) {
            post_install_checked(t, &release)?;
        }
        let (mut moved, mut touched) = (Vec::new(), Vec::new());
        if let Some(to) = to {
            for ri in &release.items {
                let from = daemon_move(t, project, &ri.item_id, to, &note, false, &actor)?;
                moved.extend(from.map(|f| (ri.item_id.clone(), f)));
                // Back in Verify, an item is free to go into a fixed package
                // (ARCH-R23 F3).
                if to == ColumnCategory::Verify && t.set_item_release(&ri.item_id, None, &actor)? {
                    touched.push(ri.item_id.clone());
                }
            }
        }
        if status != release.status {
            t.set_release_status(&release.id, status)?;
        }
        Ok((
            t.release(&release.id)?.expect("loaded"),
            moved,
            touched,
            result == DeployResult::Failed,
        ))
    })?;
    publish_moves(app, &mut feed, project, &moved);
    publish_touched(app, &mut feed, project, &touched, &moved);
    drop(feed);
    if failed && !me.has(Role::Devops) {
        ask_rollback(app, me, &release, machine)?;
    }
    // Deployed everywhere: older packages it replaces close (H-191).
    if release.status == ReleaseStatus::Deployed {
        super::supersede::after_deploy(app, me.bot, &actor, &release);
    }
    Ok(release)
}

/// Every machine the items' platforms require (or, with none configured,
/// every machine it went to) has reported a good deploy.
pub(super) fn all_done(t: &BoardTx<'_>, release: &Release) -> anyhow::Result<bool> {
    // Every tester's computer unless the owner narrowed it, as frozen at
    // submit (ARCH-R55 M1). A rolled-back install doesn't count (H-191 S2).
    let required = machines::deploys_to(t, release)?;
    if required.is_empty() {
        // No machine is configured or has a tester: every machine it went to.
        let mut deploys = release
            .deployments
            .iter()
            .filter(|d| d.action == DeployAction::Deploy)
            .peekable();
        return Ok(deploys.peek().is_some()
            && deploys.all(|d| machines::installed_on(release, &d.machine)));
    }
    Ok(required.iter().all(|m| machines::installed_on(release, m)))
}

fn all_rolled_back(release: &Release) -> bool {
    release
        .deployments
        .iter()
        .filter(|d| d.action == DeployAction::Deploy && d.result == Some(DeployResult::Ok))
        .all(|d| {
            release.deployments.iter().any(|r| {
                machines::same_target(&r.machine, &d.machine)
                    && r.action == DeployAction::Rollback
                    && r.result == Some(DeployResult::RolledBack)
            })
        })
}

/// A failed deploy becomes a rollback task to DevOps, from the tester.
fn ask_rollback(
    app: &Arc<AppState>,
    me: &Caller<'_>,
    release: &Release,
    machine: &str,
) -> anyhow::Result<()> {
    let devops = app
        .db
        .project_roles(&release.project_id)?
        .into_iter()
        .find(|r| r.role == Role::Devops)
        .map(|r| r.bot_id);
    if let Some(devops) = devops.filter(|d| *d != me.bot.id) {
        let body = format!(
            "Release {} failed on {machine}; its items are back in Verify. Roll it back with \
             release_rollback (release_id {}, machine {machine}).",
            release.name, release.id
        );
        let task_id = open_task(app, me.bot, &devops, &body)?;
        app.db.set_task_release(&task_id, &release.id)?;
    }
    Ok(())
}
