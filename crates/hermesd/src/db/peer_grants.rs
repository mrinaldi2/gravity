//! Grants waiting for a linked computer (H-163): an owner's ruling granting
//! extras to a bot that runs elsewhere, kept until that computer applies or
//! refuses them, bound to the ruling it came from (CE-020 M1). And on the
//! bot's own computer, when the owner last changed its extras there (M2).

use chrono::{DateTime, Utc};
use rusqlite::{params, OptionalExtension, Row};

use super::{parse_ts, ts, Db};

/// One ruling's grants for one linked bot.
#[derive(Debug, Clone, PartialEq)]
pub struct PeerGrant {
    pub id: String,
    pub decision_id: String,
    /// The sha of the option the ruling picked, as `grants::sha` gives it.
    pub grants_sha: String,
    /// When the owner ruled; the bot's computer refuses it if its own owner
    /// changed the bot's extras since.
    pub decided_at: DateTime<Utc>,
    /// The stand-in on this computer.
    pub bot_id: String,
    pub peer_id: String,
    pub extras: Vec<String>,
    pub attempts: u32,
    pub created_at: DateTime<Utc>,
}

/// How a grant ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GrantEnd {
    Applied,
    Refused,
    /// The ruling no longer holds: settled differently, reopened, withdrawn
    /// or superseded.
    Cancelled,
    /// Not delivered within a day.
    Expired,
}

/// The ruling a grant came from, as `insert_peer_grant` keeps it.
pub struct GrantRuling<'a> {
    pub decision_id: &'a str,
    pub grants_sha: &'a str,
    pub decided_at: DateTime<Utc>,
}

const COLUMNS: &str =
    "id, decision_id, grants_sha, decided_at, bot_id, peer_id, extras, attempts, created_at";

fn row(r: &Row<'_>) -> rusqlite::Result<PeerGrant> {
    let extras: String = r.get(6)?;
    Ok(PeerGrant {
        id: r.get(0)?,
        decision_id: r.get(1)?,
        grants_sha: r.get(2)?,
        decided_at: parse_ts(&r.get::<_, String>(3)?),
        bot_id: r.get(4)?,
        peer_id: r.get(5)?,
        extras: serde_json::from_str(&extras).unwrap_or_default(),
        attempts: r.get(7)?,
        created_at: parse_ts(&r.get::<_, String>(8)?),
    })
}

impl Db {
    /// Keeps a grant for `peer_id` until it applies, is refused, or the
    /// ruling it came from no longer holds.
    pub fn insert_peer_grant(
        &self,
        ruling: &GrantRuling<'_>,
        bot_id: &str,
        peer_id: &str,
        extras: &[String],
    ) -> anyhow::Result<()> {
        let now = ts(Utc::now());
        self.lock().execute(
            "INSERT INTO peer_grant(id, decision_id, grants_sha, decided_at, bot_id, peer_id,
                                    extras, state, attempts, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'pending', 0, ?8, ?8)",
            params![
                uuid::Uuid::new_v4().to_string(),
                ruling.decision_id,
                ruling.grants_sha,
                ts(ruling.decided_at),
                bot_id,
                peer_id,
                serde_json::to_string(extras)?,
                now
            ],
        )?;
        Ok(())
    }

    /// The grants still waiting for `peer_id`, oldest first; every peer's
    /// when `peer_id` is None.
    pub fn pending_peer_grants(&self, peer_id: Option<&str>) -> anyhow::Result<Vec<PeerGrant>> {
        let conn = self.lock();
        let sql = format!(
            "SELECT {COLUMNS} FROM peer_grant
             WHERE state = 'pending' AND (?1 IS NULL OR peer_id = ?1) ORDER BY created_at"
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt
            .query_map(params![peer_id], row)?
            .collect::<Result<_, _>>()?;
        Ok(rows)
    }

    /// One more try that didn't reach the linked computer; the count so far.
    pub fn retry_peer_grant(&self, id: &str, error: &str) -> anyhow::Result<u32> {
        let conn = self.lock();
        conn.execute(
            "UPDATE peer_grant SET attempts = attempts + 1, last_error = ?2, updated_at = ?3
             WHERE id = ?1 AND state = 'pending'",
            params![id, error, ts(Utc::now())],
        )?;
        Ok(conn.query_row(
            "SELECT attempts FROM peer_grant WHERE id = ?1",
            params![id],
            |r| r.get(0),
        )?)
    }

    /// The grant is over. False when another sweep ended it first, so only
    /// one of them says so on the decision.
    pub fn end_peer_grant(
        &self,
        id: &str,
        end: GrantEnd,
        error: Option<&str>,
    ) -> anyhow::Result<bool> {
        let state = match end {
            GrantEnd::Applied => "applied",
            GrantEnd::Refused => "refused",
            GrantEnd::Cancelled => "cancelled",
            GrantEnd::Expired => "expired",
        };
        let changed = self.lock().execute(
            "UPDATE peer_grant SET state = ?2, last_error = ?3, attempts = attempts + 1,
                                   updated_at = ?4
             WHERE id = ?1 AND state = 'pending'",
            params![id, state, error, ts(Utc::now())],
        )?;
        Ok(changed == 1)
    }

    /// Notes that the owner changed the bot's extras on this computer now.
    pub fn stamp_extras_changed(&self, bot_id: &str) -> anyhow::Result<()> {
        self.lock().execute(
            "INSERT INTO bot_extras_changed(bot_id, changed_at) VALUES (?1, ?2)
             ON CONFLICT(bot_id) DO UPDATE SET changed_at = excluded.changed_at",
            params![bot_id, ts(Utc::now())],
        )?;
        Ok(())
    }

    /// When the owner last changed the bot's extras on this computer.
    pub fn extras_changed_at(&self, bot_id: &str) -> anyhow::Result<Option<DateTime<Utc>>> {
        Ok(self
            .lock()
            .query_row(
                "SELECT changed_at FROM bot_extras_changed WHERE bot_id = ?1",
                params![bot_id],
                |r| r.get::<_, String>(0),
            )
            .optional()?
            .map(|t| parse_ts(&t)))
    }
}
