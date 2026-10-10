//! The owner's PR requests (H-269; H-261 §4.3, §9): the Owner review
//! setting, the owner's verdict and line comments, and the owner's flag.
//! The setting, verdicts and comments need `approve` and a paired device or
//! the app's ticket (`owner_auth`); the owner token is refused before these
//! run. JSON WS `{type, req_id, ...}` with `hermes.pr.v1` field names.

use serde_json::{json, Value};

use super::Conn;
use crate::board::policy::base;
use crate::prs::owner::{self, Mode, Settings};
use crate::prs::review_model::{Finding, Verdict};

/// The owner's PR requests this module serves.
pub(super) const KINDS: &[&str] = &[
    "review_settings_get",
    "review_settings_set",
    "pr_review_submit",
    "pr_comment_add",
    "pr_flag",
    "pr_merge_undo",
    "pr_comment_resolve",
];

/// The ones that are the owner's own acts: device or ticket only.
pub(super) const OWNER_ONLY: &[&str] = &[
    "review_settings_set",
    "pr_review_submit",
    "pr_comment_add",
    "pr_flag",
    "pr_merge_undo",
    "pr_comment_resolve",
];

fn number(req: &Value) -> anyhow::Result<u32> {
    req.get("number")
        .and_then(Value::as_u64)
        .and_then(|n| u32::try_from(n).ok())
        .ok_or_else(|| anyhow::anyhow!("'number' is required"))
}

impl Conn {
    pub(super) fn pr_request(&self, kind: &str, req_id: &Value, req: &Value) -> anyhow::Result<()> {
        let project = Self::str_field(req, "project_id")?;
        let reply = match kind {
            "review_settings_get" => self.review_settings(project)?,
            "review_settings_set" => {
                let mode = req
                    .get("owner_review")
                    .and_then(Value::as_str)
                    .and_then(Mode::parse)
                    .ok_or_else(|| {
                        anyhow::anyhow!("owner_review is all, areas, flagged or none")
                    })?;
                let areas: Vec<String> = req
                    .get("owner_review_areas")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(|a| a.as_str().map(str::to_string))
                    .collect();
                let proof = self.proof()?;
                let by = owner::provenance(&proof)?;
                self.app
                    .db
                    .set_review_settings(project, &Settings { mode, areas }, &by)?;
                self.review_settings(project)?
            }
            "pr_review_submit" => {
                let verdict = req
                    .get("verdict")
                    .and_then(Value::as_str)
                    .and_then(Verdict::parse)
                    .ok_or_else(|| anyhow::anyhow!("verdict is approved or changes_requested"))?;
                let findings: Vec<Finding> =
                    serde_json::from_value(req.get("findings").cloned().unwrap_or(json!([])))?;
                let review = owner::submit(
                    &self.app,
                    project,
                    owner::OwnerVerdict {
                        number: number(req)?,
                        sha: Self::str_field(req, "sha")?,
                        verdict,
                        summary: req
                            .get("summary")
                            .and_then(Value::as_str)
                            .unwrap_or_default(),
                        findings,
                    },
                    &self.proof()?,
                )?;
                let pr = self.pr(project, number(req)?)?;
                json!({ "type": "pr", "pr": crate::prs::detail(&self.app, &pr)?, "review": review.to_json(&pr) })
            }
            "pr_comment_add" => {
                let proof = self.proof()?;
                let comment = crate::prs::comments::add_owner(
                    &self.app,
                    project,
                    number(req)?,
                    req,
                    &owner::provenance(&proof)?,
                )?;
                json!({ "type": "comment", "comment": comment })
            }
            "pr_flag" => {
                let flagged = req.get("flagged").and_then(Value::as_bool).unwrap_or(true);
                let reason = req
                    .get("reason")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                self.proof()?;
                let pr = owner::flag(
                    &self.app,
                    project,
                    number(req)?,
                    flagged,
                    reason,
                    owner::Flagger::Owner,
                )?;
                json!({ "type": "pr", "pr": crate::prs::detail(&self.app, &pr)? })
            }
            "pr_comment_resolve" => {
                let by = format!("owner:{}", owner::provenance(&self.proof()?)?);
                let id = Self::str_field(req, "comment_id")?;
                let c =
                    crate::prs::comments::resolve(&self.app, project, number(req)?, id, &by, true)?;
                let pr = self.pr(project, number(req)?)?;
                json!({ "type": "comment", "comment": crate::prs::comments::shown(&self.app, &pr, &c, &pr.head_sha)? })
            }
            "pr_merge_undo" => {
                let pr = crate::prs::queue::undo(&self.app, project, number(req)?, &self.proof()?)?;
                json!({ "type": "pr", "pr": crate::prs::detail(&self.app, &pr)? })
            }
            other => anyhow::bail!("unknown PR request {other}"),
        };
        let mut reply = reply;
        reply["req_id"] = req_id.clone();
        self.send(reply);
        Ok(())
    }

    fn proof(&self) -> anyhow::Result<crate::db::OwnerProof> {
        self.owner_proof().ok_or_else(|| {
            crate::decisions::forbidden("only the owner's app or a paired device does this")
        })
    }

    fn pr(&self, project: &str, number: u32) -> anyhow::Result<crate::prs::model::Pr> {
        self.app
            .db
            .board_read(|t| t.pr(project, number))?
            .ok_or_else(|| crate::decisions::not_found(format!("no PR #{number}")))
    }

    /// The setting, with the area names the project's main has to pick from.
    fn review_settings(&self, project: &str) -> anyhow::Result<Value> {
        let settings = self.app.db.review_settings(project)?;
        let areas: Vec<String> = base::load(&self.app, project, "refs/heads/main")
            .ok()
            .and_then(|b| b.reviewers.ok())
            .map(|r| r.areas.into_iter().map(|a| a.name).collect())
            .unwrap_or_default();
        Ok(json!({
            "type": "review_settings",
            "review_settings": owner::settings_json(project, &settings, &areas),
        }))
    }
}
