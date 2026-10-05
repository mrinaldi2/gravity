//! Release packages over the WebSocket (H-020 §1.5, §2.2): the owner's
//! release review reads them, and `release_rule` is the only way a release
//! decision is settled. It needs the `approve` grant, and the board's home:
//! a connection here is the owner's own, never a bot's.

use serde_json::{json, Value};

use bus::Capability;
use chrono::{DateTime, Utc};

use crate::board::release::lifecycle::{self, can_rule};
use crate::board::release::model::{parse_arg, Release, Verdict};
use crate::board::release::rule::{rule, ItemVerdict};
use crate::decisions::{invalid, not_found};

use super::Conn;

impl Conn {
    pub(super) fn list_releases(&self, req_id: &Value, req: &Value) -> anyhow::Result<()> {
        let project_id = Self::str_field(req, "project_id")?;
        let releases = self.app.db.board_read(|t| t.releases(project_id))?;
        self.send(json!({
            "type": "releases", "req_id": req_id,
            "releases": releases.iter().map(|r| self.release_json(r)).collect::<Result<Vec<_>, _>>()?,
        }));
        Ok(())
    }

    pub(super) fn get_release(&self, req_id: &Value, req: &Value) -> anyhow::Result<()> {
        let id = Self::str_field(req, "release_id")?;
        let release = self
            .app
            .db
            .board_read(|t| t.release(id))?
            .ok_or_else(|| not_found(format!("no release {id}")))?;
        self.reply_release(req_id, &release)
    }

    /// `{release_id, verdicts: [{item_id, verdict, note?}], expected_version}`.
    pub(super) fn release_rule(&self, req_id: &Value, req: &Value) -> anyhow::Result<()> {
        let id = Self::str_field(req, "release_id")?;
        let expected = req
            .get("expected_version")
            .and_then(Value::as_u64)
            .ok_or_else(|| invalid("'expected_version' is required"))?;
        let verdicts = req
            .get("verdicts")
            .and_then(Value::as_array)
            .ok_or_else(|| invalid("'verdicts' is required: one per item"))?
            .iter()
            .map(|v| {
                let item_id = v.get("item_id").and_then(Value::as_str).unwrap_or_default();
                let verdict = v.get("verdict").and_then(Value::as_str).unwrap_or_default();
                Ok(ItemVerdict {
                    item_id: item_id.to_string(),
                    verdict: parse_arg("verdict", verdict, Verdict::ALL)?,
                    note: v.get("note").and_then(Value::as_str).map(str::to_string),
                })
            })
            .collect::<anyhow::Result<Vec<_>>>()?;
        let release = rule(&self.app, &self.owner(), id, &verdicts, expected)?;
        self.reply_release(req_id, &release)
    }

    /// `{release_id, note?, remind_at?}`: hold a package without rejecting it.
    pub(super) fn release_hold(&self, req_id: &Value, req: &Value) -> anyhow::Result<()> {
        let remind_at = req
            .get("remind_at")
            .and_then(Value::as_str)
            .map(|t| {
                t.parse::<DateTime<Utc>>()
                    .map_err(|_| invalid("'remind_at' is RFC 3339"))
            })
            .transpose()?;
        let note = req.get("note").and_then(Value::as_str);
        let id = Self::str_field(req, "release_id")?;
        let release = lifecycle::hold(&self.app, &self.owner(), id, note, remind_at)?;
        self.reply_release(req_id, &release)
    }

    pub(super) fn release_unhold(&self, req_id: &Value, req: &Value) -> anyhow::Result<()> {
        let id = Self::str_field(req, "release_id")?;
        let release = lifecycle::unhold(&self.app, &self.owner(), id)?;
        self.reply_release(req_id, &release)
    }

    /// `{release_id, reason}`.
    pub(super) fn release_pause(&self, req_id: &Value, req: &Value) -> anyhow::Result<()> {
        let id = Self::str_field(req, "release_id")?;
        let reason = Self::str_field(req, "reason")?;
        let release = lifecycle::pause(&self.app, &self.owner(), &[], id, reason)?;
        self.reply_release(req_id, &release)
    }

    pub(super) fn release_resume(&self, req_id: &Value, req: &Value) -> anyhow::Result<()> {
        let id = Self::str_field(req, "release_id")?;
        let release = lifecycle::resume(&self.app, &self.owner(), &[], id)?;
        self.reply_release(req_id, &release)
    }

    /// A package as this connection sees it: with `can_rule`, and when it
    /// can't, `rule_on`, the computer where it can (H-020 §6.5).
    fn release_json(&self, release: &Release) -> anyhow::Result<Value> {
        let approve = self.caps.contains(&Capability::Approve);
        let (can, on) = can_rule(&self.app, &self.owner(), approve, &release.project_id)?;
        let mut v = release.to_json();
        v["can_rule"] = json!(can);
        v["rule_on"] = json!(on);
        Ok(v)
    }

    fn reply_release(&self, req_id: &Value, release: &Release) -> anyhow::Result<()> {
        let v = self.release_json(release)?;
        self.send(json!({ "type": "release", "req_id": req_id, "release": v }));
        Ok(())
    }
}
