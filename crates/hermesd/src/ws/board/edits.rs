//! The owner's edits from the item drawer (U4, H-018 §3.6): a comment,
//! written as the owner and pushed like a bot's, so open boards and drawers
//! see it.

use bus::contract::board as c;

use super::{not_found, refuse, Refusal};
use crate::board::feed::{card_after_commit, Change, ChangeKind};
use crate::ws::Conn;

impl Conn {
    pub(super) fn item_comment(&self, r: &c::ItemAddComment) -> Result<c::EditResult, Refusal> {
        let body = r.body.trim();
        if body.is_empty() {
            return Err(refuse("invalid_request", "the comment is empty"));
        }
        let db = &self.app.db;
        let project_id = db
            .board_read(|t| t.item_project(&r.id))?
            .ok_or_else(|| not_found(&r.id))?;
        let mut feed = self.app.board.writer();
        db.add_item_comment(&r.id, body, r.reply_to.as_deref(), &self.actor())?;
        feed.publish(Change {
            project_id: &project_id,
            kind: ChangeKind::ItemUpserted,
            item_id: &r.id,
            card: card_after_commit(db, &r.id),
            from_column: None,
        });
        drop(feed);
        let item = db
            .board_read(|t| t.item(&r.id))?
            .ok_or_else(|| not_found(&r.id))?;
        Ok(c::EditResult {
            outcome: Some(c::edit_result::Outcome::Done(item.into())),
        })
    }
}
