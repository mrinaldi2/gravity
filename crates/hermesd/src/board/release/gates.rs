//! The daemon's side of `hermesd release land` (H-117 X2) and
//! `hermesd release build-installer` (X3). The command runs in the bot's
//! session and asks over the local endpoint, which knows the bot by its
//! process, so the daemon checks the extra, the role and the release before
//! the command touches anything, and records what it did on the release.
//!
//! - `hermes/release_land {release_id}`: the bot holds `release_main` and is
//!   the project's DevOps; the owner approved the release.
//! - `hermes/release_landed {release_id, commit, tag}`: recorded.
//! - `hermes/release_build_installer {release_id}`: the bot holds
//!   `build_installers`; the owner approved the release.
//! - `hermes/release_installer_built {release_id, commit, file, sha256}`:
//!   recorded, for `release publish`.

use std::sync::Arc;

use bus::PermissionExtra;
use chrono::Utc;
use serde_json::{json, Value};

use crate::app::AppState;
use crate::board::model::Role;
use crate::bot_permissions::Stored;
use crate::decisions::{forbidden, invalid};

use super::model::{Release, ReleaseEvent, ReleaseStatus};
use super::{load, Caller};

const LAND: &str = "hermes/release_land";
const LANDED: &str = "hermes/release_landed";
const BUILD: &str = "hermes/release_build_installer";
const BUILT: &str = "hermes/release_installer_built";

/// Answers one of the release gates from `bot_id`'s session, or `None` for
/// any other request.
pub fn serve(app: &Arc<AppState>, bot_id: &str, request: &Value) -> Option<Value> {
    let method = request.get("method").and_then(Value::as_str)?;
    if ![LAND, LANDED, BUILD, BUILT].contains(&method) {
        return None;
    }
    let id = request.get("id").cloned().unwrap_or(Value::Null);
    let params = request.get("params").cloned().unwrap_or(json!({}));
    let answer = answer(app, bot_id, method, &params);
    Some(match answer {
        Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
        Err(e) => json!({ "jsonrpc": "2.0", "id": id,
                          "error": { "code": -32001, "message": e.to_string() } }),
    })
}

fn text<'a>(params: &'a Value, key: &str) -> anyhow::Result<&'a str> {
    params
        .get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| invalid(format!("'{key}' is required")))
}

fn answer(
    app: &Arc<AppState>,
    bot_id: &str,
    method: &str,
    params: &Value,
) -> anyhow::Result<Value> {
    let bot = app
        .db
        .get_live_bot(bot_id)?
        .ok_or_else(|| forbidden("no such bot"))?;
    let release_id = text(params, "release_id")?;
    let extra = if matches!(method, LAND | LANDED) {
        PermissionExtra::ReleaseMain
    } else {
        PermissionExtra::BuildInstallers
    };
    let release = gate(app, &bot, release_id, extra)?;
    // The commit the owner approved: what the builds were made from.
    let recorded = recorded_commit(&release)?;
    let commit = if extra == PermissionExtra::ReleaseMain {
        Some(recorded.ok_or_else(|| {
            forbidden(format!(
                "release {}'s builds don't all record the commit they were built from, so \
                 nothing can say what the owner approved; republish them with --source-commit",
                release.name
            ))
        })?)
    } else {
        recorded
    };
    if let (Some(sent), Some(commit)) = (params.get("commit").and_then(Value::as_str), &commit) {
        if sent != commit {
            return Err(forbidden(format!(
                "{sent} isn't the commit release {} was built from ({commit})",
                release.name
            )));
        }
    }
    match method {
        LAND | BUILD => Ok(json!({
            "release_id": release.id,
            "name": release.name,
            "version": version_of(&release),
            "commit": commit,
            // Where `land` checks its result: the project's repo, as the
            // owner configured it, not the checkout's own remote.
            "repo_url": app.db.project_repo(&bot.project_id)?.map(|r| r.url),
        })),
        LANDED => {
            let detail = json!({ "commit": text(params, "commit")?, "tag": text(params, "tag")? });
            record(
                app,
                &release,
                "landed",
                &bot,
                "landed on main and tagged",
                detail,
            )
        }
        _ => {
            let detail = json!({
                "commit": text(params, "commit")?,
                "file": text(params, "file")?,
                "sha256": text(params, "sha256")?,
            });
            record(
                app,
                &release,
                "installer_built",
                &bot,
                "Windows installer built",
                detail,
            )
        }
    }
}

/// The version a release ships: its display version, else its name.
fn version_of(release: &Release) -> String {
    release
        .display_version
        .clone()
        .unwrap_or_else(|| release.name.clone())
        .trim_start_matches('v')
        .to_string()
}

