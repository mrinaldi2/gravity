//! `quiesce start` (H-117 Q2 + Q3): pause every project, reap what their
//! sessions left running, stop the services holding the home, and report
//! what still holds it. The install goes ahead only when nothing does.
//!
//! The installing bot (`exempt_bot`) is left alone, its processes included,
//! so the install it is running can hand off (ARCH-R49 M1).

use std::collections::HashSet;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use serde_json::{json, Value};

use crate::app::AppState;
use crate::db::Quiesce;
use crate::holders::{self, ledger};

use super::{fallback, pause_all, reap, services, PauseRequest};
use crate::owner_action;

/// Pauses (or, for the same release, takes up the open pause again), reaps
/// and reports. The report's `unresolved` lists what still holds the home;
/// the install must not go ahead while it isn't empty.
pub fn start(
    app: &Arc<AppState>,
    req: &PauseRequest<'_>,
    now: DateTime<Utc>,
) -> anyhow::Result<(Quiesce, Value)> {
    let q = match app.db.open_quiesce()? {
        // A retry after the owner cleared what held the home.
        Some(open) if open.release_id.is_some() && open.release_id.as_deref() == req.release_id => {
            open
        }
        _ => pause_all(app, req, now)?,
    };
    let exempt = q.exempt_bot.clone();
    let roots = holders::spellings(&app.cfg.home);
    let before = holders::list(&roots).unwrap_or_default();

    let sessions = ledger::session_processes(None);
    let (spared, targets): (Vec<_>, Vec<_>) = sessions
        .into_iter()
        .partition(|e| exempt.as_deref() == Some(e.bot_id.as_str()));
    let reaped = reap::reap(&targets);
    let services = services::stop_holding(app, &q, &before);

    let spared: HashSet<u32> = spared.iter().map(|e| e.pid).collect();
    let known = ledger::session_processes(None);
    let unresolved: Vec<Value> = holders::list(&roots)
        .unwrap_or_default()
        .into_iter()
        .filter(|h| !spared.contains(&h.pid))
        .map(|h| {
            let tie = known.iter().find(|e| e.pid == h.pid);
            // Named for the owner's banner (UX on H-117).
            let bot_name = tie
                .and_then(|e| app.db.get_bot(&e.bot_id).ok().flatten())
                .map(|b| b.name);
            let project_name = tie
                .and_then(|e| app.db.get_project(&e.project_id).ok().flatten())
                .map(|p| crate::db::Db::display_project_name(&p));
            json!({
                "pid": h.pid, "command": h.command, "cwd": h.cwd,
                "path": h.path, "project_id": tie.map(|e| &e.project_id),
                "bot_id": tie.map(|e| &e.bot_id),
                "bot_name": bot_name, "project_name": project_name,
            })
        })
        .collect();
    let mut report = q.report.clone();
    if !report.is_object() {
        report = json!({});
    }
    report["reaped"] = json!(reaped);
    report["services"] = json!(services);
    report["services_changed"] = json!(services::changed_since_start(app));
    report["unresolved"] = json!(unresolved);
    let phase = if unresolved.is_empty() {
        "ready"
    } else {
        // The owner gets one Run card that stops them (R4).
        match fallback::file(app, &q, &unresolved, &services) {
            Ok(Some(card)) => report["owner_action"] = owner_action::view(app, &card),
            Ok(None) => {}
            Err(e) => tracing::warn!(error = %e, "could not file the owner's Run card"),
        }
        "blocked"
    };
    app.db.set_quiesce_phase(&q.id, phase, &report)?;
    super::changed(app);
    let q = app.db.get_quiesce(&q.id)?.unwrap_or(q);
    Ok((q, report))
}
