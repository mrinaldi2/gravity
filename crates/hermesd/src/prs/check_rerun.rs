//! A check's next run (H-283, H-261 §7): a re-run asked for by the owner,
//! the lead or the PR's author; one automatic retry after an `error`, never
//! after a `fail`; and an `error` for a job that ended without a result (a
//! restart cut it short, or its computer never answered), which counts as
//! any error.

use std::sync::Arc;

use crate::app::AppState;
use crate::check_exec;
use crate::db::check_jobs::JobCause;
use crate::decisions::{conflict, forbidden, invalid, not_found};
use crate::prs::check_jobs;
use crate::prs::check_model::{CheckResult, CheckRun, Report};
use crate::prs::checks::short;

/// Who asks for a re-run.
pub enum Asker<'a> {
    /// From a device or ticket connection.
    Owner,
    Bot {
        bot: &'a bus::Bot,
        lead: bool,
    },
}

/// Queues `name` on `sha` again, once it has a result.
pub fn rerun(
    app: &Arc<AppState>,
    project: &str,
    sha: &str,
    name: &str,
    asker: &Asker<'_>,
) -> anyhow::Result<CheckRun> {
    let run = app.db.board_tx(|t| {
        let run = t
            .check_run(project, sha, name)?
            .ok_or_else(|| not_found(format!("no check {name} on {}", short(sha))))?;
        let who = match asker {
            Asker::Owner => "the owner".to_string(),
            Asker::Bot { bot, lead: true } => bot.name.clone(),
            Asker::Bot { bot, lead: false } => {
                let author = t.pr_of_head(project, sha)?.map(|p| p.1);
                if author.as_deref() != Some(bot.id.as_str()) {
                    return Err(forbidden(
                        "only the owner, the lead or the PR's author re-runs a check",
                    ));
                }
                bot.name.clone()
            }
        };
        if run.run.is_empty() {
            return Err(invalid(format!(
                "{name} has nothing to run; fix the base's checks.toml instead"
            )));
        }
        if !run.result.is_final() {
            return Err(conflict(format!(
                "{name} on {} is {} already",
                short(sha),
                run.result.as_str()
            )));
        }
        t.end_check_jobs(&run.id)?;
        t.requeue_check(&run.id, &format!("re-run asked for by {who}"))?;
        t.pend_check_job(&run, JobCause::Rerun)?;
        t.check_run(project, sha, name)?
            .ok_or_else(|| not_found(format!("no check {name}")))
    })?;
    check_jobs::nudge();
    Ok(run)
}

/// After a result: a final one ends the check's job, and an `error` is
/// retried once, unless it came from that retry.
pub fn after_report(app: &AppState, run: &CheckRun) -> anyhow::Result<()> {
    if !run.result.is_final() {
        return Ok(());
    }
    app.db.board_tx(|t| settle(t, run))?;
    check_jobs::nudge();
    Ok(())
}

fn settle(t: &crate::db::BoardTx<'_>, run: &CheckRun) -> anyhow::Result<()> {
    let cause = t.last_job_cause(&run.id)?;
    t.end_check_jobs(&run.id)?;
    if run.result == CheckResult::Error && cause.is_some() && cause != Some(JobCause::Auto) {
        let why = run.note.as_deref().unwrap_or("it errored");
        t.requeue_check(&run.id, &format!("retrying once after an error: {why}"))?;
        t.pend_check_job(run, JobCause::Auto)?;
    }
    Ok(())
}

/// Jobs that will never report: one open here that this daemon isn't
/// running (a restart cut it short), or one on another computer with no
/// result long after the longest a check may run. Each is an `error`,
/// retried once like any.
pub fn reconcile(app: &AppState, project: &str) -> anyhow::Result<()> {
    let here = app
        .db
        .board_read(crate::board::release::machines::this_computer)?;
    let patience = chrono::Duration::from_std(check_exec::DEADLINE)? + chrono::Duration::hours(1);
    for job in app.db.board_read(|t| t.open_jobs(project))? {
        let why = if job.machine == here {
            if check_jobs::is_running_here(&job.id) {
                continue;
            }
            "the daemon stopped while it ran".to_string()
        } else if bus::now() - job.created_at > patience {
            format!("{} sent no result within 7 hours", job.machine)
        } else {
            continue;
        };
        app.db.board_tx(|t| {
            let Some(run) = t.check_by_id(&job.check_id)? else {
                return Ok(());
            };
            if run.result.is_final() {
                return t.end_check_jobs(&run.id);
            }
            t.report_check(
                &run.id,
                &Report {
                    result: CheckResult::Error,
                    ran_on: &job.machine,
                    log_artifact: None,
                    tool_versions: &run.tool_versions,
                },
            )?;
            t.set_check_note(&run.id, Some(&why))?;
            let run = t.check_by_id(&run.id)?.expect("just reported");
            settle(t, &run)
        })?;
    }
    Ok(())
}
