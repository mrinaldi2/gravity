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
    match method {
        LAND | BUILD => Ok(json!({
            "release_id": release.id,
            "name": release.name,
            "version": version_of(&release),
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
    if !approved(release.status) {
        return Err(forbidden(format!(
            "release {} is {}: only a release the owner approved goes on",
            release.name,
            release.status.as_str()
        )));
    }
    Ok(release)
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
