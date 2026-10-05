//! `install_quiesce` (H-117 Q4): the bot installing a release on this
//! computer pauses every project here first. Gated like `install_release`,
//! the owner-granted `quiesce` extra on top:
//! - the bot holds the `quiesce` extra (the owner grants it, once);
//! - it is the project's DevOps or holds the `install` extra;
//! - it holds an open deploy task for an approved release.
//!
//! It acts on this computer only. The caller is the pause's `exempt_bot`:
//! it keeps running until it has handed the install to the system
//! (ARCH-R49 M1).

use std::sync::Arc;

use bus::PermissionExtra;
use chrono::{Duration, Utc};
use serde_json::{json, Value};

use crate::app::AppState;
use crate::board::release::{deploy, Caller};
use crate::bot_permissions::Stored;
use crate::decisions::{forbidden, invalid};

use super::PauseRequest;

/// Why a bot may not pause every project, or `Ok` with the release's name
/// and the gate's answer (its builds).
fn gate(app: &Arc<AppState>, me: &Caller<'_>, release_id: &str) -> anyhow::Result<Value> {
    let stored = Stored::load(&app.db, me.bot)?;
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
    // An open deploy task for an approved release: install_release's gate.
    deploy::install(app, me, release_id)
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

/// `action`: `start`, `status` or `resume`.
pub fn call(
    app: &Arc<AppState>,
    me: &Caller<'_>,
    action: &str,
    release_id: &str,
    version: Option<&str>,
) -> anyhow::Result<Value> {
    match action {
        "status" => super::status(app),
        "resume" => {
            gate(app, me, release_id)?;
            let closed = super::resume_all(app, "aborted", Utc::now())?;
            Ok(json!({ "resumed": closed }))
        }
        "start" => {
            let answer = gate(app, me, release_id)?;
            let name = answer["name"].as_str().unwrap_or(release_id).to_string();
            let version = version
                .map(str::to_string)
                .or_else(|| version_here(&answer));
            let reason = format!("install of {name}");
            let started_by = format!("bot:{}", me.bot.id);
            let req = PauseRequest {
                reason: &reason,
                release_id: Some(release_id),
                version: version.as_deref(),
                exempt_bot: Some(&me.bot.id),
                started_by: &started_by,
                deadline: Duration::minutes(app.cfg.quiesce.deadline_minutes.max(1)),
            };
            let (q, report) = super::start(app, &req, Utc::now())?;
            super::announce(app, &q, &report);
            Ok(json!({ "quiesce": q, "report": report, "proceed": q.phase == "ready" }))
        }
        other => Err(invalid(format!(
            "unknown action {other}; start, status or resume"
        ))),
    }
}
