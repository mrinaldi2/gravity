//! Checks run on a linked computer (H-283). The board's home asks that
//! computer's daemon to run a job (`check_run`); it runs it with its own
//! runner, exactly as the home runs one ([`check_exec`]), and sends the
//! result back from the exit status (`check_result`). The log stays on
//! that computer, recorded as `<computer>:<path>`.

use std::sync::Arc;
use std::time::Duration;

use bus::Peer;
use serde_json::{json, Value};

use crate::app::AppState;
use crate::check_exec::{self, Outcome, Spec};
use crate::peer::refuse;
use crate::prs::check_model::{CheckResult, CheckRun};
use crate::prs::checks::RunLog;
use crate::prs::{check_checkout, repo};

/// The peer request that asks a computer to run a check.
pub const RUN: &str = "check_run";
/// The peer event that carries a check's result back to the board's home.
pub const RESULT: &str = "check_result";

/// How long a finished job's result waits for the link to the home.
const RESULT_PATIENCE: Duration = Duration::from_secs(30 * 60);

/// Asks the linked computer `machine` to run `job`. Refused (its disk under
/// the floor, a repository it doesn't have), the check waits.
pub(super) async fn start(
    app: &Arc<AppState>,
    check: &CheckRun,
    machine: &str,
    job: &str,
    spec: &Spec,
) -> anyhow::Result<()> {
    let peer = super::check_jobs::peer_for(app, &check.project_id, machine)
        .ok_or_else(|| anyhow::anyhow!("{machine} is offline"))?;
    let frame = json!({
        "type": RUN,
        "project_id": check.project_id,
        "job": job,
        "url": spec.url,
        "sha": spec.sha,
        "run": spec.run,
    });
    app.peers.request(&peer.id, frame).await?;
    Ok(())
}

/// A linked board home asks this computer to run a check: the repository
/// must be one the linked project has, cloned from this side's own URL for
/// it, and the disk must be above the floor. The run goes on after the
/// answer; its result is sent back when it ends.
pub fn serve_run(app: &Arc<AppState>, peer: &Peer, frame: &Value) -> anyhow::Result<Value> {
    let field = |k: &str| frame.get(k).and_then(Value::as_str).unwrap_or_default();
    let link = app
        .db
        .project_link_by_remote(&peer.id, field("project_id"))?
        .ok_or_else(|| refuse("not_linked", "that project is not linked with one here"))?;
    let asked = field("url");
    let own = app.db.project_repo(&link.project_id)?.map(|r| r.url);
    let url = own
        .into_iter()
        .chain(app.db.extra_repos(&link.project_id)?)
        .find(|url| repo::same(url, asked))
        .ok_or_else(|| {
            refuse(
                "not_found",
                format!("{asked} isn't one of this project's repositories"),
            )
        })?;
    check_checkout::disk_floor(&app.cfg.home, app.cfg.checks.disk_floor_gb)
        .map_err(|low| refuse("at_capacity", low.to_string()))?;
    let job = field("job").to_string();
    check_exec::job_dir(&app.cfg.home, &job).map_err(|e| refuse("invalid", e.to_string()))?;
    let spec = Spec {
        url,
        sha: field("sha").to_string(),
        run: field("run").to_string(),
    };
    let (app, peer_id) = (app.clone(), peer.id.clone());
    tokio::spawn(async move {
        let outcome = check_exec::run(&app.cfg, &job, spec).await;
        send_result(&app, &peer_id, &job, &outcome).await;
    });
    Ok(json!({}))
}

/// Sends a result home, waiting a while for the link if it is down.
async fn send_result(app: &AppState, peer_id: &str, job: &str, outcome: &Outcome) {
    let frame = super::check_jobs::result_frame(job, outcome);
    let deadline = tokio::time::Instant::now() + RESULT_PATIENCE;
    while !app.peers.is_online(peer_id) {
        if tokio::time::Instant::now() > deadline {
            tracing::warn!(
                job,
                "check result not sent: the board's home stayed offline"
            );
            return;
        }
        tokio::time::sleep(Duration::from_secs(10)).await;
    }
    app.peers.notify(peer_id, frame);
}

/// A result from the computer a job was routed to; from any other, it is
/// ignored.
pub fn receive_result(app: &AppState, peer_id: &str, frame: &Value) {
    let Ok(Some(peer)) = app.db.get_peer(peer_id) else {
        return;
    };
    let job = frame["job"].as_str().unwrap_or_default();
    let routed_there = app
        .db
        .board_read(|t| t.check_job(job))
        .ok()
        .flatten()
        .is_some_and(|j| j.machine == peer.name && j.ended_at.is_none());
    let result = frame["result"]
        .as_str()
        .and_then(CheckResult::parse)
        .filter(|r| r.is_final());
    let (true, Some(result)) = (routed_there, result) else {
        tracing::warn!(peer = %peer.name, job, "check result ignored");
        return;
    };
    let note: String = frame["note"]
        .as_str()
        .unwrap_or_default()
        .chars()
        .take(500)
        .collect();
    let log = match frame["log"].as_str() {
        Some(path) => RunLog::There(format!("{}:{path}", peer.name)),
        None => RunLog::None,
    };
    let outcome = Outcome {
        result,
        note,
        log: None,
    };
    super::check_jobs::finish(app, job, &outcome, log);
}
