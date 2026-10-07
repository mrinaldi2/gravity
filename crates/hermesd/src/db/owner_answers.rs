//! Answers taken from a bot's session into its owner thread (H-192): one
//! row per turn whose final text was posted, so a turn is posted at most once.

use std::collections::HashMap;

use bus::now;
use rusqlite::params;

use super::{ts, Db};

impl Db {
    /// Claims `turn_id` for posting: true when it wasn't claimed before.
    pub fn claim_owner_answer(&self, bot_id: &str, turn_id: &str) -> anyhow::Result<bool> {
        let conn = self.lock();
        let added = conn.execute(
            "INSERT OR IGNORE INTO owner_answer (bot_id, turn_id, posted_at) VALUES (?1, ?2, ?3)",
            params![bot_id, turn_id, ts(now())],
        )?;
        Ok(added == 1)
    }

    /// Gives a claim back when its post failed, so a later pass retries.
    pub fn release_owner_answer(&self, bot_id: &str, turn_id: &str) -> anyhow::Result<()> {
        self.lock().execute(
            "DELETE FROM owner_answer
              WHERE bot_id = ?1 AND turn_id = ?2 AND message_num IS NULL",
            params![bot_id, turn_id],
        )?;
        Ok(())
    }

    /// The thread message the turn's answer became.
    pub fn set_owner_answer_num(
        &self,
        bot_id: &str,
        turn_id: &str,
        num: i64,
    ) -> anyhow::Result<()> {
        self.lock().execute(
            "UPDATE owner_answer SET message_num = ?3 WHERE bot_id = ?1 AND turn_id = ?2",
            params![bot_id, turn_id, num],
        )?;
        Ok(())
    }

    /// The bot's posted answers: turn id → thread message number.
    pub fn owner_answers(&self, bot_id: &str) -> anyhow::Result<HashMap<String, i64>> {
        let conn = self.lock();
        let mut stmt = conn.prepare(
            "SELECT turn_id, message_num FROM owner_answer
              WHERE bot_id = ?1 AND message_num IS NOT NULL",
        )?;
        let rows = stmt.query_map(params![bot_id], |r| Ok((r.get(0)?, r.get(1)?)))?;
        Ok(rows.collect::<Result<_, _>>()?)
    }
}
