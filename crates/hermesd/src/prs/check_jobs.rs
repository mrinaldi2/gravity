//! The check runner (H-283, H-261 §7). Each queued check is routed to a
//! computer whose tools cover it, and that computer's daemon runs it
//! itself ([`check_exec`]): a fresh checkout at the exact sha, the command
//! from the base's `checks.toml`, and a result from its exit status alone.
//! No bot and no AI worker is in the loop (ARCH M1, M2): the runner is the
//! daemon identity [`SYSTEM_RUNNER`], which has no inbox, no parent and
//! no task, and is never a PR's author or pusher.
//!
//! Each computer runs at most `jobs_per_machine` check jobs at once, and a
//! job doesn't start on a computer whose disk is under the floor.

use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::sync::Notify;

use crate::app::AppState;
use crate::board::release::machines;
use crate::check_exec::{self, Outcome, Spec};
use crate::prs::check_checkout;
use crate::prs::check_model::{CheckResult, CheckRun, Report};
use crate::prs::check_route::{needs_of, route, Limits, Machine, Route};
use crate::prs::checks::{self, short, RunLog};
use crate::prs::repo;

/// The runner every check is dispatched to: the daemon itself. Not a bot,
/// so nothing can message it, and never a PR's author or pusher.
pub const SYSTEM_RUNNER: &str = "daemon:check-runner";

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
    /// The longest a check may run before its whole tree is stopped and it
    /// is an `error`, in seconds.
    pub timeout_secs: u64,
    /// The `hermesd` that runs a check (`hermesd check run`); this
    /// daemon's own binary when unset.
    pub runner: Option<PathBuf>,
}

impl Default for ChecksConfig {
    fn default() -> Self {
        Self {
            jobs_per_machine: 2,
            disk_floor_gb: 20,
            probe_interval_secs: 3600,
            dispatch_interval_secs: 30,
            timeout_secs: check_exec::DEADLINE.as_secs(),
            runner: None,
        }
    }
}

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

/// At boot (H-291): nothing runs here yet, so every job still open on this
/// computer was cut short by the stop. Each is an `error`, retried once,
/// and any checkout a stop left behind goes.
pub fn on_boot(app: &AppState) {
    if let Ok(jobs) = std::fs::read_dir(app.cfg.home.join("run").join("checks")) {
        for job in jobs.flatten() {
            if let Err(error) = check_checkout::remove(&job.path()) {
                tracing::warn!(job = %job.path().display(), %error, "old check checkout not removed");
            }
        }
    }
    for project in home_projects(app).unwrap_or_default() {
        if let Err(error) = super::check_rerun::reconcile(app, &project) {
            tracing::warn!(project, %error, "check jobs not reconciled at boot");
        }
    }
    nudge();
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

/// Routes `project`'s queued checks and starts each one that has a
/// computer.
pub async fn dispatch(app: &Arc<AppState>, project: &str) -> anyhow::Result<()> {
    // One pass at a time, or two would route the same check twice.
    static DISPATCHING: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    let _one = DISPATCHING.lock().await;
    super::check_rerun::reconcile(app, project)?;
    let checks = app.db.board_read(|t| t.undispatched_checks(project))?;
    if checks.is_empty() {
        return Ok(());
    }
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
                match start(app, check, &needs, m).await {
                    Ok(()) => m.open_jobs += 1,
                    Err(error) => note(
                        app,
                        check,
                        &format!("waiting: {} didn't start it: {error:#}", m.name),
                    ),
                }
            }
        }
    }
    Ok(())
}

/// Why a check still waits, shown with it (`check_updated` pushes come
/// with H-273's read surface).
fn note(app: &AppState, check: &CheckRun, why: &str) {
    if let Err(error) = app.db.board_tx(|t| t.set_check_note(&check.id, Some(why))) {
        tracing::warn!(check = %check.name, %error, "check note not kept");
    }
}

/// Every computer that reported its tools, as routing sees it: this one,
/// and the linked ones this project may run checks on.
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

