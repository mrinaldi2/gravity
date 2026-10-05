//! Quiesce for an install (H-117 Q2): every project on this computer is held
//! still (bots, routines, workers, deliveries) while the daemon is replaced,
//! and resumed after. The pause is a row in the database, so it outlasts the
//! install's daemon restart. While a row is open:
//! - the supervision tick stops every session and starts none
//!   (`Supervisor::hold`, from `server`);
//! - the scheduler records due routine slots but runs nothing; on resume
//!   each routine keeps only its latest slot, so a pause fires a routine at
//!   most once, never a backlog;
//! - the delivery worker leaves messages queued, local and peer alike;
//! - the worker queue places nothing; a worker is a bot, so it is held like
//!   one and comes back, task still open, when the pause ends.
//!
//! Resuming extends open tasks' deadlines by the time paused. A pause left
//! open past its deadline (30 minutes by default) resumes by itself and
//! says so: the dead-man switch.

use std::sync::Arc;

use chrono::{DateTime, Duration, Utc};
use serde_json::{json, Value};

use crate::app::AppState;
use crate::board::model::Role;
use crate::db::{NewQuiesce, Quiesce};
use crate::decisions::conflict;
use crate::events::Push;

pub mod cli;
pub mod fallback;
mod outcome;
pub mod reap;
pub mod services;
mod start;
pub mod tool;

pub use outcome::{announce, boot_outcome, file_sha256, on_boot, INSTALL_STARTED};
pub use start::start;

/// How long a pause may stay open before it resumes by itself.
pub const DEFAULT_DEADLINE_MINUTES: i64 = 30;
/// How often the dead-man switch looks.
const DEADMAN_SECS: u64 = 15;

/// What a bot shows while held.
/// `Paused (install of 0.17.0)`: an install's reason names its release.
pub fn reason_line(q: &Quiesce) -> String {
    format!("Paused ({})", q.reason)
}

/// Who asked for the pause and why.
pub struct PauseRequest<'a> {
    pub reason: &'a str,
    pub release_id: Option<&'a str>,
    /// The version being installed (Q4's boot check).
    pub version: Option<&'a str>,
    /// The bot running the install, left running until it has handed the
    /// install to the system (ARCH-R49 M1).
    pub exempt_bot: Option<&'a str>,
    pub started_by: &'a str,
    pub deadline: Duration,
}

fn changed(app: &AppState) {
    let quiesce = app.db.open_quiesce().ok().flatten();
    app.events.push(Push::QuiesceUpdate {
        quiesce: quiesce.map(Box::new),
    });
}

/// Pauses every project on this computer. Refused while a pause is open.
/// Returns the pause with its report: `{paused: {bots, routines, workers,
/// queued}}`.
pub fn pause_all(
    app: &Arc<AppState>,
    req: &PauseRequest<'_>,
    now: DateTime<Utc>,
) -> anyhow::Result<Quiesce> {
    let new = NewQuiesce {
        reason: req.reason,
        release_id: req.release_id,
        version: req.version,
        exempt_bot: req.exempt_bot,
        started_by: req.started_by,
        now,
        deadline: req.deadline,
    };
    let Some(q) = app.db.open_new_quiesce(&new)? else {
        let open = app.db.open_quiesce()?;
        return Err(conflict(match open {
            Some(o) => format!(
                "this computer is already paused since {} by {} ({})",
                o.started_at.to_rfc3339(),
                o.started_by,
                o.reason
            ),
            None => "this computer is already paused".to_string(),
        }));
    };
    let bots = app
        .supervisor
        .hold(&reason_line(&q), q.exempt_bot.as_deref());
    let routines = app
        .db
        .list_routines(None)?
        .iter()
        .filter(|r| r.enabled)
        .count();
    let report = json!({
        "paused": {
            "bots": bots,
            "routines": routines,
            "workers": app.db.running_workers()?.len(),
            "queued": app.db.queued_delivery_count()?,
        },
    });
    app.db.set_quiesce_phase(&q.id, "paused", &report)?;
    tracing::warn!(reason = %q.reason, release = ?q.release_id, "every project paused");
    changed(app);
    Ok(app.db.get_quiesce(&q.id)?.unwrap_or(q))
}

/// Resumes the open pause, if any, as `outcome` (`resumed`, `install_ok`,
/// `rolled_back`, `deadline`). Services the pause stopped start again.
/// Returns the closed pause.
pub fn resume_all(
    app: &Arc<AppState>,
    outcome: &str,
    now: DateTime<Utc>,
) -> anyhow::Result<Option<Quiesce>> {
    let Some(q) = app.db.open_quiesce()? else {
        return Ok(None);
    };
    let paused_for = (now - q.started_at).max(Duration::zero());
    let coalesced = app.db.coalesce_scheduled_runs(now)?;
    let extended = app.db.extend_open_task_deadlines(paused_for)?;
    let restarted = services::restart_stopped(app, &q);
    let mut report = q.report.clone();
    if !report.is_object() {
        report = json!({});
    }
    report["resumed"] = json!({
        "at": now,
        "outcome": outcome,
        "paused_seconds": paused_for.num_seconds(),
        "coalesced_runs": coalesced,
        "extended_tasks": extended,
        "services_started": restarted,
    });
    if !app.db.close_quiesce(&q.id, outcome, &report, now)? {
        return Ok(None);
    }
    tracing::warn!(outcome, "every project resumed");
    app.supervisor.reconcile();
    app.workers.nudge();
    changed(app);
    app.db.get_quiesce(&q.id)
}

