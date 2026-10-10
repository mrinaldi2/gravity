//! Cleanup after a merge (H-261 §15.1–15.3, CL-1). Once a PR merged, the
//! board's home queues a `cleanup_job` for each worktree the bot reported
//! and one `discover` job per computer (this one and each linked one), then
//! runs this computer's jobs itself and asks each linked computer to run its
//! own with a `cleanup_request`. A computer that is offline is asked again
//! once it is back, so it runs them when it reconnects.
//!
//! Each removal follows §15.3 in order, and any rule that fails leaves the
//! tree where it is, `held` with the reason. A tree in use is tried again
//! every 15 minutes for 24 hours. A tree with unsaved work is saved first
//! (bundle and patch), then held, and is never removed without the owner
//! (ruling 7629a873). Bots never delete trees themselves: the daemon does,
//! and tells the author what went and how much it freed.

pub mod batch;
pub mod closed;
pub mod discover;
pub mod merged;
pub mod model;
pub mod remote;
pub mod remove;
pub mod run;
pub mod scope;
pub mod sweep;
pub mod sweep_remote;
pub mod unsaved;

use std::collections::{BTreeMap, HashSet};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde_json::Value;

use crate::app::AppState;
use crate::board::release::machines;
use crate::prs::model::Pr;
use batch::{Batch, Discovered, Work};
pub use merged::{ready, verify_merge};
use model::{human_bytes, Job, JobState, Kind, NewJob, Outcome};

/// How often the home looks for jobs due.
const EVERY: Duration = Duration::from_secs(60);
/// A tree in use is tried again this often…
pub const RETRY_EVERY: chrono::Duration = chrono::Duration::minutes(15);
/// …for this long, then held.
pub const BUSY_FOR: chrono::Duration = chrono::Duration::hours(24);
/// A linked computer that hasn't answered is asked again after this.
pub const RESEND_AFTER: chrono::Duration = chrono::Duration::minutes(15);

/// Queues the cleanup of a merged PR: its reported worktrees, and a
/// discovery on every computer the project is on. The number queued.
pub fn enqueue(app: &AppState, pr: &Pr) -> anyhow::Result<usize> {
    enqueue_until(app, pr, None)
}

/// [`enqueue`], with the jobs due only from `due` on (a closed PR's 7 days).
pub fn enqueue_until(
    app: &AppState,
    pr: &Pr,
    due: Option<DateTime<Utc>>,
) -> anyhow::Result<usize> {
    let peers: Vec<String> = app
        .db
        .project_links(&pr.project_id)?
        .into_iter()
        .filter_map(|l| app.db.get_peer(&l.peer_id).ok().flatten())
        .filter(|p| p.revoked_at.is_none())
        .map(|p| p.name)
        .collect();
    app.db.board_tx(|t| {
        let here = machines::this_computer(t)?;
        let new = |machine: &str, kind, path: &str, main: Option<&str>, bot: Option<&str>| NewJob {
            project_id: pr.project_id.clone(),
            pr_id: pr.id.clone(),
            machine: machine.to_string(),
            kind,
            path_or_ref: path.to_string(),
            main_clone: main.map(str::to_string),
            bot_id: bot.map(str::to_string),
        };
        let mut jobs: Vec<NewJob> = t
            .pr_worktrees(&pr.id)?
            .iter()
            .map(|w| {
                let main = Some(w.main_clone.as_str());
                new(&w.machine, Kind::Worktree, &w.path, main, Some(&w.bot_id))
            })
            .collect();
        for machine in std::iter::once(&here).chain(&peers) {
            jobs.push(new(
                machine,
                Kind::Discover,
                &pr.branch,
                None,
                Some(&pr.author),
            ));
        }
        let mut added = 0;
        for job in &jobs {
            added += usize::from(t.add_cleanup_job(job)?.is_some());
        }
        if let Some(due) = due {
            t.defer_cleanup_jobs(&pr.id, due)?;
        }
        Ok(added)
    })
}

pub fn spawn(app: Arc<AppState>) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(EVERY);
        loop {
            tick.tick().await;
            step(&app, Utc::now()).await;
        }
    });
}

/// Runs a pass now, when there is a runtime to run it on.
pub fn kick(app: &Arc<AppState>) {
    if let Ok(rt) = tokio::runtime::Handle::try_current() {
        let app = app.clone();
        rt.spawn(async move { step(&app, Utc::now()).await });
    }
}

/// The jobs a pass on this computer is running now.
fn running() -> &'static Mutex<HashSet<String>> {
    static RUNNING: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    RUNNING.get_or_init(Mutex::default)
}

