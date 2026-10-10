//! Message and conversation requests.

use bus::MessageKind;
use serde_json::{json, Value};

use crate::messaging;

use super::{owner_card, Conn};

impl Conn {
    // ---- messaging ----

    /// The owner's chat to a bot, optionally on a card (H-128 D5).
    pub(super) fn send_user_message(&self, req_id: &Value, req: &Value) -> anyhow::Result<()> {
        let body = Self::str_field(req, "body")?;
        let sender = messaging::user_sender();
        let bot_id = Self::str_field(req, "to_bot_id")?;
        let item_id = req["item_id"].as_str().filter(|id| !id.is_empty());
        let card = match item_id {
            Some(item_id) => {
                let bot = self
                    .app
                    .db
                    .get_live_bot(bot_id)?
                    .ok_or_else(|| crate::decisions::not_found(format!("no bot {bot_id}")))?;
                Some((item_id, self.owner_card(&bot, item_id)?, bot))
            }
            None => None,
        };
        let stored = match &card {
            Some((item_id, ..)) => owner_card::with_card(item_id, body),
            None => body.to_string(),
        };
        // Recorded as the owner's own only when this connection proved it
        // is the owner: never on the owner token a bot can read (H-195 D1).
        let proof = self.owner_proof();
        let mut dm = messaging::Dm::new(bot_id, &sender, MessageKind::Chat, &stored);
        dm.owner = proof.as_ref();
        let msg = messaging::send_dm(&self.app.db, &self.app.events, dm)?;
        let Some((item_id, board, bot)) = card else {
            self.send(json!({ "type": "message", "req_id": req_id, "message": msg }));
            return Ok(());
        };
        // The message is sent: a comment that fails is logged, not refused.
        let commented = self
            .comment_owner_card(&board, &bot, item_id, body)
            .unwrap_or_else(|e| {
                tracing::warn!(item_id, error = %e, "owner's card comment failed");
                false
            });
        self.send(json!({
            "type": "message", "req_id": req_id, "message": msg,
            "item_id": item_id, "commented": commented,
        }));
        Ok(())
    }

    pub(super) fn list_messages(&self, req_id: &Value, req: &Value) -> anyhow::Result<()> {
        let conversation_id = Self::str_field(req, "conversation_id")?;
        let before = req.get("before_num").and_then(|v| v.as_i64());
        let limit = req.get("limit").and_then(|v| v.as_i64()).unwrap_or(100);
        let messages = self.app.db.list_messages(conversation_id, before, limit)?;
        let messages = messaging::for_clients(&self.app.db, messages)?;
        self.send(json!({ "type": "messages", "req_id": req_id, "messages": messages }));
        Ok(())
    }

    pub(super) fn list_conversations(&self, req_id: &Value, req: &Value) -> anyhow::Result<()> {
        let project_id = req.get("project_id").and_then(|v| v.as_str());
        let conversations = self.app.db.list_conversations(project_id)?;
        self.send(
            json!({ "type": "conversations", "req_id": req_id, "conversations": conversations }),
        );
        Ok(())
    }
}
