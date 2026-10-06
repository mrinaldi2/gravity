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
        if let Some(card) = report["owner_action"]["id"].as_str() {
            line.push_str(&format!(
                " The owner has a Run card ({card}) that stops them; start again once it ran."
            ));
        }
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
/// daemon against the one being installed (ARCH-R50 S1):
/// - only once the install has started, so a crash or KeepAlive restart
///   of the old daemon before the swap keeps the pause open;
/// - by the installed binary's sha256 when the install recorded it, so a
///   same-version reinstall that rolled back reads as rolled back;
/// - by version otherwise (a Windows setup seals its binary).
///
/// `None` keeps the pause open.
pub fn boot_outcome(
    q: &Quiesce,
    running_version: &str,
    running_sha256: Option<&str>,
) -> Option<&'static str> {
    if q.phase != INSTALL_STARTED {
        return None;
    }
    let installing = q.version.as_deref()?;
    let ok = match q.report["install"]["binary_sha256"].as_str() {
        Some(want) => running_sha256 == Some(want),
        None => installing.trim_start_matches('v') == running_version,
    };
    Some(if ok { "install_ok" } else { "rolled_back" })
}

/// The phase a pause is in once the install has been handed to the system.
pub const INSTALL_STARTED: &str = "install_started";

/// The sha256 of a file, read in full.
pub fn file_sha256(path: &std::path::Path) -> anyhow::Result<String> {
    use sha2::{Digest, Sha256};
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    std::io::copy(&mut file, &mut hasher)?;
    Ok(hex::encode(hasher.finalize()))
}

/// At daemon start: ends an install's open pause (see the module docs).
pub fn on_boot(app: &Arc<AppState>) {
    // An install whose rollback failed too left word for the owner.
    crate::board::release::install::rollback_notice::report(app);
    let Ok(Some(q)) = app.db.open_quiesce() else {
        return;
    };
    let running_sha = std::env::current_exe()
        .ok()
        .and_then(|exe| file_sha256(&exe).ok());
    let Some(outcome) = boot_outcome(&q, env!("CARGO_PKG_VERSION"), running_sha.as_deref()) else {
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

    fn started(version: Option<&str>, sha: Option<&str>) -> Quiesce {
        let mut q = pause(version);
        q.phase = INSTALL_STARTED.into();
        q.report = json!({ "install": { "binary_sha256": sha } });
        q
    }

    #[test]
    fn a_boot_before_the_install_started_keeps_the_pause() {
        // ARCH-R50 S1 / F2: the old daemon restarting before the swap.
        assert_eq!(boot_outcome(&pause(Some("0.17.0")), "0.16.3", None), None);
        assert_eq!(boot_outcome(&pause(Some("0.17.0")), "0.17.0", None), None);
    }

    #[test]
    fn once_started_the_binary_hash_decides_and_version_is_the_fallback() {
        // The installed binary's hash: same version or not, it decides.
        let q = started(Some("0.17.0"), Some("aaa"));
        assert_eq!(boot_outcome(&q, "0.17.0", Some("aaa")), Some("install_ok"));
        assert_eq!(boot_outcome(&q, "0.17.0", Some("bbb")), Some("rolled_back"));
        assert_eq!(boot_outcome(&q, "0.17.0", None), Some("rolled_back"));
        // A Windows setup seals its binary: the version decides.
        let q = started(Some("v0.17.0"), None);
        assert_eq!(boot_outcome(&q, "0.17.0", None), Some("install_ok"));
        assert_eq!(boot_outcome(&q, "0.16.3", None), Some("rolled_back"));
        // Not an install's pause: the owner or the dead-man ends it.
        assert_eq!(boot_outcome(&started(None, None), "0.17.0", None), None);
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