/// Who may, for which release: the extra, DevOps for landing, and a
/// release the owner approved.
fn gate(
    app: &Arc<AppState>,
    bot: &bus::Bot,
    release_id: &str,
    extra: PermissionExtra,
) -> anyhow::Result<Release> {
    // Landing pushes to main and building ships installers: never from a
    // scratch daemon (H-171).
    if app.cfg.scratch {
        return Err(forbidden("a scratch daemon lands and builds nothing"));
    }
    let stored = Stored::load(&app.db, bot)?;
    if !stored.extras.contains(&extra) {
        return Err(forbidden(format!(
            "this needs the {} extra; the owner grants it in your permissions",
            extra.as_str()
        )));
    }
    if extra == PermissionExtra::ReleaseMain {
        let me = Caller::load(app, bot)?;
        if !me.roles.contains(&Role::Devops) {
            return Err(forbidden(
                "only the project's DevOps lands a release on main",
            ));
        }
    }
    let release = app
        .db
        .board_read(|t| load(t, &bot.project_id, release_id))
        .map_err(|_| invalid(format!("no release {release_id} on this computer's board")))?;
    // A release cut from main is already on main: it is tagged (H-272).
    if extra == PermissionExtra::ReleaseMain && release.cut.is_some() {
        return Err(forbidden(format!(
            "release {} was cut from main: tag it with `hermesd release tag`",
            release.name
        )));
    }
    // Landing takes the owner's approval. The installer is a build of the
    // package, made while it is still open for builds, before the owner
    // rules on it.
    if extra == PermissionExtra::ReleaseMain && !approved(release.status) {
        return Err(forbidden(format!(
            "release {} is {}: only a release the owner approved goes on",
            release.name,
            release.status.as_str()
        )));
    }
    if extra == PermissionExtra::BuildInstallers {
        super::plan::builds_wait(&release)?;
    }
    if extra == PermissionExtra::BuildInstallers && !release.status.is_assembling() {
        return Err(forbidden(format!(
            "release {} is {}: its builds were frozen when it was submitted",
            release.name,
            release.status.as_str()
        )));
    }
    Ok(release)
}

/// The one commit a release's builds (and its installer builds) were made
/// from, `None` when no build says yet (ARCH-R52 M1). Refused when they
/// disagree, and, once there are builds, when any build doesn't say.
pub(crate) fn recorded_commit(release: &Release) -> anyhow::Result<Option<String>> {
    let mut commits = std::collections::BTreeSet::new();
    let mut unnamed = Vec::new();
    for b in &release.builds {
        match &b.source_commit {
            Some(c) => {
                commits.insert(c.clone());
            }
            None => unnamed.push(b.platform.clone()),
        }
    }
    for e in release
        .events
        .iter()
        .filter(|e| e.kind == "installer_built")
    {
        if let Some(c) = e.detail["commit"].as_str() {
            commits.insert(c.to_string());
        }
    }
    if commits.len() > 1 {
        return Err(forbidden(format!(
            "release {}'s builds come from different commits ({}); one release is one commit",
            release.name,
            commits.into_iter().collect::<Vec<_>>().join(", ")
        )));
    }
    if !unnamed.is_empty() && !commits.is_empty() {
        return Err(forbidden(format!(
            "release {}'s {} build records no source commit; republish it with --source-commit",
            release.name,
            unnamed.join(", ")
        )));
    }
    if !unnamed.is_empty() {
        return Ok(None);
    }
    Ok(commits.into_iter().next())
}

/// Approved by the owner, and not stopped since.
pub(crate) fn approved(status: ReleaseStatus) -> bool {
    matches!(
        status,
        ReleaseStatus::Approved
            | ReleaseStatus::Deploying
            | ReleaseStatus::PartiallyDeployed
            | ReleaseStatus::Deployed
    )
}

fn record(
    app: &AppState,
    release: &Release,
    kind: &str,
    bot: &bus::Bot,
    note: &str,
    detail: Value,
) -> anyhow::Result<Value> {
    let event = ReleaseEvent {
        release_id: release.id.clone(),
        release_name: release.name.clone(),
        related_id: None,
        kind: kind.to_string(),
        actor: format!("bot:{}", bot.id),
        note: Some(note.to_string()),
        detail: detail.clone(),
        at: Utc::now(),
    };
    app.db
        .board_tx(|t| t.record_release_event(&event, &release.project_id))?;
    Ok(json!({ "recorded": kind, "detail": detail }))
}
