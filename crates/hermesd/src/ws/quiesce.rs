//! The pause for an install over the WebSocket (H-117 Q2): the app's banner
//! reads it, and the owner's **Resume now** ends it.

use serde_json::{json, Value};

use super::Conn;

impl Conn {
    /// `{type: "quiesce", quiesce: …|null}`.
    pub(super) fn quiesce_status(&self, req_id: &Value) -> anyhow::Result<()> {
        let mut out = crate::quiesce::status(&self.app)?;
        out["type"] = json!("quiesce");
        out["req_id"] = req_id.clone();
        self.send(out);
        Ok(())
    }

    /// The owner resumes every project now (approve grant).
    pub(super) fn quiesce_resume(&self, req_id: &Value) -> anyhow::Result<()> {
        let closed = crate::quiesce::resume_all(&self.app, "resumed", chrono::Utc::now())?;
        self.send(
            json!({ "type": "quiesce", "req_id": req_id, "quiesce": null,
                          "resumed": closed }),
        );
        Ok(())
    }
}
