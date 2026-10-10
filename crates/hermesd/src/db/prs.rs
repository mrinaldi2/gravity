//! Pull requests, their pushes and worktrees (H-261 §1.1, §1.2, §15.1), in
//! the board's transaction: opening or closing one moves its card in the
//! same commit.

use bus::now;
use rusqlite::{params, OptionalExtension, Row};

use super::board_tx::BoardTx;
use super::{parse_ts, ts};
use crate::prs::model::{Pr, PrPush, PrState, PrWorktree};

const COLUMNS: &str = "id, project_id, number, repo, item_id, branch, base, base_sha, head_sha,
    head_patch_id, remote_sha, moved_unreported, state, author, title, change_note,
    owner_flagged, owner_flag_reason, merged_sha, merged_at, close_reason, opened_at,
    closed_at, updated_at, version";

fn pr_row(r: &Row<'_>) -> rusqlite::Result<Pr> {
    let state: String = r.get(12)?;
    let opt_ts = |i: usize| -> rusqlite::Result<_> {
        Ok(r.get::<_, Option<String>>(i)?.as_deref().map(parse_ts))
    };
    Ok(Pr {
        id: r.get(0)?,
        project_id: r.get(1)?,
        number: r.get(2)?,
        repo: r.get(3)?,
        item_id: r.get(4)?,
        branch: r.get(5)?,
        base: r.get(6)?,
        base_sha: r.get(7)?,
        head_sha: r.get(8)?,
        head_patch_id: r.get(9)?,
        remote_sha: r.get(10)?,
        moved_unreported: r.get(11)?,
        state: PrState::parse(&state).unwrap_or(PrState::Closed),
        author: r.get(13)?,
        title: r.get(14)?,
        change_note: r.get(15)?,
        owner_flagged: r.get(16)?,
        owner_flag_reason: r.get(17)?,
        merged_sha: r.get(18)?,
        merged_at: opt_ts(19)?,
        close_reason: r.get(20)?,
        opened_at: parse_ts(&r.get::<_, String>(21)?),
        closed_at: opt_ts(22)?,
        updated_at: parse_ts(&r.get::<_, String>(23)?),
        version: r.get::<_, i64>(24)? as u64,
    })
}

/// A new PR's facts, all verified by the caller.
pub struct NewPr<'a> {
    pub project_id: &'a str,
    pub repo: &'a str,
    pub item_id: &'a str,
    pub branch: &'a str,
    pub base_sha: &'a str,
    pub head_sha: &'a str,
    pub patch_id: &'a str,
    pub author: &'a str,
    pub title: &'a str,
    pub change_note: &'a str,
}

/// A verified head: the reported tip, or one seen without a report.
pub struct Head<'a> {
    pub sha: &'a str,
    pub patch_id: &'a str,
    pub base_sha: &'a str,
    /// The reporter; `None` = it moved without a report.
    pub pushed_by: Option<&'a str>,
}

