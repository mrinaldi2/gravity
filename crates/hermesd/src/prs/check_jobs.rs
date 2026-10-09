//! The check runner (H-283, H-261 §7). Each queued check is routed to a
//! computer whose tools cover it, and a worker is spawned for it there,
//! pinned to that computer, with the PR's card. The worker gets a fresh
//! checkout at the exact sha in its workspace (made by that computer's
//! daemon, removed once it reports), runs the check and reports it. The
//! runner is always a new worker: never the PR's author or a pusher.
//!
//! Each computer runs at most `jobs_per_machine` check jobs at once, and a
//! job doesn't start on a computer whose disk is under the floor.

use std::sync::{Arc, OnceLock};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::sync::Notify;

use crate::app::AppState;
use crate::board::model::Role;
use crate::board::release::machines;
use crate::db::NewWorker;
use crate::prs::check_checkout;
use crate::prs::check_model::CheckRun;
use crate::prs::check_route::{needs_of, route, Limits, Machine, Route};
use crate::prs::checks::{dispatch as dispatch_check, short};
use crate::prs::repo;
use crate::workers::{self, HERE};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ChecksConfig {
    /// Check jobs one computer runs at once, every project's (the Mac
    /// takes at most 2 Rust builds).
    pub jobs_per_machine: usize,
    /// No job starts on a computer with less free disk than this, in GB.
    pub disk_floor_gb: u64,
    /// How often each computer probes its tools, in seconds; 0 never.
    pub probe_interval_secs: u64,
    /// How often queued checks are routed when nothing nudges sooner, in
    /// seconds; 0 routes them only when asked (tests).
    pub dispatch_interval_secs: u64,
}

impl Default for ChecksConfig {
    fn default() -> Self {
        Self {
            jobs_per_machine: 2,
            disk_floor_gb: 20,
            probe_interval_secs: 3600,
            dispatch_interval_secs: 30,
        }
    }
}

/// How long a check worker's task may run.
const DEADLINE_HOURS: i64 = 6;

const INSTRUCTIONS: &str = "You run one check and report it with check_report, then \
complete_task. Never edit, commit or push anything.";

fn wake() -> &'static Notify {
    static WAKE: OnceLock<Notify> = OnceLock::new();
    WAKE.get_or_init(Notify::new)
}

/// Route queued checks soon: one was queued, a job ended, tools changed.
pub fn nudge() {
    wake().notify_one();
}

/// Routes the queued checks of every board held here, now and whenever
/// nudged, at least every `dispatch_interval_secs`.
pub fn spawn(app: Arc<AppState>) {
    if app.cfg.checks.dispatch_interval_secs == 0 {
        return;
    }
    let every = Duration::from_secs(app.cfg.checks.dispatch_interval_secs);
    tokio::spawn(async move {
        loop {
            for project in home_projects(&app).unwrap_or_default() {
                if let Err(error) = dispatch(&app, &project).await {
                    tracing::warn!(project, %error, "routing checks failed");
                }
            }
            let _ = tokio::time::timeout(every, wake().notified()).await;
        }
    });
}

/// The projects whose board lives on this computer.
fn home_projects(app: &AppState) -> anyhow::Result<Vec<String>> {
    let me = app.db.daemon_id()?;
    let mut out = Vec::new();
    for project in app.db.list_projects()? {
        if app
            .db
            .board_settings(&project.id)?
            .is_some_and(|s| s.home_daemon_id == me)
        {
            out.push(project.id);
        }
    }
    Ok(out)
}

/// Routes `project`'s queued checks and spawns a worker for each one that
/// has a computer.
pub async fn dispatch(app: &Arc<AppState>, project: &str) -> anyhow::Result<()> {
    // One pass at a time, or two would route the same check twice.
    static DISPATCHING: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    let _one = DISPATCHING.lock().await;
    super::check_rerun::reconcile(app, project)?;
    let checks = app.db.board_read(|t| t.undispatched_checks(project))?;
    if checks.is_empty() {
        return Ok(());
    }
    let Some(lead) = lead_here(app, project)? else {
        for check in &checks {
            note(
                app,
                check,
                "waiting for a lead on the board's computer to spawn its worker",
            );
        }
        return Ok(());
    };
    let mut machines = inventory(app, project)?;
    let limits = Limits {
        jobs_per_machine: app.cfg.checks.jobs_per_machine.max(1),
        disk_floor_gb: app.cfg.checks.disk_floor_gb,
    };
    for check in &checks {
        let needs = needs_of(&check.needs, &check.run);
        match route(&needs, check.machine.as_deref(), &machines, limits) {
            Route::Wait(why) => note(app, check, &why),
            Route::To(name) => {
                let Some(m) = machines.iter_mut().find(|m| m.name == name) else {
                    continue;
                };
                match queue(app, &lead, check, m).await {
                    Ok(()) => m.open_jobs += 1,
                    Err(error) => {
                        note(app, check, &format!("couldn't spawn its worker: {error:#}"))
                    }
                }
            }
        }
    }
    workers::place_queued(app, project).await;
    Ok(())
}

