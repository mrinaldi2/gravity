//! The owner's PR requests (H-269; H-261 §4.3, §9): the Owner review
//! setting, the owner's verdict, line comments, flag and resolves, the merge
//! Undo and the release's Leave out. Served here for the app's connection on
//! the board's home, and for one on a linked computer, whose daemon
//! forwards them as `pr_owner` with how the owner proved it there (H-285).
//! Every act but reading the setting needs the owner's proof: a paired
//! device or the app's ticket, here or on the forwarding computer.

use std::sync::Arc;

use serde_json::{json, Value};

use crate::app::AppState;
use crate::board::policy::base;
use crate::db::OwnerProof;
use crate::prs::owner::{self, Mode, Settings};
use crate::prs::review_model::{Finding, Verdict};

fn number(req: &Value) -> anyhow::Result<u32> {
    req.get("number")
        .and_then(Value::as_u64)
        .and_then(|n| u32::try_from(n).ok())
        .ok_or_else(|| anyhow::anyhow!("'number' is required"))
}

fn field<'a>(req: &'a Value, key: &str) -> anyhow::Result<&'a str> {
    req.get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("'{key}' is required"))
}

fn need(proof: Option<&OwnerProof>) -> anyhow::Result<OwnerProof> {
    proof.cloned().ok_or_else(|| {
        crate::decisions::forbidden("only the owner's app or a paired device does this")
    })
}

fn pr(app: &AppState, project: &str, number: u32) -> anyhow::Result<crate::prs::model::Pr> {
    app.db
        .board_read(|t| t.pr(project, number))?
        .ok_or_else(|| crate::decisions::not_found(format!("no PR #{number}")))
}

/// The setting, with the area names the project's main has to pick from.
fn review_settings(app: &Arc<AppState>, project: &str) -> anyhow::Result<Value> {
    let settings = app.db.review_settings(project)?;
    let areas: Vec<String> = base::load(app, project, "refs/heads/main")
        .ok()
        .and_then(|b| b.reviewers.ok())
        .map(|r| r.areas.into_iter().map(|a| a.name).collect())
        .unwrap_or_default();
    Ok(json!({
        "type": "review_settings",
        "review_settings": owner::settings_json(project, &settings, &areas),
    }))
}

/// One owner request on `project`, its JSON reply (without `req_id`).
pub fn serve(
    app: &Arc<AppState>,
    project: &str,
    kind: &str,
    req: &Value,
    proof: Option<&OwnerProof>,
) -> anyhow::Result<Value> {
    Ok(match kind {
        "review_settings_get" => review_settings(app, project)?,
        "review_settings_set" => {
            let mode = req
                .get("owner_review")
                .and_then(Value::as_str)
                .and_then(Mode::parse)
                .ok_or_else(|| anyhow::anyhow!("owner_review is all, areas, flagged or none"))?;
            let areas: Vec<String> = req
                .get("owner_review_areas")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|a| a.as_str().map(str::to_string))
                .collect();
            let proof = need(proof)?;
            let by = owner::provenance(&proof)?;
            app.db
                .set_review_settings(project, &Settings { mode, areas }, &by)?;
            review_settings(app, project)?
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
                app,
                project,
                owner::OwnerVerdict {
                    number: number(req)?,
                    sha: field(req, "sha")?,
                    verdict,
                    summary: req
                        .get("summary")
                        .and_then(Value::as_str)
                        .unwrap_or_default(),
                    findings,
                },
                &need(proof)?,
            )?;
            let pr = pr(app, project, number(req)?)?;
            json!({ "type": "pr", "pr": crate::prs::detail(app, &pr)?, "review": review.to_json(&pr) })
        }
        "pr_comment_add" => {
            let proof = need(proof)?;
            let comment = crate::prs::comments::add_owner(
                app,
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
            need(proof)?;
            let pr = owner::flag(
                app,
                project,
                number(req)?,
                flagged,
                reason,
                owner::Flagger::Owner,
            )?;
            json!({ "type": "pr", "pr": crate::prs::detail(app, &pr)? })
        }
        "pr_comment_resolve" => {
            let by = format!("owner:{}", owner::provenance(&need(proof)?)?);
            let id = field(req, "comment_id")?;
            let c = crate::prs::comments::resolve(app, project, number(req)?, id, &by, true)?;
            let pr = pr(app, project, number(req)?)?;
            json!({ "type": "comment", "comment": crate::prs::comments::shown(app, &pr, &c, &pr.head_sha)? })
        }
        "pr_merge_undo" => {
            let pr = crate::prs::queue::undo(app, project, number(req)?, &need(proof)?)?;
            json!({ "type": "pr", "pr": crate::prs::detail(app, &pr)? })
        }
        "release_leave_out" => {
            let release = field(req, "release_id")?;
            let numbers: Vec<u32> = req
                .get("prs")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|n| n.as_u64().and_then(|n| u32::try_from(n).ok()))
                .collect();
            let out = crate::board::release::leave_out::request(
                app,
                project,
                release,
                &numbers,
                &need(proof)?,
            )?;
            json!({ "type": "leave_out", "result": out })
        }
        // The owner's re-run, forwarded from a linked computer's app (H-285).
        "check_rerun" => {
            need(proof)?;
            let run = crate::prs::check_rerun::rerun(
                app,
                project,
                field(req, "sha")?.trim(),
                field(req, "name")?.trim(),
                &crate::prs::check_rerun::Asker::Owner,
            )?;
            json!({ "type": "check", "check": run.to_json() })
        }
        // A held or failed cleanup: Remove anyway or Keep (H-275, §15.6).
        "cleanup_resolve" => {
            let item = crate::cleanup::owner::resolve(
                app,
                project,
                field(req, "job_id")?,
                field(req, "action")?,
                &need(proof)?,
            )?;
            json!({ "type": "cleanup_item", "cleanup_item": item })
        }
        other => anyhow::bail!("unknown PR request {other}"),
    })
}