impl BoardTx<'_> {
    pub fn pr(&self, project_id: &str, number: u32) -> anyhow::Result<Option<Pr>> {
        Ok(self
            .conn
            .query_row(
                &format!("SELECT {COLUMNS} FROM pr WHERE project_id = ?1 AND number = ?2"),
                params![project_id, number],
                pr_row,
            )
            .optional()?)
    }

    pub fn pr_by_id(&self, id: &str) -> anyhow::Result<Option<Pr>> {
        Ok(self
            .conn
            .query_row(
                &format!("SELECT {COLUMNS} FROM pr WHERE id = ?1"),
                params![id],
                pr_row,
            )
            .optional()?)
    }

    /// The PR that reverted this one for a Leave out (H-272), by number.
    pub fn reverted_by(&self, pr_id: &str) -> anyhow::Result<Option<u32>> {
        Ok(self
            .conn
            .query_row(
                "SELECT p.number FROM pr_reverted v JOIN pr p ON p.id = v.revert_pr_id
                 WHERE v.pr_id = ?1",
                params![pr_id],
                |r| r.get(0),
            )
            .optional()?)
    }

    /// The card's open or merging PR in `repo`, if it has one: a card has
    /// at most one per repo (H-261 §6.5).
    pub fn live_pr_of(&self, item_id: &str, repo: &str) -> anyhow::Result<Option<Pr>> {
        Ok(self
            .conn
            .query_row(
                &format!(
                    "SELECT {COLUMNS} FROM pr WHERE item_id = ?1 AND repo = ?2
                       AND state IN ('open', 'merging')"
                ),
                params![item_id, repo],
                pr_row,
            )
            .optional()?)
    }

    /// Every PR the card ever had, in every repo, oldest first.
    pub fn prs_of_item(&self, item_id: &str) -> anyhow::Result<Vec<Pr>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {COLUMNS} FROM pr WHERE item_id = ?1 ORDER BY number"
        ))?;
        let rows = stmt
            .query_map(params![item_id], pr_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// The project's PRs in these states (every state when empty), open
    /// first, then newest.
    pub fn prs(&self, project_id: &str, states: &[PrState]) -> anyhow::Result<Vec<Pr>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {COLUMNS} FROM pr WHERE project_id = ?1
             ORDER BY state NOT IN ('open', 'merging'), number DESC"
        ))?;
        let all = stmt
            .query_map(params![project_id], pr_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(all
            .into_iter()
            .filter(|p| states.is_empty() || states.contains(&p.state))
            .collect())
    }

    /// Records a new PR as #n, the project's next number, with its head
    /// pushed by its author.
    pub fn insert_pr(&self, new: &NewPr<'_>) -> anyhow::Result<Pr> {
        let number: u32 = self.conn.query_row(
            "SELECT COALESCE(MAX(number), 0) + 1 FROM pr WHERE project_id = ?1",
            params![new.project_id],
            |r| r.get(0),
        )?;
        let id = uuid::Uuid::new_v4().to_string();
        let at = ts(now());
        self.conn.execute(
            "INSERT INTO pr(id, project_id, number, repo, item_id, branch, base, base_sha,
                head_sha, head_patch_id, remote_sha, author, title, change_note, opened_at,
                updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'main', ?7, ?8, ?9, ?8, ?10, ?11, ?12, ?13, ?13)",
            params![
                id,
                new.project_id,
                number,
                new.repo,
                new.item_id,
                new.branch,
                new.base_sha,
                new.head_sha,
                new.patch_id,
                new.author,
                new.title,
                new.change_note,
                at
            ],
        )?;
        self.add_pr_push(&id, new.head_sha, new.patch_id, Some(new.author))?;
        self.note_pr_id(&id)?;
        self.pr(new.project_id, number)?
            .ok_or_else(|| anyhow::anyhow!("PR #{number} vanished"))
    }

    fn add_pr_push(
        &self,
        pr_id: &str,
        sha: &str,
        patch_id: &str,
        pushed_by: Option<&str>,
    ) -> anyhow::Result<()> {
        self.conn.execute(
            "INSERT INTO pr_push(pr_id, sha, patch_id, pushed_by, at) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![pr_id, sha, patch_id, pushed_by, ts(now())],
        )?;
        self.note_pr_id(pr_id)
    }

    /// A reported head becomes the PR's head; it is no longer unreported.
    pub fn set_pr_head(&self, pr: &Pr, head: &Head<'_>) -> anyhow::Result<()> {
        self.conn.execute(
            "UPDATE pr SET head_sha = ?2, head_patch_id = ?3, base_sha = ?4, remote_sha = ?2,
                moved_unreported = 0, version = version + 1, updated_at = ?5
             WHERE id = ?1",
            params![pr.id, head.sha, head.patch_id, head.base_sha, ts(now())],
        )?;
        self.add_pr_push(&pr.id, head.sha, head.patch_id, head.pushed_by)
    }

    /// The branch moved to `tip` with no report: the head stays, the move is
    /// recorded once and attributed to no one (§1.2).
    pub fn set_pr_moved(&self, pr: &Pr, tip: &str, patch_id: &str) -> anyhow::Result<()> {
        self.conn.execute(
            "UPDATE pr SET remote_sha = ?2, moved_unreported = 1, version = version + 1,
                updated_at = ?3
             WHERE id = ?1",
            params![pr.id, tip, ts(now())],
        )?;
        self.add_pr_push(&pr.id, tip, patch_id, None)
    }

    /// The PR's author becomes `by` (H-272 M1: a bot pushed to the owner's
    /// revert, so it is that bot's change now).
    pub fn set_pr_author(&self, pr: &Pr, by: &str) -> anyhow::Result<()> {
        self.conn.execute(
            "UPDATE pr SET author = ?2, version = version + 1, updated_at = ?3 WHERE id = ?1",
            params![pr.id, by, ts(now())],
        )?;
        Ok(())
    }

    /// Main fast-forwarded to `sha`, the PR's head, by `by` (H-284).
    pub fn set_pr_merged(&self, pr: &Pr, sha: &str, by: &str) -> anyhow::Result<()> {
        let at = ts(now());
        self.conn.execute(
            "UPDATE pr SET state = 'merged', merged_sha = ?2, merged_at = ?3, merged_by = ?4,
                closed_at = ?3, version = version + 1, updated_at = ?3
             WHERE id = ?1",
            params![pr.id, sha, at, by],
        )?;
        Ok(())
    }

    pub fn close_pr(&self, pr: &Pr, reason: &str) -> anyhow::Result<()> {
        let at = ts(now());
        self.conn.execute(
            "UPDATE pr SET state = 'closed', close_reason = ?2, closed_at = ?3,
                version = version + 1, updated_at = ?3
             WHERE id = ?1",
            params![pr.id, reason, at],
        )?;
        self.note_pr(pr);
        Ok(())
    }

    /// Projects with an open or merging PR.
    pub fn projects_with_live_prs(&self) -> anyhow::Result<Vec<String>> {
        let mut stmt = self
            .conn
            .prepare("SELECT DISTINCT project_id FROM pr WHERE state IN ('open', 'merging')")?;
        let ids = stmt
            .query_map([], |r| r.get(0))?
            .collect::<rusqlite::Result<Vec<String>>>()?;
        Ok(ids)
    }

    pub fn pr_pushes(&self, pr_id: &str) -> anyhow::Result<Vec<PrPush>> {
        let mut stmt = self.conn.prepare(
            "SELECT sha, patch_id, pushed_by, at FROM pr_push WHERE pr_id = ?1 ORDER BY id",
        )?;
        let pushes = stmt
            .query_map(params![pr_id], |r| {
                Ok(PrPush {
                    sha: r.get(0)?,
                    patch_id: r.get(1)?,
                    pushed_by: r.get(2)?,
                    at: parse_ts(&r.get::<_, String>(3)?),
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(pushes)
    }

    pub fn add_pr_worktree(&self, pr_id: &str, w: &PrWorktree) -> anyhow::Result<()> {
        self.conn.execute(
            "INSERT INTO pr_worktree(pr_id, machine, bot_id, path, main_clone, reported_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(pr_id, machine, path) DO UPDATE SET
                bot_id = excluded.bot_id, main_clone = excluded.main_clone,
                reported_at = excluded.reported_at",
            params![pr_id, w.machine, w.bot_id, w.path, w.main_clone, ts(now())],
        )?;
        self.note_pr_id(pr_id)
    }

    pub fn pr_worktrees(&self, pr_id: &str) -> anyhow::Result<Vec<PrWorktree>> {
        let mut stmt = self.conn.prepare(
            "SELECT machine, bot_id, path, main_clone FROM pr_worktree WHERE pr_id = ?1
             ORDER BY reported_at",
        )?;
        let rows = stmt
            .query_map(params![pr_id], |r| {
                Ok(PrWorktree {
                    machine: r.get(0)?,
                    bot_id: r.get(1)?,
                    path: r.get(2)?,
                    main_clone: r.get(3)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }
}