/// Why a check still waits, shown with it (`check_updated` pushes come
/// with H-273's read surface).
fn note(app: &AppState, check: &CheckRun, why: &str) {
    if let Err(error) = app.db.board_tx(|t| t.set_check_note(&check.id, Some(why))) {
        tracing::warn!(check = %check.name, %error, "check note not kept");
    }
}

/// The bot check workers are spawned under: the project's lead, when it
/// runs on this computer.
fn lead_here(app: &AppState, project: &str) -> anyhow::Result<Option<bus::Bot>> {
    let Some(p) = app.db.get_project(project)? else {
        return Ok(None);
    };
    let id = match p.lead_bot_id {
        Some(id) => Some(id),
        None => app
            .db
            .project_roles(project)?
            .into_iter()
            .find(|r| r.role == Role::Lead)
            .map(|r| r.bot_id),
    };
    let Some(id) = id else { return Ok(None) };
    Ok(app
        .db
        .get_live_bot(&id)?
        .filter(|b| !b.is_linked() && !b.temporary))
}

/// Every computer that reported its tools, as routing sees it: this one,
/// and the linked ones this project may run workers on.
fn inventory(app: &AppState, project: &str) -> anyhow::Result<Vec<Machine>> {
    let (here, reported, open) = app.db.board_read(|t| {
        let here = machines::this_computer(t)?;
        let reported = t.all_machine_tools()?;
        let open = reported
            .iter()
            .map(|(m, _)| t.open_jobs_on(m))
            .collect::<anyhow::Result<Vec<_>>>()?;
        Ok((here, reported, open))
    })?;
    let mut out = Vec::new();
    for ((name, tools), open_jobs) in reported.into_iter().zip(open) {
        let is_here = name == here;
        let online = is_here || peer_for(app, project, &name).is_some();
        out.push(Machine {
            name,
            tools,
            open_jobs,
            here: is_here,
            online,
        });
    }
    Ok(out)
}

/// The linked, online peer named `name` that `project` may run workers on.
fn peer_for(app: &AppState, project: &str, name: &str) -> Option<bus::Peer> {
    let peer = app.db.get_peer_by_name(name).ok()??;
    let linked = app.db.project_link(project, &peer.id).ok()?.is_some();
    (peer.revoked_at.is_none() && linked && app.peers.is_online(&peer.id)).then_some(peer)
}

/// Queues the worker for `check` on `machine`, with its job.
async fn queue(
    app: &Arc<AppState>,
    lead: &bus::Bot,
    check: &CheckRun,
    machine: &Machine,
) -> anyhow::Result<()> {
    let url = repo::of_project(app, &check.project_id, Some(&check.repo))?.url;
    let pr = app
        .db
        .board_read(|t| t.pr_of_head(&check.project_id, &check.sha))?;
    let name = worker_name(app, check)?;
    let brief = brief(check, &url, pr.as_ref().map(|p| p.2));
    let description = format!("Runs check {} on {}", check.name, short(&check.sha));
    let new = NewWorker {
        project_id: &check.project_id,
        parent_bot_id: &lead.id,
        name: &name,
        brief: &brief,
        description: &description,
        instructions: INSTRUCTIONS,
        runtime: None,
        machine: Some(if machine.here { HERE } else { &machine.name }),
        deadline_hours: DEADLINE_HOURS,
    };
    workers::queue_with(app, &new, |worker| {
        app.db.board_tx(|t| {
            t.assign_check_job(check, &machine.name, &worker.id)?;
            t.set_check_note(&check.id, None).map(|_| ())
        })?;
        if let Some((card, _, _)) = &pr {
            app.db.set_worker_card(&worker.id, card)?;
            app.db.set_worker_item(&worker.id, card)?;
        }
        Ok(())
    })
    .await?;
    tracing::info!(check = %check.name, sha = short(&check.sha), machine = %machine.name, "check routed");
    Ok(())
}

/// `check-<name>-<sha7>`, numbered when that is taken.
fn worker_name(app: &Arc<AppState>, check: &CheckRun) -> anyhow::Result<String> {
    let slug: String = check
        .name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    let base = format!("check-{}-{}", slug.trim_matches('-'), short(&check.sha));
    (1..=50)
        .map(|n| {
            if n == 1 {
                base.clone()
            } else {
                format!("{base}-{n}")
            }
        })
        .find(|name| crate::botmgmt::validate_name(app, &check.project_id, name, None).is_ok())
        .ok_or_else(|| anyhow::anyhow!("no free name for {base}"))
}

