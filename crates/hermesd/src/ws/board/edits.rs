//! The owner's edits from the item drawer (U4, H-018 §3.6): a comment,
//! written as the owner and pushed like a bot's, so open boards and drawers
//! see it. The bots that asked on the card, its assignee and the lead are
//! told (H-201, H-211).

use bus::contract::board as c;
use bus::MessageKind;

use super::{not_found, refuse, Refusal};
use crate::board::feed::{card_after_commit, Change, ChangeKind};
use crate::board::model::{Item, ItemComment};
use crate::messaging;
use crate::ws::owner_card::with_card;
use crate::ws::Conn;

/// Why a bot hears about the owner's comment, which sets its words.
#[derive(Clone, Copy, PartialEq)]
enum Why {
    /// It replies to the bot's question.
    Replied,
    /// The bot asked on the card and no earlier comment answered it.
    Asked,
    /// The card's assignee or the project's lead.
    Card,
}

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
        let comment = db.add_item_comment(&r.id, body, r.reply_to.as_deref(), &self.actor())?;
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
        let told = self
            .tell_about_comment(&project_id, &item, &comment)
            .unwrap_or_else(|e| {
                tracing::warn!(item_id = %r.id, error = %e, "owner's comment wasn't delivered");
                Vec::new()
            });
        Ok(c::EditResult {
            outcome: Some(c::edit_result::Outcome::Done(item.into())),
            told,
        })
    }

    /// The owner's comment reaches every bot that asked on the card and
    /// hasn't had an answer (one on a linked computer through its stand-in),
    /// the card's assignee and the project's lead, once each. It goes as a
    /// note, not a chat, so the bot answers on the card rather than in the
    /// owner's thread (H-192). Returns the bots told.
    fn tell_about_comment(
        &self,
        project_id: &str,
        item: &Item,
        comment: &ItemComment,
    ) -> anyhow::Result<Vec<String>> {
        let db = &self.app.db;
        let lead = db.get_project(project_id)?.and_then(|p| p.lead_bot_id);
        let mut to: Vec<(String, Why)> = db
            .askers_answered_by(comment)?
            .into_iter()
            .map(|a| (a.bot_id, if a.replied { Why::Replied } else { Why::Asked }))
            .collect();
        for bot_id in item.assignee.iter().chain(lead.iter()) {
            to.push((bot_id.clone(), Why::Card));
        }
        let sender = messaging::user_sender();
        let mut told: Vec<String> = Vec::new();
        for (bot_id, why) in to {
            if told.contains(&bot_id) {
                continue;
            }
            // One recipient failing never stops the others (ARCH S1).
            match self.tell_one(&bot_id, &sender, &message(&item.id, comment, why)) {
                Ok(true) => told.push(bot_id),
                Ok(false) => {}
                Err(e) => tracing::warn!(bot_id, error = %e, "owner's comment not delivered"),
            }
        }
        Ok(told)
    }

    fn tell_one(&self, bot_id: &str, sender: &bus::Sender, text: &str) -> anyhow::Result<bool> {
        let db = &self.app.db;
        if db.get_live_bot(bot_id)?.is_none() {
            return Ok(false);
        }
        let dm = messaging::Dm::new(bot_id, sender, MessageKind::Note, text);
        messaging::send_dm(db, &self.app.events, dm)?;
        Ok(true)
    }
}

/// The owner's comment as the bot reads it, with how to answer on the card.
fn message(item_id: &str, comment: &ItemComment, why: Why) -> String {
    let lead = match why {
        Why::Replied => "Owner replied to your question on the card",
        Why::Asked => "Owner commented on the card you asked about",
        Why::Card => "Owner commented on the card",
    };
    with_card(
        item_id,
        &format!(
            "{lead}: {}\n(Answer on the card: item_comment with reply_to {}.)",
            comment.body, comment.id
        ),
    )
}