/// One pass: each PR's due jobs, per computer.
pub async fn step(app: &Arc<AppState>, now: DateTime<Utc>) {
    let due = app
        .db
        .board_read(|t| Ok((t.cleanup_jobs_due(now)?, machines::this_computer(t)?)));
    let (due, here) = match due {
        Ok(d) => d,
        Err(error) => return tracing::warn!(%error, "listing cleanup jobs failed"),
    };
    let mut groups: BTreeMap<(String, String), Vec<Job>> = BTreeMap::new();
    for job in due {
        let Some(pr) = job.pr_id.clone() else {
            continue;
        };
        groups
            .entry((pr, job.machine.clone()))
            .or_default()
            .push(job);
    }
    for ((pr_id, machine), jobs) in groups {
        if machine == here {
            let jobs = claim(jobs);
            if jobs.is_empty() {
                continue;
            }
            let ids: Vec<String> = jobs.iter().map(|j| j.id.clone()).collect();
            let a = app.clone();
            let ran = tokio::task::spawn_blocking(move || run_here(&a, &pr_id, jobs, now)).await;
            running()
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .retain(|id| !ids.contains(id));
            if let Err(error) = ran {
                tracing::warn!(%error, "a cleanup pass failed");
            }
        } else {
            remote::ask(app, &pr_id, &machine, jobs, now).await;
        }
    }
}

/// The jobs no other pass here is running, now marked as running.
fn claim(jobs: Vec<Job>) -> Vec<Job> {
    let mut set = running().lock().unwrap_or_else(|e| e.into_inner());
    jobs.into_iter()
        .filter(|j| set.insert(j.id.clone()))
        .collect()
}

/// The batch for `jobs` of `pr` on one computer, as the home builds it.
pub fn batch_for(app: &AppState, pr: &Pr, url: &str, jobs: &[Job]) -> anyhow::Result<Batch> {
    let known = app
        .db
        .board_read(|t| t.cleanup_jobs_of_pr(&pr.id))?
        .into_iter()
        .filter(|j| {
            j.kind == Kind::Worktree && Some(&j.machine) == jobs.first().map(|f| &f.machine)
        })
        .map(|j| j.path_or_ref)
        .collect();
    let mut work = Vec::new();
    for job in jobs {
        let bot_id = job.bot_id.clone();
        let bot_name = bot_id
            .as_deref()
            .and_then(|id| app.db.get_bot(id).ok().flatten())
            .map(|b| b.name)
            .unwrap_or_default();
        let no_other_work = match bot_id.as_deref() {
            Some(id) => !app.db.board_read(|t| t.has_other_live_pr(id, &pr.id))?,
            None => false,
        };
        work.push(Work {
            id: job.id.clone(),
            kind: job.kind,
            path_or_ref: job.path_or_ref.clone(),
            main_clone: job.main_clone.clone(),
            bot_id,
            bot_name,
            no_other_work,
        });
    }
    Ok(Batch {
        project_id: pr.project_id.clone(),
        pr_id: pr.id.clone(),
        pr_number: pr.number,
        branch: pr.branch.clone(),
        url: url.to_string(),
        merged_sha: pr.merged_sha.clone().unwrap_or_default(),
        closed: pr.state == crate::prs::model::PrState::Closed,
        jobs: work,
        known,
    })
}

fn pr_of(app: &AppState, pr_id: &str) -> Option<Pr> {
    app.db.board_read(|t| t.pr_by_id(pr_id)).ok().flatten()
}

/// This computer's jobs of one PR, run here.
fn run_here(app: &AppState, pr_id: &str, jobs: Vec<Job>, now: DateTime<Utc>) {
    let Some(pr) = pr_of(app, pr_id) else {
        return;
    };
    let url = match ready(app, &pr) {
        Ok(url) => url,
        Err(outcome) => {
            for job in &jobs {
                record(app, job, &outcome, now);
            }
            return;
        }
    };
    let batch = match batch_for(app, &pr, &url, &jobs) {
        Ok(b) => b,
        Err(error) => return tracing::warn!(%error, "building a cleanup batch failed"),
    };
    let answer = batch::run(app, &batch);
    let machine = jobs[0].machine.clone();
    apply(app, &pr, &machine, &answer.results, &answer.discovered, now);
}

