//! Checks per commit (H-261 §7). When a head is reported, the base's
//! `checks.toml`, filtered by the paths the PR changes, says which checks it
//! must pass; each is queued on that commit. A pass or a fail comes only
//! from the exit status of the check's command, as the daemon's runner
//! captured it (H-283 ARCH M1); a bot may report only that it couldn't run.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use bus::contract::pr::CheckReport;

use crate::app::AppState;
use crate::board::policy::{base, parse_checks, Checks, CHECKS_PATH};
use crate::board::release::{git_cache, machines};
use crate::decisions::{conflict, forbidden, invalid, not_found};
use crate::prs::check_log;
use crate::prs::check_model::{CheckResult, CheckRun, NewCheck, Report};

/// The checks a head must pass, and the tree it holds.
#[derive(Debug, Clone, PartialEq)]
pub struct Required {
    pub tree: String,
    pub checks: Vec<NewCheck>,
}

/// The required checks for `head` under the `checks.toml` at `base_sha`,
/// for the paths `head` changes since it left `base_sha`. A base whose file
/// doesn't parse queues one `checks.toml` check as an error, so the head
/// can't count as checked by a broken policy, with one way out (H-270 ARCH
/// M1): a PR that changes `checks.toml` to a file that parses runs the
/// head's checks, plus a passing `checks.toml` check, so the repair can
/// merge. It still needs architect, ce and the owner, as any policy change.
pub fn required(cache: &Path, base_sha: &str, head: &str) -> anyhow::Result<Required> {
    let tree = git_cache::tree_of(cache, head)
        .ok_or_else(|| anyhow::anyhow!("{head} isn't in the repository"))?;
    let changed = git_cache::changed_paths(cache, base_sha, head)?;
    let checks = match base::read(cache, base_sha)?.checks {
        Ok(policy) => queue(&policy, &changed),
        Err(e) => match repair(cache, head, &changed)? {
            Some(policy) => {
                let mut checks = queue(&policy, &changed);
                checks.push(policy_check(
                    CheckResult::Pass,
                    "repairs the base's broken checks.toml; the head's checks run",
                ));
                checks
            }
            None => vec![policy_check(CheckResult::Error, &e)],
        },
    };
    Ok(Required { tree, checks })
}

fn queue(policy: &Checks, changed: &[String]) -> Vec<NewCheck> {
    policy
        .for_change(changed)
        .filter(|c| c.required)
        .map(|c| NewCheck {
            name: c.name.clone(),
            run: c.run.clone(),
            needs: c.needs.clone(),
            machine: c.machine.clone(),
            required: true,
            result: CheckResult::Queued,
            note: None,
        })
        .collect()
}

fn policy_check(result: CheckResult, note: &str) -> NewCheck {
    NewCheck {
        name: CHECKS_PATH.to_string(),
        run: String::new(),
        needs: Vec::new(),
        machine: None,
        required: true,
        result,
        note: Some(note.to_string()),
    }
}

/// The head's `checks.toml`, when the PR changes it and it parses.
fn repair(cache: &Path, head: &str, changed: &[String]) -> anyhow::Result<Option<Checks>> {
    if !changed.iter().any(|p| p == CHECKS_PATH) {
        return Ok(None);
    }
    Ok(git_cache::file_at(cache, head, CHECKS_PATH)?.and_then(|text| parse_checks(&text).ok()))
}

/// Dispatches a queued check to `runner`: the daemon's
/// [`SYSTEM_RUNNER`](crate::prs::check_jobs::SYSTEM_RUNNER) (H-283), or a
/// bot, which may then only report `running` or `error`. Never to a bot that
/// opened or pushed a PR with that head (§7).
pub fn dispatch(
    app: &AppState,
    project: &str,
    sha: &str,
    name: &str,
    runner: &str,
) -> anyhow::Result<()> {
    app.db.board_tx(|t| {
        let run = t
            .check_run(project, sha, name)?
            .ok_or_else(|| not_found(format!("no check {name} on {}", short(sha))))?;
        if t.wrote_head(project, sha, runner)? {
            return Err(forbidden(
                "a PR's author or pusher never runs its checks; dispatch it to another worker",
            ));
        }
        anyhow::ensure!(
            t.dispatch_check(&run.id, runner)?,
            conflict(format!(
                "{name} on {} is {}, not queued",
                short(sha),
                run.result.as_str()
            ))
        );
        Ok(())
    })
}

