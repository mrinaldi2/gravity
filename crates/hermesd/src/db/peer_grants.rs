//! Grants waiting for a linked computer (H-163): an owner's ruling granting
//! extras to a bot that runs elsewhere, kept until that computer applies or
//! refuses them.

use chrono::Utc;
use rusqlite::{params, Row};

use super::{ts, Db};

/// One ruling's grants for one linked bot.
#[derive(Debug, Clone, PartialEq)]
pub struct PeerGrant {
    pub id: String,
    pub decision_id: String,
    /// The stand-in on this computer.
    pub bot_id: String,
    pub peer_id: String,
    pub extras: Vec<String>,
    pub attempts: u32,
}

/// What became of a grant the linked computer answered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GrantEnd {
    Applied,
    Refused,
}

fn row(r: &Row<'_>) -> rusqlite::Result<PeerGrant> {
    let extras: String = r.get(4)?;
    Ok(PeerGrant {
        id: r.get(0)?,
        decision_id: r.get(1)?,
        bot_id: r.get(2)?,
        peer_id: r.get(3)?,
        extras: serde_json::from_str(&extras).unwrap_or_default(),
        attempts: r.get(5)?,
    })
}

impl Db {
    /// Keeps a grant for `peer_id` until it applies or is refused.
    pub fn insert_peer_grant(
        &self,
        decision_id: &str,
        bot_id: &str,
        peer_id: &str,
        extras: &[String],
    ) -> anyhow::Result<PeerGrant> {
        let grant = PeerGrant {
            id: uuid::Uuid::new_v4().to_string(),
            decision_id: decision_id.to_string(),
            bot_id: bot_id.to_string(),
            peer_id: peer_id.to_string(),
            extras: extras.to_vec(),
            attempts: 0,
        };
        let now = ts(Utc::now());
        self.lock().execute(
            "INSERT INTO peer_grant(id, decision_id, bot_id, peer_id, extras, state, attempts,
                                    created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, 'pending', 0, ?6, ?6)",
            params![
                grant.id,
                grant.decision_id,
                grant.bot_id,
                grant.peer_id,
                serde_json::to_string(&grant.extras)?,
                now
            ],
        )?;
        Ok(grant)
    }

    /// The grants still waiting for `peer_id`, oldest first; every peer's
    /// when `peer_id` is None.
    pub fn pending_peer_grants(&self, peer_id: Option<&str>) -> anyhow::Result<Vec<PeerGrant>> {
        let conn = self.lock();
        let mut stmt = conn.prepare(
            "SELECT id, decision_id, bot_id, peer_id, extras, attempts FROM peer_grant
             WHERE state = 'pending' AND (?1 IS NULL OR peer_id = ?1)
             ORDER BY created_at",
        )?;
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

    /// The linked computer answered: applied, or refused for good. False
    /// when another sweep closed it first, so only one of them says so.
    pub fn end_peer_grant(
        &self,
        id: &str,
        end: GrantEnd,
        error: Option<&str>,
    ) -> anyhow::Result<bool> {
        let state = match end {
            GrantEnd::Applied => "applied",
            GrantEnd::Refused => "refused",
        };
        let changed = self.lock().execute(
            "UPDATE peer_grant SET state = ?2, last_error = ?3, attempts = attempts + 1,
                                   updated_at = ?4
             WHERE id = ?1 AND state = 'pending'",
            params![id, state, error, ts(Utc::now())],
        )?;
        Ok(changed == 1)
    }
}
