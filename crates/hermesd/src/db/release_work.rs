//! A release's REL work card (H-247, UX-048 §6): the card DevOps works the
//! release on, where its owner Run cards hang. Set explicitly, or found by
//! the `REL-<version>` title the team names it with.

use bus::now;
use rusqlite::{params, Connection, OptionalExtension};

use super::board_tx::BoardTx;
use super::ts;

pub(super) fn work_item_in(
    conn: &Connection,
    release_id: &str,
) -> rusqlite::Result<Option<String>> {
    conn.query_row(
        "SELECT item_id FROM release_work_item WHERE release_id = ?1",
        params![release_id],
        |r| r.get(0),
    )
    .optional()
}

/// `title` names release `version`: `REL-0.17.5: …` or `REL 0.17.5 …`, not
/// `REL-0.17.50`.
fn names_version(title: &str, version: &str) -> bool {
    ["REL-", "REL "].iter().any(|prefix| {
        title
            .strip_prefix(prefix)
            .and_then(|rest| rest.strip_prefix(version))
            .is_some_and(|rest| !rest.starts_with(|c: char| c.is_ascii_digit() || c == '.'))
    })
}

impl BoardTx<'_> {
    /// Sets (or moves) a release's work card.
    pub fn set_release_work_item(
        &self,
        release_id: &str,
        item_id: &str,
        set_by: &str,
    ) -> anyhow::Result<()> {
        self.conn.execute(
            "INSERT INTO release_work_item(release_id, item_id, set_by, at)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(release_id) DO UPDATE SET
                 item_id = excluded.item_id, set_by = excluded.set_by, at = excluded.at",
            params![release_id, item_id, set_by, ts(now())],
        )?;
        Ok(())
    }

    /// The project's card named for release `version` (UX-048 §6d): a title
    /// starting `REL-<version>` or `REL <version>`; the newest when several.
    pub fn rel_card_named(
        &self,
        project_id: &str,
        version: &str,
    ) -> anyhow::Result<Option<String>> {
        let rows: Vec<(String, String)> = self
            .conn
            .prepare(
                "SELECT id, title FROM item
                 WHERE project_id = ?1 AND (title LIKE 'REL-%' OR title LIKE 'REL %')
                 ORDER BY created_at DESC",
            )?
            .query_map(params![project_id], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<Result<_, _>>()?;
        Ok(rows
            .into_iter()
            .find(|(_, title)| names_version(title, version))
            .map(|(id, _)| id))
    }
}

/// One owner-only thing on a card, as stored: what the release lists.
#[derive(Debug, Clone)]
pub struct OnCard {
    pub id: String,
    pub title: String,
    pub item_id: String,
    pub by: String,
    /// An owner action's target daemon.
    pub target: Option<String>,
    pub created_at: String,
}

fn on_card(r: &rusqlite::Row<'_>) -> rusqlite::Result<OnCard> {
    Ok(OnCard {
        id: r.get(0)?,
        title: r.get(1)?,
        item_id: r.get(2)?,
        by: r.get(3)?,
        target: r.get(4)?,
        created_at: r.get(5)?,
    })
}

impl BoardTx<'_> {
    /// Run cards waiting for the owner on any of `cards` (JSON array of ids).
    pub fn proposed_actions_on(
        &self,
        project_id: &str,
        cards: &str,
    ) -> anyhow::Result<Vec<OnCard>> {
        Ok(self
            .conn
            .prepare(
                "SELECT id, reason, item_id, proposed_by, target_machine, created_at
                 FROM owner_action
                 WHERE state = 'proposed' AND expires_at > ?3
                   AND coalesce(local_project_id, project_id) = ?1
                   AND item_id IN (SELECT value FROM json_each(?2))
                 ORDER BY created_at",
            )?
            .query_map(params![project_id, cards, ts(now())], on_card)?
            .collect::<Result<_, _>>()?)
    }

    /// Decisions still with the owner (open or held) on any of `cards`: by
    /// their own card or an `item_link kind=decision` on one.
    pub fn open_decisions_on(&self, project_id: &str, cards: &str) -> anyhow::Result<Vec<OnCard>> {
        Ok(self
            .conn
            .prepare(
                "SELECT id, title, card, raised_by_bot_id, NULL, created_at FROM (
                     SELECT d.*, coalesce(
                         CASE WHEN d.item_id IN (SELECT value FROM json_each(?2))
                              THEN d.item_id END,
                         (SELECT l.item_id FROM item_link l
                          WHERE l.kind = 'decision' AND l.ref = d.id
                            AND l.item_id IN (SELECT value FROM json_each(?2))
                          ORDER BY l.at LIMIT 1)) AS card
                     FROM decision d
                     WHERE d.project_id = ?1 AND d.state IN ('open', 'held'))
                 WHERE card IS NOT NULL
                 ORDER BY created_at",
            )?
            .query_map(params![project_id, cards], on_card)?
            .collect::<Result<_, _>>()?)
    }

    /// Every release still open, with its project.
    pub fn open_release_ids(&self) -> anyhow::Result<Vec<(String, String)>> {
        Ok(self
            .conn
            .prepare(
                "SELECT id, project_id FROM release
                 WHERE status NOT IN ('superseded', 'deployed', 'rejected', 'rolled_back')
                 ORDER BY created_at",
            )?
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<Result<_, _>>()?)
    }

    /// The cards of a bot's open tasks.
    pub fn open_task_cards(&self, bot_id: &str) -> anyhow::Result<Vec<String>> {
        Ok(self
            .conn
            .prepare(
                "SELECT DISTINCT item_id FROM task
                 WHERE to_bot_id = ?1 AND state = 'open' AND item_id IS NOT NULL",
            )?
            .query_map(params![bot_id], |r| r.get(0))?
            .collect::<Result<_, _>>()?)
    }
}

#[cfg(test)]
mod tests {
    use super::names_version;

    #[test]
    fn a_rel_title_names_its_version_only() {
        assert!(names_version(
            "REL-0.17.5: assemble, verify, sign",
            "0.17.5"
        ));
        assert!(names_version("REL 0.17.5 desktop", "0.17.5"));
        assert!(names_version("REL-0.17.5", "0.17.5"));
        assert!(!names_version("REL-0.17.50: later", "0.17.5"));
        assert!(!names_version("REL-0.17.5.1", "0.17.5"));
        assert!(!names_version("Fix REL-0.17.5", "0.17.5"));
    }
}
