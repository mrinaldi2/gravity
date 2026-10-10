//! Running the daily sweep (§15.5) on every computer: due once a day, or at
//! once from the owner's Clean up (§15.6, which also trims build caches).
//! The board's home plans each project's sweep and keeps what it did; a
//! linked computer asks it for the plan and sends the result back. Frames
//! carry the sender's own project id; the receiver finds its own through
//! the project link, and only a linked peer is answered.

use std::sync::Arc;

use bus::Peer;
use chrono::{DateTime, Utc};
use serde_json::{json, Value};

use super::sweep::{self, Plan, Swept};
use crate::app::AppState;
use crate::board::release::machines;
use crate::peer::refuse;

/// The peer request a linked computer makes for its sweep plan.
pub const PLAN: &str = "cleanup_sweep_plan";
/// The peer event that carries what its sweep did.
pub const SWEPT: &str = "cleanup_swept";

/// What one run of the sweep did on this computer.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Summary {
    pub trees: usize,
    pub freed: u64,
}

/// Runs the sweep when a day has passed since the last one here.
pub async fn if_due(app: &Arc<AppState>, now: DateTime<Utc>) -> anyhow::Result<Option<Summary>> {
    let last = app.db.board_read(|t| {
        let here = machines::this_computer(t)?;
        t.swept_at(&here)
    })?;
    if last.is_some_and(|at| now - at < sweep::DAILY) {
        return Ok(None);
    }
    run_now(app, now, false).await.map(Some)
}

/// The sweep of every project here, now; with `trim`, each bot's build
/// cache is trimmed as §15.2 allows too (the owner's Clean up).
pub async fn run_now(app: &Arc<AppState>, now: DateTime<Utc>, trim: bool) -> anyhow::Result<Summary> {
    let here = app.db.board_tx(|t| {
        let here = machines::this_computer(t)?;
        t.set_swept(&here, now)?;
        Ok(here)
    })?;
    let mut summary = Summary::default();
    for project in app.db.list_projects()? {
        let home = app.board_mirror.home_peer(&project.id);
        let plan = match &home {
            None => sweep::plan_at_home(app, &project.id, &here)?,
            Some(peer) => match ask(app, peer, &project.id).await {
                Ok(plan) => plan,
                Err(error) => {
                    tracing::info!(project = %project.name, %error, "no sweep plan from the home");
                    continue;
                }
            },
        };
        let (a, id, p) = (app.clone(), project.id.clone(), plan.clone());
        let (swept, trimmed) = tokio::task::spawn_blocking(move || {
            let swept = sweep::sweep(&a, &id, &p, now);
            let trimmed = if trim { trim_caches(&a, &id, &p) } else { 0 };
            (swept, trimmed)
        })
        .await?;
        summary.trees += swept.len();
        summary.freed += trimmed + freed(&swept);
        match home {
            None => sweep::record(app, &project.id, &here, &swept, now),
            Some(peer) => {
                let found: Vec<Value> = swept.iter().map(Swept::to_json).collect();
                app.peers.notify(
                    &peer,
                    json!({"type": SWEPT, "project_id": project.id, "found": found}),
                );
            }
        }
    }
    let home = app.cfg.home.clone();
    tokio::task::spawn_blocking(move || sweep::prune_salvage(&home, now)).await?;
    Ok(summary)
}

fn freed(swept: &[Swept]) -> u64 {
    swept
        .iter()
        .map(|s| match s.outcome {
            super::model::Outcome::Done { bytes } => bytes,
            _ => 0,
        })
        .sum()
}

/// Each of the project's bots here: its build cache trimmed when §15.2
/// allows (no live PR, under 20 GB free, or over its cap).
fn trim_caches(app: &AppState, project_id: &str, plan: &Plan) -> u64 {
    super::batch::bots_here(app, project_id)
        .into_iter()
        .filter_map(|b| Some((b.workspace?, b.name)))
        .map(|(ws, name)| super::remove::trim_cache(&ws, !plan.busy.contains(&name)))
        .sum()
}

async fn ask(app: &AppState, peer_id: &str, project_id: &str) -> anyhow::Result<Plan> {
    let reply = app
        .peers
        .request(peer_id, json!({"type": PLAN, "project_id": project_id}))
        .await?;
    Ok(Plan::from_json(&reply))
}

/// The board's home answers a linked computer's plan request for a project
/// linked with it, as that computer (by its peer name).
pub fn serve_plan(app: &Arc<AppState>, peer: &Peer, frame: &Value) -> anyhow::Result<Value> {
    let theirs = frame["project_id"].as_str().unwrap_or_default();
    let link = app
        .db
        .project_link_by_remote(&peer.id, theirs)?
        .ok_or_else(|| refuse("not_linked", "that project is not linked with one here"))?;
    if app.board_mirror.home_peer(&link.project_id).is_some() {
        return Err(refuse("forbidden", "this computer isn't the project's board home"));
    }
    Ok(sweep::plan_at_home(app, &link.project_id, &peer.name)?.to_json())
}

/// What a linked computer's sweep did, kept under its name.
pub fn receive_swept(app: &AppState, peer_id: &str, frame: &Value) {
    let Ok(Some(peer)) = app.db.get_peer(peer_id) else {
        return;
    };
    let theirs = frame["project_id"].as_str().unwrap_or_default();
    let Ok(Some(link)) = app.db.project_link_by_remote(&peer.id, theirs) else {
        tracing::warn!(peer = %peer.name, "a sweep result for a project not linked with it");
        return;
    };
    if app.board_mirror.home_peer(&link.project_id).is_some() {
        return;
    }
    let found: Vec<Swept> = frame["found"]
        .as_array()
        .map(|a| a.iter().filter_map(Swept::from_json).collect())
        .unwrap_or_default();
    sweep::record(app, &link.project_id, &peer.name, &found, Utc::now());
}
