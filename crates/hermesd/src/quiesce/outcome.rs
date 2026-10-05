//! What a pause for an install tells, and how it ends across the install's
//! restart (H-117 Q4).
//!
//! The report goes on the release, as a `quiesce` event its review shows,
//! and to each project's lead as a note. At boot, a daemon finding a pause
//! open for an install ends it: running the version being installed, the
//! install worked (`install_ok`); running another, the install's rollback
//! brought the old daemon back (`rolled_back`). Either way every project
//! resumes; the dead-man switch covers a crash between the two.

use std::sync::Arc;

use chrono::Utc;
use serde_json::{json, Value};

use crate::app::AppState;
use crate::board::model::Role;
use crate::board::release::model::ReleaseEvent;
use crate::db::Quiesce;

/// One line of what a pause did.
pub fn summary(report: &Value) -> String {
    let count = |key: &str| report[key].as_array().map_or(0, Vec::len);
    let paused = &report["paused"];
    let mut line = format!(
        "Paused {} bots, {} routines, {} workers ({} messages queued); reaped {} processes; \
         stopped {} services.",
        paused["bots"].as_u64().unwrap_or(0),
        paused["routines"].as_u64().unwrap_or(0),
        paused["workers"].as_u64().unwrap_or(0),
        paused["queued"].as_u64().unwrap_or(0),
        count("reaped"),
        report["services"]
            .as_array()
            .map_or(0, |s| s.iter().filter(|s| s["stopped"] == true).count()),
    );
    let unresolved = count("unresolved");
    if unresolved > 0 {
        line.push_str(&format!(
            " {unresolved} other processes still hold the home, so the install waits."
        ));
    }
    line
}

/// Posts `report` on the pause's release and tells every lead.
pub fn announce(app: &AppState, q: &Quiesce, report: &Value) {
    let line = summary(report);
    if let Some(release_id) = &q.release_id {
        record(app, release_id, "quiesce", &q.started_by, &line, report);
    }
    super::tell_roles(
        app,
        &[Role::Lead],
        &format!(
            "{} (this computer, for an install) {line}",
            super::reason_line(q)
        ),
    );
}

fn record(app: &AppState, release_id: &str, kind: &str, actor: &str, note: &str, detail: &Value) {
    let Ok(Some(release)) = app.db.board_read(|t| t.release(release_id)) else {
        return;
    };
    let event = ReleaseEvent {
        release_id: release.id.clone(),
        release_name: release.name.clone(),
        related_id: None,
        kind: kind.to_string(),
        actor: actor.to_string(),
        note: Some(note.to_string()),
        detail: detail.clone(),
        at: Utc::now(),
    };
    if let Err(e) = app
        .db
        .board_tx(|t| t.record_release_event(&event, &release.project_id))
    {
        tracing::warn!(error = %e, "could not record the pause on the release");
    }
}

/// How an install's pause ends when a daemon boots with it open: by this
/// daemon's version against the one being installed. `None` for a pause
/// that isn't an install's.
pub fn boot_outcome(q: &Quiesce, running: &str) -> Option<&'static str> {
    let installing = q.version.as_deref()?;
    Some(if installing.trim_start_matches('v') == running {
        "install_ok"
    } else {
        "rolled_back"
    })
}

/// At daemon start: ends an install's open pause (see the module docs).
pub fn on_boot(app: &Arc<AppState>) {
    let Ok(Some(q)) = app.db.open_quiesce() else {
        return;
    };
    let Some(outcome) = boot_outcome(&q, env!("CARGO_PKG_VERSION")) else {
        return;
    };
    match super::resume_all(app, outcome, Utc::now()) {
        Ok(Some(closed)) => {
            let line = match outcome {
                "install_ok" => "The install finished; every project resumed.",
                _ => "The install was rolled back; every project resumed on the old version.",
            };
            if let Some(release_id) = &closed.release_id {
                let detail = json!({ "outcome": outcome, "resumed": closed.report["resumed"] });
                record(app, release_id, outcome, "daemon", line, &detail);
            }
            super::tell_roles(app, &[Role::Lead, Role::Devops], line);
        }
        Ok(None) => {}
        Err(e) => tracing::warn!(error = %e, "could not end the install's pause at boot"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pause(version: Option<&str>) -> Quiesce {
        Quiesce {
            id: "q".into(),
            reason: "install of 0.17.0".into(),
            release_id: Some("r".into()),
            version: version.map(str::to_string),
            exempt_bot: None,
            started_by: "bot:t".into(),
            started_at: Utc::now(),
            deadline_at: Utc::now(),
            phase: "ready".into(),
            report: json!({}),
            services_stopped: Vec::new(),
            resumed_at: None,
            outcome: None,
        }
    }

    #[test]
    fn a_booting_daemon_reads_how_the_install_ended_from_its_own_version() {
        assert_eq!(
            boot_outcome(&pause(Some("0.17.0")), "0.17.0"),
            Some("install_ok")
        );
        assert_eq!(
            boot_outcome(&pause(Some("v0.17.0")), "0.17.0"),
            Some("install_ok")
        );
        assert_eq!(
            boot_outcome(&pause(Some("0.17.0")), "0.16.3"),
            Some("rolled_back")
        );
        // Not an install's pause: the owner or the dead-man ends it.
        assert_eq!(boot_outcome(&pause(None), "0.17.0"), None);
    }

    #[test]
    fn the_summary_counts_what_the_pause_did_and_says_why_it_waits() {
        let report = json!({
            "paused": {"bots": 3, "routines": 2, "workers": 1, "queued": 4},
            "reaped": [{"pid": 1}, {"pid": 2}],
            "services": [{"name": "colima", "stopped": true}],
            "unresolved": [{"pid": 9}],
        });
        let line = summary(&report);
        assert!(line.starts_with("Paused 3 bots, 2 routines, 1 workers (4 messages queued)"));
        assert!(line.contains("reaped 2 processes; stopped 1 services."));
        assert!(
            line.contains("1 other processes still hold the home"),
            "{line}"
        );
    }
}
