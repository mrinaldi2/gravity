//! Checks per commit and the tools each computer has (H-261 §1.5, §1.6), in
//! the board's transaction: a head's checks are queued in the same commit
//! that records the head.

use std::collections::BTreeMap;

use bus::now;
use rusqlite::{params, OptionalExtension, Row};

use super::board_tx::BoardTx;
use super::{parse_ts, ts};
use crate::prs::check_model::{CheckResult, CheckRun, NewCheck, Report};

pub(super) const COLUMNS: &str =
    "id, project_id, repo, sha, tree, name, run, needs, machine, required,
    result, note, runner, ran_on, log_artifact, tool_versions, queued_at, started_at,
    finished_at";

pub(super) fn check_row(r: &Row<'_>) -> rusqlite::Result<CheckRun> {
    let opt_ts = |i: usize| -> rusqlite::Result<_> {
        Ok(r.get::<_, Option<String>>(i)?.as_deref().map(parse_ts))
    };
    let result: String = r.get(10)?;
    Ok(CheckRun {
        id: r.get(0)?,
        project_id: r.get(1)?,
        repo: r.get(2)?,
        sha: r.get(3)?,
        tree: r.get(4)?,
        name: r.get(5)?,
        run: r.get(6)?,
        needs: serde_json::from_str(&r.get::<_, String>(7)?).unwrap_or_default(),
        machine: r.get(8)?,
        required: r.get(9)?,
        result: CheckResult::parse(&result).unwrap_or(CheckResult::Error),
        note: r.get(11)?,
        runner: r.get(12)?,
        ran_on: r.get(13)?,
        log_artifact: r.get(14)?,
        tool_versions: serde_json::from_str(&r.get::<_, String>(15)?).unwrap_or_default(),
        queued_at: parse_ts(&r.get::<_, String>(16)?),
        started_at: opt_ts(17)?,
        finished_at: opt_ts(18)?,
        tree_of: None,
    })
}

