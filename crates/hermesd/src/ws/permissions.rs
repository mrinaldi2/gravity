//! Permission prompts waiting on the owner, and answering them.

use bus::Capability;
use serde_json::{json, Value};

use crate::approval::{Answer, NeedsApprove};

use super::Conn;

impl Conn {
    pub(super) fn list_permissions(&self, req_id: &Value, req: &Value) -> anyhow::Result<()> {
        let bot_id = req.get("bot_id").and_then(Value::as_str);
        let permissions = self.app.approvals.list(bot_id);
        self.send(json!({ "type": "permissions", "req_id": req_id, "permissions": permissions }));
        Ok(())
    }

    /// Answers a prompt. `control`, like typing the answer into the terminal:
    /// letting a bot run a tool is running the fleet, not ruling for the owner.
    ///
    /// A terminal command's card is the exception: allowing it makes that
    /// command the owner, so it takes `approve`. Every WS connection is the
    /// owner directly (client token or ticket) or one of their devices, never
    /// a relayed actor, so the grant is the whole check.
    pub(super) fn answer_permission(&self, req_id: &Value, req: &Value) -> anyhow::Result<()> {
        let request_id = Self::str_field(req, "request_id")?;
        let answer: Answer = serde_json::from_value(
            req.get("decision")
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("'decision' is required"))?,
        )
        .map_err(|_| anyhow::anyhow!("'decision' must be allow_once, allow_session or deny"))?;
        let reason = req
            .get("reason")
            .and_then(Value::as_str)
            .filter(|r| !r.trim().is_empty())
            .map(str::to_string);
        let can_approve = self.caps.contains(&Capability::Approve);
        match self
            .app
            .approvals
            .answer(request_id, answer, reason, can_approve)
        {
            Ok(permission) => self
                .send(json!({ "type": "permission", "req_id": req_id, "permission": permission })),
            Err(e) if e.is::<NeedsApprove>() => self.reply_err(req_id, "forbidden", &e.to_string()),
            Err(e) => self.reply_err(req_id, "conflict", &e.to_string()),
        }
        Ok(())
    }
}
