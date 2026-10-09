//! The owner as a reviewer (H-261 §4.3; ruling 7629a873): the project's
//! Owner review setting, which PRs wait for the owner, and the owner's own
//! verdict. The owner reviews after the bots; security work always needs
//! the owner unless the setting is `none`. Verdicts and the setting come only
//! from the owner's paired device or the app's ticket, never the owner token
//! a bot can read and never over MCP.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::actor::Actor;
use crate::app::AppState;
use crate::board::model::ColumnCategory;
use crate::board::release::{daemon_move, publish_moves};
use crate::db::reviews::NewReview;
use crate::db::OwnerProof;
use crate::decisions::{conflict, invalid, not_found};
use crate::prs::model::Pr;
use crate::prs::review_model::{Finding, Review, Verdict};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    All,
    Areas,
    Flagged,
    None,
}

impl Mode {
    pub const ALL: [Mode; 4] = [Mode::All, Mode::Areas, Mode::Flagged, Mode::None];

    pub fn as_str(self) -> &'static str {
        match self {
            Mode::All => "all",
            Mode::Areas => "areas",
            Mode::Flagged => "flagged",
            Mode::None => "none",
        }
    }

    pub fn parse(s: &str) -> Option<Mode> {
        Self::ALL.into_iter().find(|m| m.as_str() == s)
    }
}

/// The project's Owner review setting. No row reads as `all`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    pub mode: Mode,
    pub areas: Vec<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            mode: Mode::All,
            areas: Vec::new(),
        }
    }
}

/// What a PR's head touches, as the setting reads it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Shape {
    /// The `reviewers.toml` areas its paths match.
    pub areas: Vec<String>,
    /// A ce area or the policy files: security work.
    pub security: bool,
}

/// Whether the PR waits for the owner (§4.3, ruling 7629a873): never under
/// `none`; always for security work otherwise; then per the setting.
pub fn required(settings: &Settings, shape: &Shape, pr: &Pr) -> bool {
    match settings.mode {
        Mode::None => false,
        _ if shape.security => true,
        Mode::All => true,
        Mode::Areas => shape.areas.iter().any(|a| settings.areas.contains(a)),
        Mode::Flagged => pr.owner_flagged,
    }
}

/// The owner's provenance as a review stores it.
pub fn provenance(proof: &OwnerProof) -> anyhow::Result<String> {
    match proof {
        OwnerProof::Device { device_id } => Ok(format!("device:{device_id}")),
        OwnerProof::Ticket => Ok("ticket".to_string()),
        OwnerProof::Peer { .. } => Err(invalid(
            "review it on the board's own computer; a relayed review isn't recorded",
        )),
    }
}

/// The owner's verdict on the head (§4.3). A request for changes needs a
/// note and sends the card back to Doing.
/// The owner's verdict, as the app sends it.
pub struct OwnerVerdict<'a> {
    pub number: u32,
    pub sha: &'a str,
    pub verdict: Verdict,
    pub summary: &'a str,
    pub findings: Vec<Finding>,
}

pub fn submit(
    app: &Arc<AppState>,
    project: &str,
    v: OwnerVerdict<'_>,
    proof: &OwnerProof,
) -> anyhow::Result<Review> {
    let OwnerVerdict {
        number,
        sha,
        verdict,
        summary,
        findings,
    } = v;
    let provenance = provenance(proof)?;
    let pr = app
        .db
        .board_read(|t| t.pr(project, number))?
        .ok_or_else(|| not_found(format!("no PR #{number} in this project")))?;
    anyhow::ensure!(pr.state.is_live(), "PR #{number} is {}", pr.state.as_str());
    if sha.trim() != pr.head_sha {
        return Err(conflict(format!(
            "PR #{number} is at {}; review the head that is there",
            pr.head_sha
        )));
    }
    if pr.moved_unreported {
        return Err(conflict(format!(
            "PR #{number}'s branch moved without a report; wait for the pusher to report it"
        )));
    }
    if verdict == Verdict::ChangesRequested && summary.trim().is_empty() {
        return Err(invalid("say what to change"));
    }
    let mut feed = app.board.writer();
    let (review, moved) = app.db.board_tx(|t| {
        let review = t.insert_review(&NewReview {
            pr_id: &pr.id,
            role: "owner",
            reviewer: "owner",
            provenance: Some(&provenance),
            sha: &pr.head_sha,
            patch_id: &pr.head_patch_id,
            verdict,
            summary: summary.trim(),
            findings: &findings,
            artifact: None,
        })?;
        let moved = if verdict == Verdict::ChangesRequested {
            let note = format!("the owner asked for changes on PR #{number}");
            daemon_move(
                t,
                project,
                &pr.item_id,
                ColumnCategory::Doing,
                &note,
                true,
                &Actor::User,
            )?
            .map(|from| vec![(pr.item_id.clone(), from)])
            .unwrap_or_default()
        } else {
            Vec::new()
        };
        Ok((review, moved))
    })?;
    publish_moves(app, &mut feed, project, &moved);
    Ok(review)
}

/// Whether the PR is ready for the owner's look: the owner is needed, the
/// branch didn't move unreported, every bot role has a fresh approval and
/// the owner has none on this change yet (ruling 7629a873: after the bots).
pub fn waits_for_owner(
    settings: &Settings,
    shape: &Shape,
    pr: &Pr,
    needs: &[String],
    reviews: &[Review],
) -> bool {
    if !pr.state.is_live() || pr.moved_unreported || !required(settings, shape, pr) {
        return false;
    }
    let fresh = |role: &str| {
        reviews
            .iter()
            .rev()
            .find(|r| r.role == role)
            .is_some_and(|r| r.verdict == Verdict::Approved && !r.stale(pr))
    };
    needs.iter().filter(|r| *r != "owner").all(|r| fresh(r)) && !fresh("owner")
}

/// The setting as the WS answers it, with the base's area names to pick from.
pub fn settings_json(project: &str, settings: &Settings, areas: &[String]) -> Value {
    json!({
        "project_id": project,
        "owner_review": settings.mode.as_str(),
        "owner_review_areas": settings.areas,
        "areas": areas,
    })
}

/// `pr_flag`: the owner (WS) or the lead (MCP) flags a PR for the owner's
/// review with a reason, or clears the flag. Under `flagged` a flagged PR
/// waits for the owner.
pub fn flag(
    app: &AppState,
    project: &str,
    number: u32,
    flagged: bool,
    reason: &str,
) -> anyhow::Result<Pr> {
    let reason = reason.trim();
    if flagged && reason.is_empty() {
        return Err(invalid("say why the owner should review it"));
    }
    app.db.board_tx(|t| {
        let pr = t
            .pr(project, number)?
            .ok_or_else(|| not_found(format!("no PR #{number} in this project")))?;
        t.set_pr_flag(&pr, flagged, flagged.then_some(reason))?;
        t.pr(project, number)?
            .ok_or_else(|| not_found(format!("no PR #{number}")))
    })
}

/// A PR's owner fields for clients: whether it waits for the owner.
pub fn owner_json(app: &AppState, pr: &Pr) -> anyhow::Result<Value> {
    let settings = app.db.review_settings(&pr.project_id)?;
    let shape = app.db.board_read(|t| t.pr_shape(&pr.id))?;
    Ok(json!({
        "owner_review_required": required(&settings, &shape, pr),
        "areas": shape.areas,
        "security": shape.security,
    }))
}

#[cfg(test)]
#[path = "owner_tests.rs"]
mod tests;
