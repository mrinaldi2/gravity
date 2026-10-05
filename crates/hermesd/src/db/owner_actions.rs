//! Owner action storage (H-117 R1). Content fields are written once; the
//! migration's trigger refuses changing them. A run claims its action with
//! one guarded update, so two taps can't both run it.

use chrono::{DateTime, Utc};
use rusqlite::{params, OptionalExtension, Row};
use serde_json::Value;

use crate::owner_action::model::{OwnerAction, Pinned, Proposal, Shell, State};

use super::{parse_ts, ts, Db};

const COLUMNS: &str = "id, project_id, proposed_by, item_id, decision_id, target_machine, shell, \
     cwd, content, pinned_files, reason, timeout_s, sha256, flags, origin, state, created_at, \
     expires_at, run_by, run_at, finished_at, exit_code, output_path, output_tail, reject_reason";

fn bad(text: &str) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(
        0,
        rusqlite::types::Type::Text,
        format!("unknown value {text}").into(),
    )
}

fn row(r: &Row<'_>) -> rusqlite::Result<OwnerAction> {
    let shell: String = r.get(6)?;
    let state: String = r.get(15)?;
    let pinned: Vec<Pinned> = serde_json::from_str(&r.get::<_, String>(9)?).unwrap_or_default();
    let at = |i: usize| -> rusqlite::Result<Option<DateTime<Utc>>> {
        Ok(r.get::<_, Option<String>>(i)?.map(|t| parse_ts(&t)))
    };
    Ok(OwnerAction {
        id: r.get(0)?,
        proposal: Proposal {
            project_id: r.get(1)?,
            proposed_by: r.get(2)?,
            item_id: r.get(3)?,
            decision_id: r.get(4)?,
            target_machine: r.get(5)?,
            shell: Shell::parse(&shell).ok_or_else(|| bad(&shell))?,
            cwd: r.get(7)?,
            content: r.get(8)?,
            pinned_files: pinned,
            reason: r.get(10)?,
            timeout_s: r.get(11)?,
        },
        sha256: r.get(12)?,
        flags: serde_json::from_str(&r.get::<_, String>(13)?).unwrap_or_default(),
        origin: r.get(14)?,
        state: State::parse(&state).ok_or_else(|| bad(&state))?,
        created_at: parse_ts(&r.get::<_, String>(16)?),
        expires_at: parse_ts(&r.get::<_, String>(17)?),
        run_by: r.get(18)?,
        run_at: at(19)?,
        finished_at: at(20)?,
        exit_code: r.get(21)?,
        output_path: r.get(22)?,
        output_tail: r.get(23)?,
        reject_reason: r.get(24)?,
    })
}