/// The dead-man switch: an open pause past its deadline resumes, and the
/// owner and each project's DevOps and lead are told. True when it fired.
pub fn check_deadline(app: &Arc<AppState>, now: DateTime<Utc>) -> anyhow::Result<bool> {
    let Some(q) = app.db.open_quiesce()? else {
        return Ok(false);
    };
    if now < q.deadline_at {
        return Ok(false);
    }
    resume_all(app, "deadline", now)?;
    let what = reason_line(&q);
    let body = format!(
        "The pause for an install ({}) passed its deadline without the install \
         finishing, so every project resumed by itself. Check the install and \
         its log before trying again.",
        what.trim_start_matches("Paused (").trim_end_matches(')')
    );
    app.events.push(Push::notice(
        "warn",
        "Projects resumed after a stuck install",
        &body,
    ));
    tell_roles(app, &[Role::Devops, Role::Lead], &body);
    Ok(true)
}

/// A note to every bot holding one of `roles`, in every project here.
pub fn tell_roles(app: &AppState, roles: &[Role], body: &str) {
    let sender = crate::messaging::daemon_sender();
    let Ok(projects) = app.db.list_projects() else {
        return;
    };
    let mut told = std::collections::HashSet::new();
    for project in projects {
        let Ok(held) = app.db.project_roles(&project.id) else {
            continue;
        };
        for role in held.into_iter().filter(|r| roles.contains(&r.role)) {
            if !told.insert(role.bot_id.clone()) {
                continue;
            }
            let dm = crate::messaging::Dm::new(&role.bot_id, &sender, bus::MessageKind::Note, body);
            if let Err(e) = crate::messaging::send_dm(&app.db, &app.events, dm) {
                tracing::warn!(bot_id = %role.bot_id, error = %e, "quiesce note failed");
            }
        }
    }
}

/// Holds the bots still while a pause is open; true while it is. Called
/// each supervision tick in place of starting bots.
pub fn hold(app: &AppState) -> bool {
    match app.db.open_quiesce() {
        Ok(Some(q)) => {
            app.supervisor
                .hold(&reason_line(&q), q.exempt_bot.as_deref());
            true
        }
        Ok(None) => false,
        Err(e) => {
            tracing::warn!(error = %e, "could not read the quiesce state");
            false
        }
    }
}

/// Runs the dead-man switch for as long as the daemon runs.
pub fn spawn_deadman(app: Arc<AppState>) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(std::time::Duration::from_secs(DEADMAN_SECS));
        loop {
            tick.tick().await;
            if let Err(e) = check_deadline(&app, Utc::now()) {
                tracing::warn!(error = %e, "quiesce deadline check failed");
            }
        }
    });
}

/// The open pause as clients read it: `{quiesce: …|null}`.
pub fn status(app: &AppState) -> anyhow::Result<Value> {
    Ok(json!({ "quiesce": app.db.open_quiesce()? }))
}

fn open(app: &AppState) -> anyhow::Result<Quiesce> {
    app.db
        .open_quiesce()?
        .ok_or_else(|| crate::decisions::not_found("no pause is open"))
}

/// Each install step pushes the open pause's deadline a full window on, so
/// the dead-man switch can't resume projects mid-install (ARCH-R50 S2).
pub fn extend(app: &AppState, now: DateTime<Utc>) -> anyhow::Result<Quiesce> {
    let q = open(app)?;
    let at = now + Duration::minutes(app.cfg.quiesce.deadline_minutes.max(1));
    app.db.set_quiesce_deadline(&q.id, at.max(q.deadline_at))?;
    changed(app);
    open(app)
}

/// Right before the system's install job stops the daemon: the pause is in
/// `install_started`, the phase a booting daemon ends it from, with the
/// hash of the daemon binary being installed when known (ARCH-R50 S1).
pub fn install_started(
    app: &AppState,
    binary_sha256: Option<&str>,
    now: DateTime<Utc>,
) -> anyhow::Result<Quiesce> {
    let q = open(app)?;
    let mut report = if q.report.is_object() {
        q.report.clone()
    } else {
        json!({})
    };
    report["install"] = json!({ "started_at": now, "binary_sha256": binary_sha256 });
    app.db
        .set_quiesce_phase(&q.id, outcome::INSTALL_STARTED, &report)?;
    extend(app, now)
}
