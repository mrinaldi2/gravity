//! What the owner sees of cleanups, and their word on one (H-261 §15.6,
//! CL-2): a PR's cleanup as one summary; and `cleanup_resolve` for a job
//! held 3 days or failed. Keep leaves the tree; Remove anyway deletes a
//! salvaged dirty tree, after saving its work again ([`run::remove_anyway`]).
//! Only the owner's app or a paired device answers, and only on the board's
//! home: a linked computer refuses it (ruling 6df14f7a). A tree on a linked
//! computer is removed by that computer, asked by the home with
//! `cleanup_force`; it answers with `cleanup_forced`.

use std::path::PathBuf;
use std::sync::Arc;

use bus::Peer;
use serde_json::{json, Value};

use super::model::{Job, JobState, Kind, Outcome};
use super::run::{self, Target};
use crate::app::AppState;
use crate::board::release::machines;
use crate::db::OwnerProof;
use crate::decisions::{conflict, not_found};
use crate::peer::refuse;
use crate::prs::model::Pr;

/// The peer request that asks a computer to remove a held tree anyway.
pub const FORCE: &str = "cleanup_force";
/// The peer event that says how it went.
pub const FORCED: &str = "cleanup_forced";

/// A held job's unsaved work was saved first.
pub fn salvaged(job: &Job) -> bool {
    job.reason.contains("salvaged to ")
}

fn item_json(job: &Job) -> Value {
    json!({
        "job_id": job.id, "machine": job.machine, "kind": job.kind.as_str(),
        "state": job.state.as_str(), "path": job.path_or_ref, "reason": job.reason,
        "freed_bytes": job.bytes_freed, "salvaged": salvaged(job),
    })
}

/// A PR's cleanup as the app shows it: its worktrees' jobs (the discovery
/// jobs are the daemon's own), their sum, and the remote branch.
pub fn summary_json(app: &AppState, pr: &Pr) -> anyhow::Result<Value> {
    let (jobs, branch) = app
        .db
        .board_read(|t| Ok((t.cleanup_jobs_of_pr(&pr.id)?, t.branch_cleanup(&pr.id)?)))?;
    let trees: Vec<&Job> = jobs.iter().filter(|j| j.kind == Kind::Worktree).collect();
    if jobs.is_empty() && branch.is_none() {
        return Ok(Value::Null);
    }
    let any = |s: JobState| jobs.iter().any(|j| j.state == s);
    let state = if any(JobState::Queued) {
        if jobs.iter().any(|j| j.attempts > 0 || j.sent_at.is_some()) {
            "running"
        } else {
            "queued"
        }
    } else if any(JobState::Failed) {
        "failed"
    } else if any(JobState::Held) {
        "held"
    } else {
        "done"
    };
    Ok(json!({
        "state": state,
        "freed_bytes": trees.iter().map(|j| j.bytes_freed).sum::<u64>(),
        "items": trees.iter().map(|j| item_json(j)).collect::<Vec<_>>(),
        "branch_deleted": branch.is_some_and(|(deleted, _)| deleted),
    }))
}

/// Where a job's work was (and is) saved on its computer.
fn salvage_dir(home: &std::path::Path, project_id: &str, group: &str, tree: &str) -> PathBuf {
    let name = std::path::Path::new(tree).file_name().unwrap_or_default();
    home.join("salvage").join(project_id).join(group).join(name)
}

/// The salvage group: the PR's number, or `sweep`.
fn group_of(app: &AppState, job: &Job) -> String {
    job.pr_id
        .as_deref()
        .and_then(|id| app.db.board_read(|t| t.pr_by_id(id)).ok().flatten())
        .map_or_else(|| "sweep".to_string(), |pr| pr.number.to_string())
}

/// The owner's word on job `job_id` of `project`, on the board's home.
pub fn resolve(
    app: &Arc<AppState>,
    project: &str,
    job_id: &str,
    action: &str,
    proof: &OwnerProof,
) -> anyhow::Result<Value> {
    let by = format!("owner:{}", crate::prs::owner::provenance(proof)?);
    let (job, answered, here) = app.db.board_read(|t| {
        Ok((
            t.cleanup_job(job_id)?,
            t.cleanup_resolution(job_id)?,
            machines::this_computer(t)?,
        ))
    })?;
    let job = job
        .filter(|j| j.project_id == project && j.kind == Kind::Worktree)
        .ok_or_else(|| not_found(format!("no cleanup {job_id} in this project")))?;
    if !matches!(job.state, JobState::Held | JobState::Failed) {
        return Err(conflict(format!(
            "that cleanup is {}, not waiting for you",
            job.state.as_str()
        )));
    }
    if let Some(given) = answered {
        return Err(conflict(format!("you already chose {given}")));
    }
    match action {
        "keep" => {
            app.db
                .board_tx(|t| t.resolve_cleanup(&job.id, "keep", &by))?;
        }
        "remove" => {
            if !salvaged(&job) {
                return Err(conflict(
                    "only a tree whose unsaved work was saved first is removed anyway",
                ));
            }
            app.db
                .board_tx(|t| t.resolve_cleanup(&job.id, "remove", &by))?;
            if job.machine == here {
                let target = target_here(app, &job.project_id, &job, None);
                let salvage = salvage_dir(
                    &app.cfg.home,
                    &job.project_id,
                    &group_of(app, &job),
                    &job.path_or_ref,
                );
                let outcome = run::remove_anyway(app, &target, &salvage);
                apply(app, &job, &outcome)?;
            } else {
                ask(app, &job)?;
            }
        }
        other => anyhow::bail!("'action' is remove or keep, not {other:?}"),
    }
    let job = app
        .db
        .board_read(|t| t.cleanup_job(&job.id))?
        .unwrap_or(job);
    Ok(item_json(&job))
}