impl Db {
    pub fn insert_owner_action(&self, a: &OwnerAction) -> anyhow::Result<()> {
        let p = &a.proposal;
        self.lock().execute(
            "INSERT INTO owner_action (id, project_id, proposed_by, item_id, decision_id,
                 target_machine, shell, cwd, content, pinned_files, reason, timeout_s, sha256,
                 flags, origin, state, created_at, expires_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16,
                     ?17, ?18)",
            params![
                a.id,
                p.project_id,
                p.proposed_by,
                p.item_id,
                p.decision_id,
                p.target_machine,
                p.shell.as_str(),
                p.cwd,
                p.content,
                serde_json::to_string(&p.pinned_files)?,
                p.reason,
                p.timeout_s,
                a.sha256,
                serde_json::to_string(&a.flags)?,
                a.origin,
                a.state.as_str(),
                ts(a.created_at),
                ts(a.expires_at),
            ],
        )?;
        Ok(())
    }

    pub fn get_owner_action(&self, id: &str) -> anyhow::Result<Option<OwnerAction>> {
        let sql = format!("SELECT {COLUMNS} FROM owner_action WHERE id = ?1");
        Ok(self.lock().query_row(&sql, params![id], row).optional()?)
    }

    /// Newest first; one project's, or every project's.
    pub fn list_owner_actions(
        &self,
        project_id: Option<&str>,
        limit: usize,
    ) -> anyhow::Result<Vec<OwnerAction>> {
        let sql = format!(
            "SELECT {COLUMNS} FROM owner_action WHERE (?1 IS NULL OR project_id = ?1)
             ORDER BY created_at DESC LIMIT ?2"
        );
        let conn = self.lock();
        let rows = conn
            .prepare(&sql)?
            .query_map(params![project_id, limit as i64], row)?
            .collect::<Result<_, _>>()?;
        Ok(rows)
    }

    /// Claims a proposed, unexpired action for one run: `true` for the one
    /// caller that wins; the sha256 must be the stored one.
    pub fn claim_owner_action(
        &self,
        id: &str,
        sha256: &str,
        run_by: &str,
        now: DateTime<Utc>,
    ) -> anyhow::Result<bool> {
        let changed = self.lock().execute(
            "UPDATE owner_action SET state = 'running', run_by = ?3, run_at = ?4
             WHERE id = ?1 AND sha256 = ?2 AND state = 'proposed' AND expires_at > ?4",
            params![id, sha256, run_by, ts(now)],
        )?;
        Ok(changed == 1)
    }

    pub fn finish_owner_action(
        &self,
        id: &str,
        state: State,
        exit_code: Option<i32>,
        output_path: Option<&str>,
        output_tail: &str,
        now: DateTime<Utc>,
    ) -> anyhow::Result<()> {
        self.lock().execute(
            "UPDATE owner_action SET state = ?2, exit_code = ?3, output_path = ?4,
                 output_tail = ?5, finished_at = ?6 WHERE id = ?1 AND state = 'running'",
            params![
                id,
                state.as_str(),
                exit_code,
                output_path,
                output_tail,
                ts(now)
            ],
        )?;
        Ok(())
    }

    /// A proposed action closes without running: withdrawn, rejected or
    /// expired. `false` when it wasn't proposed any more.
    pub fn close_owner_action(
        &self,
        id: &str,
        state: State,
        reason: Option<&str>,
        now: DateTime<Utc>,
    ) -> anyhow::Result<bool> {
        let changed = self.lock().execute(
            "UPDATE owner_action SET state = ?2, reject_reason = ?3, finished_at = ?4
             WHERE id = ?1 AND state = 'proposed'",
            params![id, state.as_str(), reason, ts(now)],
        )?;
        Ok(changed == 1)
    }

    /// Every proposal past its 24 hours becomes expired.
    pub fn expire_owner_actions(&self, now: DateTime<Utc>) -> anyhow::Result<Vec<String>> {
        let conn = self.lock();
        let ids: Vec<String> = conn
            .prepare("SELECT id FROM owner_action WHERE state = 'proposed' AND expires_at <= ?1")?
            .query_map(params![ts(now)], |r| r.get(0))?
            .collect::<Result<_, _>>()?;
        conn.execute(
            "UPDATE owner_action SET state = 'expired', finished_at = ?1
             WHERE state = 'proposed' AND expires_at <= ?1",
            params![ts(now)],
        )?;
        Ok(ids)
    }

    pub fn audit_owner_action(
        &self,
        action_id: &str,
        actor: &str,
        event: &str,
        detail: &Value,
    ) -> anyhow::Result<()> {
        self.lock().execute(
            "INSERT INTO owner_action_audit (action_id, at, actor, event, detail)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![action_id, ts(Utc::now()), actor, event, detail.to_string()],
        )?;
        Ok(())
    }

    /// `(at, actor, event)` of an action's audit, oldest first.
    pub fn owner_action_audit(
        &self,
        action_id: &str,
    ) -> anyhow::Result<Vec<(String, String, String)>> {
        let conn = self.lock();
        let rows = conn
            .prepare(
                "SELECT at, actor, event FROM owner_action_audit WHERE action_id = ?1 ORDER BY seq",
            )?
            .query_map(params![action_id], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?))
            })?
            .collect::<Result<_, _>>()?;
        Ok(rows)
    }
}