/// `check_report`: the bot a check was dispatched to says it started or
/// couldn't run. Pass and fail are refused: they come only from the
/// command's exit status, recorded by [`record`].
pub fn report(app: &Arc<AppState>, bot: &bus::Bot, req: &CheckReport) -> anyhow::Result<CheckRun> {
    let project = bot.project_id.as_str();
    let (sha, name) = (req.sha.trim(), req.name.trim());
    let result = CheckResult::from_wire(req.result)
        .filter(|r| *r != CheckResult::Queued)
        .ok_or_else(|| invalid("result must be running or error"))?;
    if matches!(result, CheckResult::Pass | CheckResult::Fail) {
        return Err(forbidden(
            "a pass or a fail comes only from the check's exit status, which the daemon's \
             runner (hermesd check run) records; report error if it couldn't run",
        ));
    }
    let run = app
        .db
        .board_read(|t| t.check_run(project, sha, name))?
        .ok_or_else(|| {
            not_found(format!(
                "no check {name} is recorded on {sha}; report the full sha you were spawned for"
            ))
        })?;
    match run.runner.as_deref() {
        None => {
            return Err(forbidden(format!(
                "{name} on {} hasn't been dispatched",
                short(sha)
            )))
        }
        Some(runner) if runner != bot.id => {
            return Err(forbidden(format!(
                "only the worker {name} on {} was dispatched to reports it",
                short(sha)
            )))
        }
        Some(_) => {}
    }
    if run.result.is_final() {
        return Err(conflict(format!(
            "{name} on {} already reported {}; a re-run dispatches it again",
            short(sha),
            run.result.as_str()
        )));
    }
    let log = req.log.as_deref().map(str::trim).filter(|l| !l.is_empty());
    let ran_on = ran_on(app, bot)?;
    let log_artifact = log
        .map(|path| check_log::publish(app, bot, &ran_on, &run, path))
        .transpose()?;
    let tool_versions: BTreeMap<String, String> = req.tool_versions.clone().into_iter().collect();
    app.db.board_tx(|t| {
        let now = t
            .check_run(project, sha, name)?
            .filter(|r| r.runner.as_deref() == Some(bot.id.as_str()) && !r.result.is_final())
            .ok_or_else(|| conflict(format!("{name} on {} changed; read it again", short(sha))))?;
        t.report_check(
            &now.id,
            &Report {
                result,
                ran_on: &ran_on,
                log_artifact: log_artifact.as_deref(),
                tool_versions: &tool_versions,
            },
        )?;
        t.check_run(project, sha, name)?
            .ok_or_else(|| not_found(format!("no check {name}")))
    })
}

/// Where a recorded run's log is.
pub enum RunLog {
    None,
    /// A file on this computer, published under the project's artifacts.
    Here(PathBuf),
    /// On another computer, kept there: `<computer>:<path>`.
    There(String),
}

/// A final result from the check's exit status, as the daemon's runner on
/// `ran_on` captured it (H-283 ARCH M1): the only way a check passes or
/// fails. The check must still be the runner's and not final.
pub fn record(
    app: &AppState,
    check_id: &str,
    ran_on: &str,
    result: CheckResult,
    note: &str,
    log: RunLog,
) -> anyhow::Result<CheckRun> {
    let runner = crate::prs::check_jobs::SYSTEM_RUNNER;
    let ours = |run: &CheckRun| run.runner.as_deref() == Some(runner) && !run.result.is_final();
    let run = app
        .db
        .board_read(|t| t.check_by_id(check_id))?
        .filter(ours)
        .ok_or_else(|| conflict("that check isn't the runner's to record any more"))?;
    anyhow::ensure!(result.is_final(), "a runner records a final result");
    let log_artifact = match log {
        RunLog::None => None,
        RunLog::Here(file) => Some(check_log::publish_file(app, &run, &file)?),
        RunLog::There(at) => Some(at),
    };
    app.db.board_tx(|t| {
        let now = t
            .check_by_id(check_id)?
            .filter(ours)
            .ok_or_else(|| conflict("that check changed while it ran"))?;
        t.report_check(
            &now.id,
            &Report {
                result,
                ran_on,
                log_artifact: log_artifact.as_deref(),
                tool_versions: &now.tool_versions,
            },
        )?;
        t.set_check_note(&now.id, Some(note))?;
        t.check_by_id(check_id)?
            .ok_or_else(|| not_found("no such check"))
    })
}

/// The computer the reporter runs on: a linked bot's own, else this one.
fn ran_on(app: &AppState, bot: &bus::Bot) -> anyhow::Result<String> {
    if let Some(peer) = &bot.peer_id {
        if let Some(peer) = app.db.get_peer(peer)? {
            return Ok(peer.name);
        }
    }
    app.db.board_read(machines::this_computer)
}

pub(super) fn short(sha: &str) -> &str {
    &sha[..sha.len().min(7)]
}