fn target_here(app: &AppState, project_id: &str, job: &Job, bot_name: Option<&str>) -> Target {
    let bots = super::batch::bots_here(app, project_id);
    let bot = job
        .bot_id
        .as_deref()
        .and_then(|id| bots.iter().find(|b| b.id == id))
        .or_else(|| bots.iter().find(|b| Some(b.name.as_str()) == bot_name));
    Target {
        path: PathBuf::from(&job.path_or_ref),
        main_clone: job.main_clone.as_ref().map(PathBuf::from),
        bot_name: bot.map(|b| b.name.clone()).unwrap_or_default(),
        workspace: bot.and_then(|b| b.workspace.clone()),
        no_other_work: false,
    }
}

/// What a Remove anyway came to: done, or back to the owner with why.
fn apply(app: &AppState, job: &Job, outcome: &Outcome) -> anyhow::Result<()> {
    app.db.board_tx(|t| match outcome {
        Outcome::Done { bytes } => t.owner_removed_cleanup(&job.id, *bytes),
        Outcome::Failed(why) => t.reopen_cleanup(&job.id, JobState::Failed, why),
        Outcome::Held(why) | Outcome::Busy(why) => t.reopen_cleanup(&job.id, JobState::Held, why),
    })
}

/// Asks the job's computer to remove it anyway; it answers later.
fn ask(app: &Arc<AppState>, job: &Job) -> anyhow::Result<()> {
    let peer = app
        .db
        .get_peer_by_name(&job.machine)?
        .filter(|p| p.revoked_at.is_none() && app.peers.is_online(&p.id))
        .ok_or_else(|| {
            conflict(format!(
                "{} is offline; try again once it is back",
                job.machine
            ))
        })?;
    let bot_name = job
        .bot_id
        .as_deref()
        .and_then(|id| app.db.get_bot(id).ok().flatten())
        .map(|b| b.name);
    let frame = json!({
        "type": FORCE, "project_id": job.project_id, "job_id": job.id,
        "path": job.path_or_ref, "main_clone": job.main_clone, "bot_name": bot_name,
        "group": group_of(app, job),
    });
    let (app, peer_id, job) = (app.clone(), peer.id, job.clone());
    if let Ok(rt) = tokio::runtime::Handle::try_current() {
        rt.spawn(async move {
            if let Err(error) = app.peers.request(&peer_id, frame).await {
                let why = format!("{} refused: {error:#}", job.machine);
                let _ = apply(&app, &job, &Outcome::Held(why));
            }
        });
    }
    Ok(())
}

/// A board home asks this computer to remove a held tree anyway: only the
/// project's board home, for a project linked here. It runs after the
/// answer, and saves the tree's work again before anything is deleted.
pub fn serve_force(app: &Arc<AppState>, peer: &Peer, frame: &Value) -> anyhow::Result<Value> {
    let field = |k: &str| frame[k].as_str().unwrap_or_default().to_string();
    let link = app
        .db
        .project_link_by_remote(&peer.id, &field("project_id"))?
        .ok_or_else(|| refuse("not_linked", "that project is not linked with one here"))?;
    if app.board_mirror.home_peer(&link.project_id).as_deref() != Some(peer.id.as_str()) {
        return Err(refuse(
            "forbidden",
            format!("{} isn't this project's board home", peer.name),
        ));
    }
    let job = Job {
        id: field("job_id"),
        project_id: link.project_id.clone(),
        pr_id: None,
        machine: String::new(),
        kind: Kind::Worktree,
        path_or_ref: field("path"),
        main_clone: frame["main_clone"].as_str().map(str::to_string),
        bot_id: None,
        state: JobState::Held,
        reason: String::new(),
        bytes_freed: 0,
        attempts: 0,
        busy_since: None,
        next_at: None,
        sent_at: None,
        at: chrono::Utc::now(),
    };
    let target = target_here(app, &link.project_id, &job, frame["bot_name"].as_str());
    let salvage = salvage_dir(
        &app.cfg.home,
        &link.project_id,
        &field("group"),
        &job.path_or_ref,
    );
    let (app, peer_id, home_project) = (app.clone(), peer.id.clone(), field("project_id"));
    tokio::spawn(async move {
        let a = app.clone();
        let Ok(outcome) =
            tokio::task::spawn_blocking(move || run::remove_anyway(&a, &target, &salvage)).await
        else {
            return;
        };
        app.peers.notify(
            &peer_id,
            json!({"type": FORCED, "project_id": home_project, "job_id": job.id,
                   "outcome": outcome.to_json()}),
        );
    });
    Ok(json!({}))
}

/// What a linked computer did with a Remove anyway: only its own job, and
/// only one the owner asked for.
pub fn receive_forced(app: &AppState, peer_id: &str, frame: &Value) {
    let Ok(Some(peer)) = app.db.get_peer(peer_id) else {
        return;
    };
    let id = frame["job_id"].as_str().unwrap_or_default();
    let (job, asked) = match app
        .db
        .board_read(|t| Ok((t.cleanup_job(id)?, t.cleanup_resolution(id)?)))
    {
        Ok(found) => found,
        Err(_) => return,
    };
    let Some(job) = job.filter(|j| j.machine == peer.name) else {
        tracing::warn!(peer = %peer.name, "a Remove anyway answer for no job of its own");
        return;
    };
    if asked.as_deref() != Some("remove") {
        return;
    }
    if let Some(outcome) = Outcome::from_json(&frame["outcome"]) {
        if let Err(error) = apply(app, &job, &outcome) {
            tracing::warn!(job = %job.id, %error, "a Remove anyway answer wasn't kept");
        }
    }
}
