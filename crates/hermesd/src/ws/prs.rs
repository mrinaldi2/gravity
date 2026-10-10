//! The owner's PR requests (H-269; H-261 §4.3, §9): the Owner review
//! setting, the owner's verdict and line comments, and the owner's flag.
//! The setting, verdicts and comments need `approve` and a paired device or
//! the app's ticket (`owner_auth`); the owner token is refused before these
//! run. JSON WS `{type, req_id, ...}` with `hermes.pr.v1` field names.

use serde_json::{json, Value};

use super::Conn;
use crate::db::OwnerProof;

/// The owner's PR requests this module serves.
pub(super) const KINDS: &[&str] = &[
    "review_settings_get",
    "review_settings_set",
    "pr_review_submit",
    "pr_comment_add",
    "pr_flag",
    "pr_merge_undo",
    "pr_comment_resolve",
    "release_leave_out",
];

/// The ones that are the owner's own acts: device or ticket only.
pub(super) const OWNER_ONLY: &[&str] = &[
    "review_settings_set",
    "pr_review_submit",
    "pr_comment_add",
    "pr_flag",
    "pr_merge_undo",
    "pr_comment_resolve",
    "release_leave_out",
];

impl Conn {
    pub(super) fn pr_request(&self, kind: &str, req_id: &Value, req: &Value) -> anyhow::Result<()> {
        let project = Self::str_field(req, "project_id")?;
        // The board is on a linked computer: the request goes there, with
        // how the owner proved it here (H-285).
        if let Some(home) = self.app.board_mirror.home_peer(project) {
            return self.forward_owner(kind, req_id, project, &home, req);
        }
        let proof = self.owner_proof();
        let mut reply =
            crate::prs::owner_requests::serve(&self.app, project, kind, req, proof.as_ref())?;
        reply["req_id"] = req_id.clone();
        self.send(reply);
        Ok(())
    }

    /// The owner's request for a board kept on `home`: forwarded as
    /// `pr_owner` with `via` (device or ticket), answered once the home has,
    /// in this computer's ids. Without such proof only the setting is read.
    fn forward_owner(
        &self,
        kind: &str,
        req_id: &Value,
        project: &str,
        home: &str,
        req: &Value,
    ) -> anyhow::Result<()> {
        let via = match self.owner_proof() {
            Some(OwnerProof::Device { .. }) => Some("device"),
            Some(OwnerProof::Ticket) => Some("ticket"),
            _ => None,
        };
        if via.is_none() && kind != "review_settings_get" {
            return Err(crate::decisions::forbidden(
                "only the owner's app or a paired device does this",
            ));
        }
        let frame = json!({
            "type": "pr_owner", "project_id": project, "kind": kind,
            "request": req, "via": via,
        });
        let (app, home, project) = (self.app.clone(), home.to_string(), project.to_string());
        self.answer_later(req_id, async move {
            let mut reply = app.peers.request(&home, frame).await?;
            if let Some(link) = app.db.project_link(&project, &home)? {
                crate::peer::board::ids_from_home(&app, &link, &mut reply);
            }
            Ok(reply)
        });
        Ok(())
    }
}
