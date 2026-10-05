//! Running a claimed owner action in the background (H-117 R1).

use std::sync::Arc;

use chrono::Utc;
use serde_json::json;

use crate::app::AppState;
use crate::decisions::{conflict, invalid};
use crate::events::Push;

use super::model::{OwnerAction, State};
use super::{audit, changed, first_line, load, redact, run, sha256_of, tell};

/// Claims the action for one run (by the hash the owner's client showed)
/// and runs it in the background. Returns it, running.
pub fn start_run(
    app: &Arc<AppState>,
    actor: &str,
    id: &str,
    sha256: &str,
) -> anyhow::Result<OwnerAction> {
    let a = load(app, None, id)?;
    if a.proposal.target_machine != app.db.daemon_id()? {
        return Err(invalid("that action runs on another computer"));
    }
    if a.sha256 != sha256 {
        audit(
            app,
            id,
            actor,
            "refused",
            json!({ "why": "sha256 mismatch", "sent": sha256 }),
        );
        return Err(conflict(
            "the action differs from the one you were shown; reload it",
        ));
    }
    if !app.db.claim_owner_action(id, sha256, actor, Utc::now())? {
        return Err(conflict(format!(
            "owner action {id} is {}, not waiting to run",
            a.state.as_str()
        )));
    }
    audit(app, id, actor, "run", json!({ "sha256": sha256 }));
    let running = load(app, None, id)?;
    changed(app, &running);
    let (app, job) = (app.clone(), running.clone());
    tokio::spawn(async move { execute(&app, job).await });
    Ok(running)
}

/// Pinned files that changed since the proposal, as `path: was → now`.
pub fn pin_drift(a: &OwnerAction) -> Vec<String> {
    a.proposal
        .pinned_files
        .iter()
        .filter_map(|p| {
            let now = sha256_of(&p.path).unwrap_or_else(|_| "missing".to_string());
            (now != p.sha256).then(|| format!("{}: {} → {now}", p.path, p.sha256))
        })
        .collect()
}

async fn execute(app: &Arc<AppState>, a: OwnerAction) {
    let log = app
        .cfg
        .home
        .join("logs")
        .join("owner-actions")
        .join(format!("{}.log", a.id));
    let drift = pin_drift(&a);
    let ran = if drift.is_empty() {
        let id = a.id.clone();
        let events = app.events.clone();
        run::execute(&a.proposal, &log, move |chunk| {
            events.push(Push::OwnerActionOutput {
                id: id.clone(),
                chunk: redact::redact(chunk),
            });
        })
        .await
    } else {
        run::Ran {
            state: State::Failed,
            exit_code: None,
            output: format!(
                "Refused: pinned files changed since the proposal:\n{}\n",
                drift.join("\n")
            ),
        }
    };
    let tail = redact::tail(&ran.output);
    let path = log.display().to_string();
    if let Err(e) = app.db.finish_owner_action(
        &a.id,
        ran.state,
        ran.exit_code,
        Some(&path),
        &tail,
        Utc::now(),
    ) {
        tracing::warn!(error = %e, "could not record the owner action's end");
    }
    audit(
        app,
        &a.id,
        "daemon",
        "finished",
        json!({ "state": ran.state.as_str(), "exit_code": ran.exit_code }),
    );
    if let Ok(done) = load(app, None, &a.id) {
        changed(app, &done);
        let short: String = tail
            .chars()
            .rev()
            .take(3000)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        tell(
            app,
            &done,
            &format!(
                "Owner action {} ({}) {}{}.\n{short}",
                done.id,
                first_line(&done.proposal.content),
                ran.state.as_str().replace('_', " "),
                ran.exit_code
                    .map(|c| format!(", exit {c}"))
                    .unwrap_or_default(),
            ),
        );
    }
}
