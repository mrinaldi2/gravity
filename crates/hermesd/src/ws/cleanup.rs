//! The disk report and the owner's Clean up (H-275; H-261 §15.6), JSON WS:
//! `disk_report {machine?, refresh?}` (read) and `cleanup_now {machine?}`
//! (control), for this computer or a linked one by name.

use serde_json::{json, Value};

use super::Conn;

impl Conn {
    pub(super) fn cleanup_request(
        &self,
        kind: &str,
        req_id: &Value,
        req: &Value,
    ) -> anyhow::Result<()> {
        let machine = req["machine"].as_str().unwrap_or_default().to_string();
        let app = self.app.clone();
        if kind == "disk_report" {
            let refresh = req["refresh"].as_bool() == Some(true);
            self.answer_later(req_id, async move {
                let report = crate::cleanup::disk::report_of(&app, &machine, refresh).await?;
                Ok(json!({ "type": "disk_report", "disk_report": report }))
            });
        } else {
            self.answer_later(req_id, async move {
                crate::cleanup::sweep_remote::clean_up(&app, &machine).await
            });
        }
        Ok(())
    }
}
