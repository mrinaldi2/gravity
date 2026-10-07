//! The owner's edits from the item drawer (U4, H-018 §3.6): a comment,
//! written as the owner and pushed like a bot's, so open boards and drawers
//! see it. The assignee and the lead are told (H-201).

use bus::contract::board as c;
use bus::MessageKind;

use super::{not_found, refuse, Refusal};
use crate::board::feed::{card_after_commit, Change, ChangeKind};
use crate::board::model::Item;
use crate::messaging;
use crate::ws::owner_card::with_card;
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
        // The comment is posted: a delivery that fails is logged, not refused.
        if let Err(e) = self.tell_about_comment(&project_id, &item, body) {
            tracing::warn!(item_id = %r.id, error = %e, "owner's comment wasn't delivered");
        }
        Ok(c::EditResult {
            outcome: Some(c::edit_result::Outcome::Done(item.into())),
        })
    }

    /// The owner's comment reaches the card's assignee and the project's
    /// lead (H-201) as the owner's message on the card, so they act on it.
    fn tell_about_comment(&self, project_id: &str, item: &Item, body: &str) -> anyhow::Result<()> {
        let db = &self.app.db;
        let lead = db.get_project(project_id)?.and_then(|p| p.lead_bot_id);
        let mut to: Vec<String> = item.assignee.iter().chain(lead.iter()).cloned().collect();
        to.dedup();
        let text = with_card(&item.id, &format!("Owner commented on the card: {body}"));
        let sender = messaging::user_sender();
        for bot_id in to {
            if db.get_live_bot(&bot_id)?.is_none() {
                continue;
            }
            let dm = messaging::Dm::new(&bot_id, &sender, MessageKind::Chat, &text);
            messaging::send_dm(db, &self.app.events, dm)?;
        }
        Ok(())
    }
}
