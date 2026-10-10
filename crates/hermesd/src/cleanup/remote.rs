//! Cleanup on a linked computer (H-261 §15.2): the board's home sends a
//! `cleanup_request` with that computer's jobs for one merged PR; its daemon
//! runs them exactly as the home runs its own ([`super::batch::run`]) and
//! sends a `cleanup_result` back. Only the project's board home may ask
//! (ARCH-R63: board facts come from the home), and rule 1 is checked twice:
//! by the home before it asks, and here, against this computer's own fetch
//! of main, before anything is deleted. A computer that is offline isn't asked; once it is back online the
//! next pass asks it, so it runs its jobs when it reconnects. One that was
//! asked and didn't answer is asked again after 15 minutes: every step
//! re-checks what is on disk, so a second run is safe.

use std::sync::Arc;
use std::time::Duration;

use bus::Peer;
use chrono::{DateTime, Utc};
use serde_json::{json, Value};

use super::batch::{self, Answer, Batch, Discovered};
use super::model::{Job, Outcome};
use crate::app::AppState;
use crate::peer::refuse;
use crate::prs::repo;

/// The peer request that asks a computer to run its cleanup jobs.
pub const REQUEST: &str = "cleanup_request";
/// The peer event that carries what it did back to the board's home.
pub const RESULT: &str = "cleanup_result";

/// How long a finished batch's answer waits for the link to the home.
const RESULT_PATIENCE: Duration = Duration::from_secs(30 * 60);

/// The linked, online peer named `name` that `project` is shared with.
fn peer_for(app: &AppState, project: &str, name: &str) -> Option<Peer> {
    let peer = app.db.get_peer_by_name(name).ok()??;
    let linked = app.db.project_link(project, &peer.id).ok()?.is_some();
    (peer.revoked_at.is_none() && linked && app.peers.is_online(&peer.id)).then_some(peer)
}

/// Asks `machine` to run `jobs` of the PR `pr_id`, unless it was asked
/// lately or is offline (then it waits for the next pass).
pub async fn ask(
    app: &Arc<AppState>,
    pr_id: &str,
    machine: &str,
    jobs: Vec<Job>,
    now: DateTime<Utc>,
) {
    let jobs: Vec<Job> = jobs
        .into_iter()
        .filter(|j| j.sent_at.is_none_or(|at| now - at >= super::RESEND_AFTER))
        .collect();
    let Some(first) = jobs.first() else {
        return;
    };
    let Some(peer) = peer_for(app, &first.project_id, machine) else {
        return;
    };
    let (a, pr_id) = (app.clone(), pr_id.to_string());
    let built = tokio::task::spawn_blocking(move || {
        let pr = super::pr_of(&a, &pr_id).ok_or_else(|| Outcome::Failed("no such PR".into()))?;
        match super::verify_merge(&a, &pr) {
            Ok(url) => super::batch_for(&a, &pr, &url, &jobs)
                .map(|b| (b, jobs))
                .map_err(|e| Outcome::Failed(format!("{e:#}"))),
            Err(outcome) => {
                for job in &jobs {
                    super::record(&a, job, &outcome, now);
                }
                Err(outcome)
            }
        }
    })
    .await;
    let Ok(Ok((batch, jobs))) = built else {
        return;
    };
    let frame = batch.to_frame(&batch.project_id);
    if let Err(error) = app.peers.request(&peer.id, frame).await {
        tracing::warn!(machine, %error, "a computer refused its cleanup jobs");
        return;
    }
    let _ = app.db.board_tx(|t| {
        for job in &jobs {
            t.set_cleanup_sent(&job.id, now)?;
        }
        Ok(())
    });
}

/// A linked board home asks this computer to run its cleanup jobs: the
/// project must be linked here, the sender must be the peer whose board this
/// computer mirrors for it, and the repository one it has. The jobs run
/// after the answer, once this computer has seen main hold the merged
/// commit itself; what they did is sent back when they end.
pub fn serve_request(app: &Arc<AppState>, peer: &Peer, frame: &Value) -> anyhow::Result<Value> {
    let field = |k: &str| frame.get(k).and_then(Value::as_str).unwrap_or_default();
    let home_project = field("project_id").to_string();
    let link = app
        .db
        .project_link_by_remote(&peer.id, &home_project)?
        .ok_or_else(|| refuse("not_linked", "that project is not linked with one here"))?;
    if app.board_mirror.home_peer(&link.project_id).as_deref() != Some(peer.id.as_str()) {
        return Err(refuse(
            "forbidden",
            format!("{} isn't this project's board home", peer.name),
        ));
    }
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
    let batch = Batch::from_frame(frame, &link.project_id, url);
    let (app, peer_id) = (app.clone(), peer.id.clone());
    tokio::spawn(async move {
        let a = app.clone();
        let b = batch.clone();
        let Ok(answer) = tokio::task::spawn_blocking(move || checked(&a, &b)).await else {
            return;
        };
        let frame = answer.to_frame(&home_project, &batch.pr_id);
        let deadline = tokio::time::Instant::now() + RESULT_PATIENCE;
        while !app.peers.is_online(&peer_id) {
            if tokio::time::Instant::now() > deadline {
                tracing::warn!("cleanup result not sent: the board's home stayed offline");
                return;
            }
            tokio::time::sleep(Duration::from_secs(10)).await;
        }
        app.peers.notify(&peer_id, frame);
    });
    Ok(json!({}))
}

/// Runs `batch` once main here holds its merged commit; until then every
/// job gets that outcome and nothing is touched.
fn checked(app: &AppState, b: &Batch) -> Answer {
    match super::merged::main_holds(app, &b.project_id, &b.url, b.pr_number, &b.merged_sha) {
        Ok(_) => batch::run(app, b),
        Err(outcome) => {
            let outcome = match outcome {
                Outcome::Held(why) => Outcome::Held(format!("checked on this computer: {why}")),
                other => other,
            };
            Answer {
                results: b
                    .jobs
                    .iter()
                    .map(|w| (w.id.clone(), outcome.clone()))
                    .collect(),
                discovered: Vec::new(),
            }
        }
    }
}

/// What a linked computer did with its jobs; only that computer's own jobs
/// change, and what it found is recorded under its name.
pub fn receive_result(app: &AppState, peer_id: &str, frame: &Value) {
    let Ok(Some(peer)) = app.db.get_peer(peer_id) else {
        return;
    };
    let Some(pr) = frame["pr"].as_str().and_then(|id| super::pr_of(app, id)) else {
        return;
    };
    let linked = app
        .db
        .project_link(&pr.project_id, &peer.id)
        .ok()
        .flatten()
        .is_some();
    if !linked {
        tracing::warn!(peer = %peer.name, "a cleanup result for a project not linked with it");
        return;
    }
    let list = |k: &str| frame[k].as_array().cloned().unwrap_or_default();
    let results: Vec<(String, Outcome)> = list("results")
        .iter()
        .filter_map(|r| {
            Some((
                r["id"].as_str()?.to_string(),
                Outcome::from_json(&r["outcome"])?,
            ))
        })
        .collect();
    let discovered: Vec<Discovered> = list("discovered")
        .iter()
        .filter_map(|d| {
            Some(Discovered {
                path: d["path"].as_str()?.to_string(),
                main_clone: d["main_clone"].as_str()?.to_string(),
                bot_id: d["bot_id"].as_str().map(str::to_string),
                bot_name: d["bot_name"].as_str().map(str::to_string),
                outcome: Outcome::from_json(&d["outcome"])?,
            })
        })
        .collect();
    super::apply(app, &pr, &peer.name, &results, &discovered, Utc::now());
}
