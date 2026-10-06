//! `install_quiesce` (H-117 Q4): the bot installing a release on this
//! computer pauses every project here first. Gated like `install_release`,
//! the owner-granted `quiesce` extra on top:
//! - the bot holds the `quiesce` extra (the owner grants it, once);
//! - it is the project's DevOps or holds the `install` extra;
//! - it holds an open deploy task for an approved release, or it is the
//!   project's DevOps and the release is being installed on this computer
//!   (H-166).
//!
//! It acts on this computer only. The caller is the pause's `exempt_bot`:
//! it keeps running until it has handed the install to the system
//! (ARCH-R49 M1). Where the board lives on another computer, the home
//! checks the deploy task and this computer pauses itself
//! (`mcp::board_remote`).

use std::sync::Arc;

use bus::{Bot, PermissionExtra};
use chrono::{Duration, Utc};
use serde_json::{json, Value};

use crate::app::AppState;
use crate::board::model::Role;
use crate::board::release::{deploy, quiesce_gate, Caller};
use crate::bot_permissions::Stored;
use crate::decisions::{forbidden, invalid};

use super::PauseRequest;

/// Why a bot may not pause every project, or `Ok` with the release's name
/// and the gate's answer (its builds).
fn gate(app: &Arc<AppState>, me: &Caller<'_>, release_id: &str) -> anyhow::Result<Value> {
    may_pause(app, me.bot)?;
    // An open deploy task for an approved release: install_release's gate.
    match deploy::install(app, me, release_id) {
        Err(_) if me.has(Role::Devops) => quiesce_gate::devops_here(app, me, release_id),
        answer => answer,
    }
}

/// Why this bot's own extras don't let it pause every project here.
pub fn may_pause(app: &AppState, bot: &Bot) -> anyhow::Result<()> {
    let stored = Stored::load(&app.db, bot)?;
    if !stored.extras.contains(&PermissionExtra::Quiesce) {
        return Err(forbidden(
            "pausing every project needs the \"Pause all projects for an install\" extra; \
             the owner grants it in your permissions",
        ));
    }
    if !stored.devops && !stored.extras.contains(&PermissionExtra::Install) {
        return Err(forbidden(
            "only the project's DevOps, or a bot with the install extra, pauses projects \
             for an install",
        ));
    }
    Ok(())
}

/// The version this computer will run: its build's, from the gate.
fn version_here(answer: &Value) -> Option<String> {
    let builds = crate::board::release::install::builds(answer);
    crate::board::release::install::for_this_computer(
        &builds,
        cfg!(target_os = "macos"),
        cfg!(windows),
    )
    .first()
    .map(|b| b.version.clone())
}

/// The open pause, when it is this bot's install of `release_id`.
fn mine(app: &AppState, bot: &Bot, release_id: &str) -> anyhow::Result<()> {
    match app.db.open_quiesce()? {
        Some(q)
            if q.release_id.as_deref() == Some(release_id)
                && q.exempt_bot.as_deref() == Some(bot.id.as_str()) =>
        {
            Ok(())
        }
        _ => Err(invalid(format!(
            "no pause is open for your install of {release_id}; start one first"
        ))),
    }
}

/// What a bot asked `install_quiesce` to do.
pub struct Ask<'a> {
    /// `start`, `status`, `extend`, `install_started` or `resume`.
    pub action: &'a str,
    pub release_id: &'a str,
    pub version: Option<&'a str>,
    pub binary_sha256: Option<&'a str>,
}

/// `install_quiesce` where this computer holds the release's board.
pub fn call(app: &Arc<AppState>, me: &Caller<'_>, ask: &Ask<'_>) -> anyhow::Result<Value> {
    run(app, me.bot, ask, &|| gate(app, me, ask.release_id))
}

/// Act on this computer's pause for `bot`, once `gate` has answered with
/// the release's name and builds: here, or from the board's home.
pub fn run(
    app: &Arc<AppState>,
    bot: &Bot,
    ask: &Ask<'_>,
    gate: &dyn Fn() -> anyhow::Result<Value>,
) -> anyhow::Result<Value> {
    let release_id = ask.release_id;
    match ask.action {
        "status" => super::status(app),
        "extend" => {
            gate()?;
            mine(app, bot, release_id)?;
            Ok(json!({ "quiesce": super::extend(app, Utc::now())? }))
        }
        "install_started" => {
            gate()?;
            mine(app, bot, release_id)?;
            let q = super::install_started(app, ask.binary_sha256, Utc::now())?;
            Ok(json!({ "quiesce": q }))
        }
        "resume" => {
            gate()?;
            let closed = super::resume_all(app, "aborted", Utc::now())?;
            Ok(json!({ "resumed": closed }))
        }
        "start" => {
            let answer = gate()?;
            let name = answer["name"].as_str().unwrap_or(release_id).to_string();
            let version = ask
                .version
                .map(str::to_string)
                .or_else(|| version_here(&answer));
            let reason = format!("install of {name}");
            let started_by = format!("bot:{}", bot.id);
            let req = PauseRequest {
                reason: &reason,
                release_id: Some(release_id),
                version: version.as_deref(),
                exempt_bot: Some(&bot.id),
                started_by: &started_by,
                deadline: Duration::minutes(app.cfg.quiesce.deadline_minutes.max(1)),
            };
            let (q, report) = super::start(app, &req, Utc::now())?;
            super::announce(app, &q, &report);
            Ok(json!({ "quiesce": q, "report": report, "proceed": q.phase == "ready" }))
        }
        other => Err(invalid(format!(
            "unknown action {other}; start, status, extend, install_started or resume"
        ))),
    }
}
