//! The owner's chat typed into a bot's composer (H-195 D2b, architect S2):
//! which prompts spent a typed token. A turn they start is the owner's chat
//! because the daemon typed it for an owner message, not because of its text.

use std::collections::HashSet;

use rusqlite::params;

use super::{ts, Db};

impl Db {
    /// `bot_id`'s prompt with `digest` was typed for owner message
    /// `message_id`.
    pub fn record_typed_prompt(
        &self,
        bot_id: &str,
        digest: &str,
        message_id: &str,
    ) -> anyhow::Result<()> {
        self.lock().execute(
            "INSERT OR IGNORE INTO typed_prompt (bot_id, digest, message_id, at)
             VALUES (?1, ?2, ?3, ?4)",
            params![bot_id, digest, message_id, ts(chrono::Utc::now())],
        )?;
        Ok(())
    }

    /// The digests of every prompt typed into `bot_id` for the owner.
    pub fn typed_prompt_digests(&self, bot_id: &str) -> anyhow::Result<HashSet<String>> {
        let conn = self.lock();
        let mut stmt = conn.prepare("SELECT digest FROM typed_prompt WHERE bot_id = ?1")?;
        let rows = stmt.query_map(params![bot_id], |r| r.get(0))?;
        Ok(rows.collect::<Result<_, _>>()?)
    }
}

#[cfg(test)]
mod tests {
    use crate::db::tests::{setup, user_sender};

    #[test]
    fn a_typed_prompt_is_found_by_its_bot_and_digest() {
        let (db, bot) = setup();
        let conv = db.dm_conversation(&bot.id).unwrap().unwrap();
        let msg = db
            .insert_message(
                &conv.id,
                &user_sender(),
                bus::MessageKind::Chat,
                "hi",
                None,
                None,
            )
            .unwrap();
        db.record_typed_prompt(&bot.id, "d1", &msg.id).unwrap();
        db.record_typed_prompt(&bot.id, "d1", &msg.id).unwrap();
        assert!(db.typed_prompt_digests(&bot.id).unwrap().contains("d1"));
        assert!(db.typed_prompt_digests("other").unwrap().is_empty());
    }
}