/// What the worker is told to do.
pub fn brief(check: &CheckRun, url: &str, pr: Option<u32>) -> String {
    let pr = pr.map(|n| format!(" (PR #{n})")).unwrap_or_default();
    let (dir, failed) = (check_checkout::DIR, check_checkout::FAILED);
    format!(
        "Run check `{name}` on commit {sha} of {repo}{pr}.\n\n\
         1. Wait until `{dir}/` exists in your workspace: the daemon is checking that commit \
         out there for you. If `{failed}` appears instead, write it to `check.log` and report \
         `error` with that log (step 4).\n\
         2. check_report sha={sha} name={name} result=running.\n\
         3. In `{dir}/` run, with all output going to `check.log` in your workspace (not inside \
         `{dir}/`): `cd {dir} && ({run}) > ../check.log 2>&1`\n\
         4. check_report sha={sha} name={name} with result `pass` if it exited 0, `fail` if it \
         didn't, or `error` only if it couldn't run at all (a missing tool, a broken checkout); \
         log=check.log, and tool_versions with the version of each tool it used.\n\
         5. complete_task with one line: the result. Don't fix, commit or push anything.",
        name = check.name,
        sha = check.sha,
        repo = repo::name_of(url),
        run = check.run,
    )
}

// ---- placement (workers/place.rs) ----

/// Before a check worker is created here: this computer's disk floor.
pub fn before_start_here(app: &AppState, worker: &bus::Worker) -> anyhow::Result<()> {
    if app
        .db
        .board_read(|t| t.job_for_worker(&worker.id))?
        .is_some()
    {
        check_checkout::disk_floor(&app.cfg.home, app.cfg.checks.disk_floor_gb)?;
    }
    Ok(())
}

/// Once the worker's bot exists, here or as a peer's stand-in: the check is
/// dispatched to it, the only bot whose report counts.
pub fn claim(app: &AppState, worker: &bus::Worker, bot: &bus::Bot) -> anyhow::Result<()> {
    let Some((job, check)) = app.db.board_read(|t| t.job_for_worker(&worker.id))? else {
        return Ok(());
    };
    if check.runner.as_deref() != Some(bot.id.as_str()) {
        dispatch_check(app, &check.project_id, &check.sha, &check.name, &bot.id)?;
    }
    app.db.board_tx(|t| t.set_job_bot(&job.id, &bot.id))
}

/// A check worker created here: claimed, and its checkout started.
pub fn started_here(app: &AppState, worker: &bus::Worker, bot: &bus::Bot) -> anyhow::Result<()> {
    claim(app, worker, bot)?;
    let Some((_, check)) = app.db.board_read(|t| t.job_for_worker(&worker.id))? else {
        return Ok(());
    };
    let url = repo::of_project(app, &check.project_id, Some(&check.repo))?.url;
    check_checkout::prepare_in_background(bot.workspace_path.clone().into(), url, check.sha);
    Ok(())
}

/// What a peer needs to make a check worker's checkout: `check` in its
/// `create_bot` frame.
pub fn for_peer(app: &AppState, worker: &bus::Worker) -> anyhow::Result<Option<Value>> {
    let Some((_, check)) = app.db.board_read(|t| t.job_for_worker(&worker.id))? else {
        return Ok(None);
    };
    let url = repo::of_project(app, &check.project_id, Some(&check.repo))?.url;
    Ok(Some(
        json!({ "url": url, "sha": check.sha, "name": check.name }),
    ))
}

/// A peer asks for a check worker here (`check` in its `create_bot` frame):
/// the repository must be one this project has, cloned from this side's own
/// URL for it, and the disk must be above the floor, else the asker waits.
pub fn peer_check(
    app: &AppState,
    project: &str,
    frame: &Value,
) -> anyhow::Result<Option<(String, String)>> {
    let Some(check) = frame.get("check").filter(|c| c.is_object()) else {
        return Ok(None);
    };
    let asked = check["url"].as_str().unwrap_or_default();
    let sha = check["sha"].as_str().unwrap_or_default().to_string();
    let own = app.db.project_repo(project)?.map(|r| r.url);
    let url = own
        .into_iter()
        .chain(app.db.extra_repos(project)?)
        .find(|url| repo::same(url, asked))
        .ok_or_else(|| anyhow::anyhow!("{asked} isn't one of this project's repositories"))?;
    check_checkout::disk_floor(&app.cfg.home, app.cfg.checks.disk_floor_gb)
        .map_err(|low| crate::peer::refuse("at_capacity", low.to_string()))?;
    Ok(Some((url, sha)))
}