impl BoardTx<'_> {
    /// Records the head's checks. One already recorded for this commit stays
    /// as it is, so reporting the same head again re-queues nothing.
    pub fn queue_checks(
        &self,
        project_id: &str,
        repo: &str,
        sha: &str,
        tree: &str,
        checks: &[NewCheck],
    ) -> anyhow::Result<()> {
        let at = ts(now());
        for c in checks {
            let finished = c.result.is_final().then_some(&at);
            self.conn.execute(
                "INSERT INTO check_run(id, project_id, repo, sha, tree, name, run, needs,
                    machine, required, result, note, queued_at, finished_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)
                 ON CONFLICT(project_id, sha, name) DO NOTHING",
                params![
                    uuid::Uuid::new_v4().to_string(),
                    project_id,
                    repo,
                    sha,
                    tree,
                    c.name,
                    c.run,
                    serde_json::to_string(&c.needs)?,
                    c.machine,
                    c.required,
                    c.result.as_str(),
                    c.note,
                    at,
                    finished,
                ],
            )?;
        }
        Ok(())
    }

    /// The check as recorded on this very commit.
    pub fn check_run(
        &self,
        project_id: &str,
        sha: &str,
        name: &str,
    ) -> anyhow::Result<Option<CheckRun>> {
        Ok(self
            .conn
            .query_row(
                &format!(
                    "SELECT {COLUMNS} FROM check_run
                     WHERE project_id = ?1 AND sha = ?2 AND name = ?3"
                ),
                params![project_id, sha, name],
                check_row,
            )
            .optional()?)
    }

    /// The commit's checks as they count (§1.5): each one's own result, or a
    /// pass of the same check on another commit with the same tree, marked
    /// with that commit in `tree_of`.
    pub fn checks_on(&self, project_id: &str, sha: &str) -> anyhow::Result<Vec<CheckRun>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {COLUMNS} FROM check_run WHERE project_id = ?1 AND sha = ?2 ORDER BY name"
        ))?;
        let own = stmt
            .query_map(params![project_id, sha], check_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        own.into_iter()
            .map(|run| match run.result {
                CheckResult::Pass => Ok(run),
                _ => Ok(self.same_tree_pass(&run)?.unwrap_or(run)),
            })
            .collect()
    }

    fn same_tree_pass(&self, run: &CheckRun) -> anyhow::Result<Option<CheckRun>> {
        let other = self
            .conn
            .query_row(
                &format!(
                    "SELECT {COLUMNS} FROM check_run
                     WHERE project_id = ?1 AND repo = ?2 AND tree = ?3 AND name = ?4
                       AND sha != ?5 AND result = 'pass'
                     ORDER BY finished_at DESC LIMIT 1"
                ),
                params![run.project_id, run.repo, run.tree, run.name, run.sha],
                check_row,
            )
            .optional()?;
        Ok(other.map(|pass| CheckRun {
            tree_of: Some(pass.sha.clone()),
            sha: run.sha.clone(),
            id: run.id.clone(),
            ..pass
        }))
    }

    /// Names the worker a queued check is dispatched to: the only bot whose
    /// report is accepted. Returns false when it isn't queued any more.
    pub fn dispatch_check(&self, id: &str, runner: &str) -> anyhow::Result<bool> {
        let n = self.conn.execute(
            "UPDATE check_run SET runner = ?2 WHERE id = ?1 AND result = 'queued'",
            params![id, runner],
        )?;
        Ok(n == 1)
    }

    /// Whether `bot` opened or pushed a PR whose head is `sha`: such a bot
    /// never runs its checks (§7).
    pub fn wrote_head(&self, project_id: &str, sha: &str, bot: &str) -> anyhow::Result<bool> {
        Ok(self.conn.query_row(
            "SELECT EXISTS (SELECT 1 FROM pr p WHERE p.project_id = ?1 AND p.head_sha = ?2
                AND (p.author = ?3 OR EXISTS (
                    SELECT 1 FROM pr_push s WHERE s.pr_id = p.id AND s.pushed_by = ?3)))",
            params![project_id, sha, bot],
            |r| r.get(0),
        )?)
    }

    /// Records the runner's report: running sets when it started, a final
    /// result when it finished.
    pub fn report_check(&self, id: &str, report: &Report<'_>) -> anyhow::Result<()> {
        let at = ts(now());
        let finished = report.result.is_final().then_some(&at);
        self.conn.execute(
            "UPDATE check_run SET result = ?2, ran_on = ?3,
                log_artifact = COALESCE(?4, log_artifact), tool_versions = ?5,
                started_at = COALESCE(started_at, ?6), finished_at = ?7
             WHERE id = ?1",
            params![
                id,
                report.result.as_str(),
                report.ran_on,
                report.log_artifact,
                serde_json::to_string(report.tool_versions)?,
                at,
                finished,
            ],
        )?;
        Ok(())
    }

    /// A computer's probed tools (§1.6) replace what it reported before.
    pub fn set_machine_tools(
        &self,
        machine: &str,
        tools: &BTreeMap<String, String>,
    ) -> anyhow::Result<()> {
        let at = ts(now());
        self.conn.execute(
            "DELETE FROM machine_tool WHERE machine = ?1",
            params![machine],
        )?;
        for (tool, version) in tools {
            self.conn.execute(
                "INSERT INTO machine_tool(machine, tool, version, seen_at) VALUES (?1, ?2, ?3, ?4)",
                params![machine, tool, version, at],
            )?;
        }
        Ok(())
    }

    /// The tools a computer last reported, by name.
    pub fn machine_tools(&self, machine: &str) -> anyhow::Result<BTreeMap<String, String>> {
        let mut stmt = self
            .conn
            .prepare("SELECT tool, version FROM machine_tool WHERE machine = ?1")?;
        let tools = stmt
            .query_map(params![machine], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<rusqlite::Result<BTreeMap<String, String>>>()?;
        Ok(tools)
    }
}