/// The linked, online peer named `name` that `project` may run checks on.
pub(super) fn peer_for(app: &AppState, project: &str, name: &str) -> Option<bus::Peer> {
    let peer = app.db.get_peer_by_name(name).ok()??;
    let linked = app.db.project_link(project, &peer.id).ok()?.is_some();
    (peer.revoked_at.is_none() && linked && app.peers.is_online(&peer.id)).then_some(peer)
}

/// The jobs this daemon is running now: a job open here that isn't one
/// was cut short by a restart.
fn running_here() -> &'static Mutex<HashSet<String>> {
    static RUNNING: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    RUNNING.get_or_init(Mutex::default)
}

pub(super) fn is_running_here(job: &str) -> bool {
    running_here()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .contains(job)
}

/// The probed versions of the tools `needs` names, as the run's record.
fn versions(machine: &Machine, needs: &[String]) -> BTreeMap<String, String> {
    needs
        .iter()
        .filter_map(|n| Some((n.clone(), machine.tools.get(n)?.clone())))
        .collect()
}

/// Starts `check` on `machine`: its job, the daemon as its runner, and the
/// run itself, here or on the linked computer.
async fn start(
    app: &Arc<AppState>,
    check: &CheckRun,
    needs: &[String],
    machine: &Machine,
) -> anyhow::Result<()> {
    if machine.here {
        check_checkout::disk_floor(&app.cfg.home, app.cfg.checks.disk_floor_gb)?;
    }
    let url = repo::of_project(app, &check.project_id, Some(&check.repo))?.url;
    let tools = versions(machine, needs);
    let job = app.db.board_tx(|t| {
        let job = t.assign_check_job(check, &machine.name)?;
        anyhow::ensure!(
            t.dispatch_check(&check.id, SYSTEM_RUNNER)?,
            "{} isn't queued any more",
            check.name
        );
        t.report_check(
            &check.id,
            &Report {
                result: CheckResult::Running,
                ran_on: &machine.name,
                log_artifact: None,
                tool_versions: &tools,
            },
        )?;
        t.set_check_note(&check.id, None)?;
        Ok(job)
    })?;
    let spec = Spec {
        url,
        sha: check.sha.clone(),
        run: check.run.clone(),
    };
    let started = if machine.here {
        running_here()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(job.clone());
        tokio::spawn(run_here(app.clone(), job.clone(), spec));
        Ok(())
    } else {
        super::check_remote::start(app, check, &machine.name, &job, &spec).await
    };
    if started.is_err() {
        app.db.board_tx(|t| {
            t.unassign_check_job(&job)?;
            t.undispatch_check(&check.id)
        })?;
    }
    started?;
    tracing::info!(check = %check.name, sha = short(&check.sha), machine = %machine.name, "check started");
    Ok(())
}

async fn run_here(app: Arc<AppState>, job: String, spec: Spec) {
    let outcome = check_exec::run(&app.cfg, &job, spec).await;
    let log = outcome.log.clone().map_or(RunLog::None, RunLog::Here);
    finish(&app, &job, &outcome, log);
    running_here()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(&job);
}

/// Records how job `job` went, if it is still open: its check's result,
/// then the retry an error gets.
pub(super) fn finish(app: &AppState, job: &str, outcome: &Outcome, log: RunLog) {
    let recorded = (|| -> anyhow::Result<()> {
        let Some(job) = app
            .db
            .board_read(|t| t.check_job(job))?
            .filter(|j| j.ended_at.is_none())
        else {
            return Ok(());
        };
        let run = checks::record(
            app,
            &job.check_id,
            &job.machine,
            outcome.result,
            &outcome.note,
            log,
        )?;
        super::check_rerun::after_report(app, &run)
    })();
    if let Err(error) = recorded {
        tracing::warn!(job, %error, "check result not recorded");
    }
    nudge();
}

/// The frame a job's result travels in from the computer that ran it.
pub(super) fn result_frame(job: &str, outcome: &Outcome) -> serde_json::Value {
    json!({
        "type": super::check_remote::RESULT,
        "job": job,
        "result": outcome.result.as_str(),
        "note": outcome.note,
        "log": outcome.log.as_ref().map(|l| l.display().to_string()),
    })
}
