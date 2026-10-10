//! Cleanup jobs after a merge (H-261 §15.1, CL-1), in the board's
//! transaction: the board's home keeps every computer's rows.

use bus::now;
use chrono::{DateTime, Utc};
use rusqlite::{params, OptionalExtension, Row};

use super::board_tx::BoardTx;
use super::{parse_ts, ts};
use crate::cleanup::model::{Job, JobState, Kind, NewJob};

const COLUMNS: &str = "id, project_id, pr_id, machine, kind, path_or_ref, main_clone, bot_id,
    state, reason, bytes_freed, attempts, busy_since, next_at, sent_at, at";

fn job_row(r: &Row<'_>) -> rusqlite::Result<Job> {
    let opt_ts = |i: usize| -> rusqlite::Result<_> {
        Ok(r.get::<_, Option<String>>(i)?.as_deref().map(parse_ts))
    };
    let kind: String = r.get(4)?;
    let state: String = r.get(8)?;
    Ok(Job {
        id: r.get(0)?,
        project_id: r.get(1)?,
        pr_id: r.get(2)?,
        machine: r.get(3)?,
        kind: Kind::parse(&kind).unwrap_or(Kind::Worktree),
        path_or_ref: r.get(5)?,
        main_clone: r.get(6)?,
        bot_id: r.get(7)?,
        state: JobState::parse(&state).unwrap_or(JobState::Failed),
        reason: r.get(9)?,
        bytes_freed: r.get::<_, i64>(10)?.max(0) as u64,
        attempts: r.get(11)?,
        busy_since: opt_ts(12)?,
        next_at: opt_ts(13)?,
        sent_at: opt_ts(14)?,
        at: parse_ts(&r.get::<_, String>(15)?),
    })
}

impl BoardTx<'_> {
    /// Queues `job` unless the same thing is already queued or recorded for
    /// that PR and computer: its id when it was added.
    pub fn add_cleanup_job(&self, job: &NewJob) -> anyhow::Result<Option<String>> {
        let id = uuid::Uuid::new_v4().to_string();
        let added = self.conn.execute(
            "INSERT INTO cleanup_job(id, project_id, pr_id, machine, kind, path_or_ref,
                 main_clone, bot_id, at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
             ON CONFLICT(pr_id, machine, kind, path_or_ref) DO NOTHING",
            params![
                id,
                job.project_id,
                job.pr_id,
                job.machine,
                job.kind.as_str(),
                job.path_or_ref,
                job.main_clone,
                job.bot_id,
                ts(now())
            ],
        )?;
        Ok((added == 1).then_some(id))
    }

    pub fn cleanup_job(&self, id: &str) -> anyhow::Result<Option<Job>> {
        Ok(self
            .conn
            .query_row(
                &format!("SELECT {COLUMNS} FROM cleanup_job WHERE id = ?1"),
                params![id],
                job_row,
            )
            .optional()?)
    }

    /// The jobs still queued whose retry time has come, oldest first.
    pub fn cleanup_jobs_due(&self, at: DateTime<Utc>) -> anyhow::Result<Vec<Job>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {COLUMNS} FROM cleanup_job
             WHERE state = 'queued' AND (next_at IS NULL OR next_at <= ?1) ORDER BY at, id"
        ))?;
        let rows = stmt
            .query_map(params![ts(at)], job_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn cleanup_jobs_of_pr(&self, pr_id: &str) -> anyhow::Result<Vec<Job>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {COLUMNS} FROM cleanup_job WHERE pr_id = ?1 ORDER BY at, id"
        ))?;
        let rows = stmt
            .query_map(params![pr_id], job_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// A linked computer was asked to run the job at `at`.
    pub fn set_cleanup_sent(&self, id: &str, at: DateTime<Utc>) -> anyhow::Result<()> {
        self.conn.execute(
            "UPDATE cleanup_job SET sent_at = ?2 WHERE id = ?1",
            params![id, ts(at)],
        )?;
        Ok(())
    }

    /// An attempt found the tree in use: tried again at `next_at`.
    pub fn set_cleanup_busy(
        &self,
        id: &str,
        reason: &str,
        busy_since: DateTime<Utc>,
        next_at: DateTime<Utc>,
    ) -> anyhow::Result<()> {
        self.conn.execute(
            "UPDATE cleanup_job SET reason = ?2, busy_since = ?3, next_at = ?4, sent_at = NULL,
                 attempts = attempts + 1, at = ?5
             WHERE id = ?1 AND state = 'queued'",
            params![id, reason, ts(busy_since), ts(next_at), ts(now())],
        )?;
        Ok(())
    }

    /// The job's last word: done, held or failed. False when it already had
    /// one, so a repeated answer changes nothing.
    pub fn finish_cleanup_job(
        &self,
        id: &str,
        state: JobState,
        reason: &str,
        bytes: u64,
    ) -> anyhow::Result<bool> {
        let changed = self.conn.execute(
            "UPDATE cleanup_job SET state = ?2, reason = ?3, bytes_freed = ?4,
                 attempts = attempts + 1, next_at = NULL, at = ?5
             WHERE id = ?1 AND state = 'queued'",
            params![id, state.as_str(), reason, bytes as i64, ts(now())],
        )?;
        Ok(changed == 1)
    }
}