/// Records what a computer answered for `pr`'s jobs on `machine`.
pub fn apply(
    app: &AppState,
    pr: &Pr,
    machine: &str,
    results: &[(String, Outcome)],
    discovered: &[Discovered],
    now: DateTime<Utc>,
) {
    for (id, outcome) in results {
        let job = app.db.board_read(|t| t.cleanup_job(id)).ok().flatten();
        match job {
            Some(job) if job.machine == machine && job.pr_id.as_deref() == Some(&pr.id) => {
                record(app, &job, outcome, now)
            }
            _ => tracing::warn!(machine, job = %id, "a cleanup answer for no job of its own"),
        }
    }
    for found in discovered {
        let new = NewJob {
            project_id: pr.project_id.clone(),
            pr_id: pr.id.clone(),
            machine: machine.to_string(),
            kind: Kind::Worktree,
            path_or_ref: found.path.clone(),
            main_clone: Some(found.main_clone.clone()),
            bot_id: home_bot(app, &pr.project_id, found),
        };
        let added = app.db.board_tx(|t| t.add_cleanup_job(&new));
        if let Ok(Some(id)) = added {
            if let Ok(Some(job)) = app.db.board_read(|t| t.cleanup_job(&id)) {
                record(app, &job, &found.outcome, now);
            }
        }
    }
}

/// The home's own record of the bot a computer found a tree for: the same
/// id, else the project's bot of that name (a linked computer's stand-in).
fn home_bot(app: &AppState, project_id: &str, found: &Discovered) -> Option<String> {
    let bots = app.db.list_bots(Some(project_id)).ok()?;
    let by_id = found
        .bot_id
        .as_deref()
        .and_then(|id| bots.iter().find(|b| b.id == id));
    let by_name = found
        .bot_name
        .as_deref()
        .and_then(|name| bots.iter().find(|b| b.name == name));
    by_id.or(by_name).map(|b| b.id.clone())
}

/// One attempt's outcome, kept: a busy tree waits 15 minutes (held after
/// 24 hours), anything else is the job's last word.
pub fn record(app: &AppState, job: &Job, outcome: &Outcome, now: DateTime<Utc>) {
    let kept = app.db.board_tx(|t| match outcome {
        Outcome::Busy(why) => {
            let since = job.busy_since.unwrap_or(now);
            if now - since >= BUSY_FOR {
                let why = format!("still in use after 24 hours ({why})");
                t.finish_cleanup_job(&job.id, JobState::Held, &why, 0)
            } else {
                t.set_cleanup_busy(&job.id, why, since, now + RETRY_EVERY)?;
                Ok(false)
            }
        }
        Outcome::Done { bytes } => t.finish_cleanup_job(&job.id, JobState::Done, "", *bytes),
        Outcome::Held(why) => t.finish_cleanup_job(&job.id, JobState::Held, why, 0),
        Outcome::Failed(why) => t.finish_cleanup_job(&job.id, JobState::Failed, why, 0),
    });
    match kept {
        Ok(true) => tell_author(app, job),
        Ok(false) => {}
        Err(error) => tracing::warn!(job = %job.id, %error, "a cleanup outcome wasn't kept"),
    }
}

/// Tells the bot whose worktree it was what became of it: its next turn
/// mustn't `cd` into a path that is gone, and a held one waits on it.
fn tell_author(app: &AppState, job: &Job) {
    let Ok(Some(job)) = app.db.board_read(|t| t.cleanup_job(&job.id)) else {
        return;
    };
    let pr = job.pr_id.as_deref().and_then(|id| pr_of(app, id));
    let (Some(pr), Kind::Worktree) = (pr, job.kind) else {
        return;
    };
    let path = &job.path_or_ref;
    let on = &job.machine;
    let what = match pr.state {
        crate::prs::model::PrState::Closed => format!("PR #{} was closed", pr.number),
        _ => format!("PR #{} merged", pr.number),
    };
    let body = match job.state {
        JobState::Done => format!(
            "{what}; your worktree {path} on {on} was removed (freed {}).",
            human_bytes(job.bytes_freed)
        ),
        JobState::Held => format!(
            "{what}; your worktree {path} on {on} was kept: {}. Nothing was deleted.",
            job.reason
        ),
        JobState::Failed => format!(
            "{what}; removing your worktree {path} on {on} failed: {}. Nothing was deleted.",
            job.reason
        ),
        JobState::Queued => return,
    };
    let to = job.bot_id.clone().unwrap_or(pr.author.clone());
    if app.db.get_live_bot(&to).ok().flatten().is_none() {
        return;
    }
    let sender = crate::messaging::daemon_sender();
    let dm = crate::messaging::Dm::new(&to, &sender, bus::MessageKind::Note, &body);
    if let Err(error) = crate::messaging::send_dm(&app.db, &app.events, dm) {
        tracing::warn!(bot_id = %to, %error, "couldn't tell the author about its worktree");
    }
}

/// A PR's cleanup jobs, for `pr_get`.
pub fn jobs_json(app: &AppState, pr_id: &str) -> Value {
    let jobs = app
        .db
        .board_read(|t| t.cleanup_jobs_of_pr(pr_id))
        .unwrap_or_default();
    jobs.iter().map(Job::to_json).collect()
}
