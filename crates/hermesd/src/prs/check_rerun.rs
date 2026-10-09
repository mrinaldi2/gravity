//! A check's next run (H-283, H-261 §7): a re-run asked for by the owner,
//! the lead or the PR's author; one automatic retry after an `error`, never
//! after a `fail`; and an `error` for a job whose worker ended without a
//! result, which counts as any error.

use std::collections::BTreeMap;
use std::sync::Arc;

use crate::app::AppState;
use crate::db::check_jobs::JobCause;
use crate::decisions::{conflict, forbidden, invalid, not_found};
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
    super::check_jobs::nudge();
    Ok(run)
}

/// After a result: a final one ends the check's job, and an `error` is
/// retried once, unless it came from that retry.
pub fn after_report(app: &AppState, run: &CheckRun) -> anyhow::Result<()> {
    if !run.result.is_final() {
        return Ok(());
    }
    app.db.board_tx(|t| settle(t, run))?;
    super::check_jobs::nudge();
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

/// Jobs whose worker is gone: a check it never finished is an `error`,
/// retried once like any.
pub fn reconcile(app: &AppState, project: &str) -> anyhow::Result<()> {
    for job in app.db.board_read(|t| t.open_jobs(project))? {
        let Some(worker_id) = &job.worker_id else {
            continue;
        };
        let Some(worker) = app.db.get_worker(worker_id)? else {
            continue;
        };
        if !worker.state.is_final() {
            continue;
        }
        let why = format!(
            "its worker {} {} without a result{}",
            worker.name,
            worker.state.as_str(),
            worker.error.map(|e| format!(": {e}")).unwrap_or_default()
        );
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
                    tool_versions: &BTreeMap::new(),
                },
            )?;
            t.set_check_note(&run.id, Some(&why))?;
            let run = t.check_by_id(&run.id)?.expect("just reported");
            settle(t, &run)
        })?;
    }
    Ok(())
}
