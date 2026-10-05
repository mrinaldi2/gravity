//! The owner's ruling on a package (H-020 §2.2–2.3, §6.1). The only way a
//! release decision is settled, and the only path that moves items to
//! Deploying. It takes the owner's own credentials on the board's home: a bot
//! can't call it, and a relayed ruling can't exist.

use std::sync::Arc;

use bus::{now, DecisionState, Ruling};

use crate::actor::Actor;
use crate::app::AppState;
use crate::board::model::ColumnCategory;
use crate::decisions::publish::publish_settled;
use crate::decisions::{conflict, forbidden, invalid, not_found};

use super::model::{Release, ReleaseStatus, Verdict};
use super::{check_frozen, daemon_move, publish_moves, publish_touched};

pub struct ItemVerdict {
    pub item_id: String,
    pub verdict: Verdict,
    pub note: Option<String>,
}

/// What a set of verdicts does to the package (H-020 §6.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// Every item ships: the gate opens.
    Approved,
    /// Some ship, some don't: DevOps builds a successor without the rest.
    Repackaging,
    /// Every item held: the package waits, nothing moves.
    Held,
    /// Nothing ships.
    Rejected,
}

pub fn outcome(verdicts: &[Verdict]) -> Outcome {
    let all = |v: Verdict| verdicts.iter().all(|x| *x == v);
    if all(Verdict::Ship) {
        Outcome::Approved
    } else if all(Verdict::Hold) {
        Outcome::Held
    } else if verdicts.contains(&Verdict::Ship) {
        Outcome::Repackaging
    } else {
        Outcome::Rejected
    }
}

/// How the ruling is stored on the decision: `owner` or `device:<id>`.
fn answered_by(actor: &Actor<'_>) -> String {
    match actor {
        Actor::Device(id) => format!("device:{id}"),
        _ => "owner".to_string(),
    }
}

pub fn rule(
    app: &Arc<AppState>,
    actor: &Actor<'_>,
    release_id: &str,
    verdicts: &[ItemVerdict],
    expected_version: u64,
) -> anyhow::Result<Release> {
    if !actor.is_owner() {
        return Err(forbidden(
            "release rulings come from the owner's dashboard or phone",
        ));
    }
    let release = app
        .db
        .board_read(|t| t.release(release_id))?
        .ok_or_else(|| not_found(format!("no release {release_id}")))?;
    let project = release.project_id.clone();
    let home = app.db.board_settings(&project)?.map(|s| s.home_daemon_id);
    if home.as_deref() != Some(app.db.daemon_id()?.as_str()) {
        return Err(forbidden(
            "rule on this release from its board's home computer",
        ));
    }
    let decision_id = release
        .decision_id
        .clone()
        .ok_or_else(|| conflict(format!("release {} has no decision yet", release.name)))?;
    let outcome = outcome(&checked(&release, verdicts)?);
    let summary = summary(&release, verdicts, outcome);

    let mut feed = app.board.writer();
    let (ruled, moved, touched) = app.db.board_tx(|t| {
        let release = t.release(release_id)?.expect("read above");
        if release.version != expected_version {
            return Err(conflict(format!(
                "release {} changed since you read it (now version {}); review it again",
                release.name, release.version
            )));
        }
        if !matches!(
            release.status,
            ReleaseStatus::AwaitingOwner | ReleaseStatus::Held
        ) {
            return Err(conflict(format!(
                "release {} is {}; it isn't waiting for a ruling",
                release.name,
                release.status.as_str()
            )));
        }
        check_frozen(&release)?;
        let (mut moved, mut touched) = (Vec::new(), Vec::new());
        for v in verdicts {
            t.set_verdict(release_id, &v.item_id, v.verdict, v.note.as_deref())?;
            if outcome == Outcome::Held {
                continue;
            }
            let (to, first, verb) = match v.verdict {
                Verdict::Ship if outcome == Outcome::Approved => {
                    (ColumnCategory::Deploying, false, "shipped")
                }
                Verdict::Hold => (ColumnCategory::Ready, false, "held"),
                Verdict::Rework => (ColumnCategory::Doing, true, "sent back for rework"),
                // Shipped from a mixed package: waits in Owner testing for
                // the successor package, approved.
                _ => continue,
            };
            let mut note = format!("{verb} in release {}", release.name);
            if let Some(n) = v.note.as_deref().map(str::trim).filter(|n| !n.is_empty()) {
                note.push_str(&format!(": {n}"));
            }
            let from = daemon_move(t, &project, &v.item_id, to, &note, first, actor)?;
            moved.extend(from.map(|f| (v.item_id.clone(), f)));
            if to != ColumnCategory::Deploying && t.set_item_release(&v.item_id, None, actor)? {
                touched.push(v.item_id.clone());
            }
        }
        let status = match outcome {
            Outcome::Approved => ReleaseStatus::Approved,
            Outcome::Repackaging => ReleaseStatus::Repackaging,
            Outcome::Held => ReleaseStatus::Held,
            Outcome::Rejected => ReleaseStatus::Rejected,
        };
        t.set_release_status(release_id, status)?;
        Ok((t.release(release_id)?.expect("read above"), moved, touched))
    })?;
    publish_moves(app, &mut feed, &project, &moved);
    publish_touched(app, &mut feed, &project, &touched, &moved);
    drop(feed);
    settle(app, actor, &decision_id, outcome, &summary)?;
    Ok(ruled)
}

/// Every item exactly once, each with ship, hold or rework.
fn checked(release: &Release, verdicts: &[ItemVerdict]) -> anyhow::Result<Vec<Verdict>> {
    let mut seen: Vec<&str> = verdicts.iter().map(|v| v.item_id.as_str()).collect();
    seen.sort_unstable();
    let mut items: Vec<&str> = release.items.iter().map(|i| i.item_id.as_str()).collect();
    items.sort_unstable();
    if seen != items {
        return Err(invalid(format!(
            "give one verdict for each item in the release: {}",
            items.join(", ")
        )));
    }
    if verdicts.iter().any(|v| v.verdict == Verdict::Pending) {
        return Err(invalid("each verdict is ship, hold or rework"));
    }
    Ok(verdicts.iter().map(|v| v.verdict).collect())
}

fn summary(release: &Release, verdicts: &[ItemVerdict], outcome: Outcome) -> String {
    let head = match outcome {
        Outcome::Approved => format!("Ship release {}.", release.name),
        Outcome::Repackaging => format!(
            "Release {}: ship some, not all. Build a successor package with only the shipped items.",
            release.name
        ),
        Outcome::Held => format!("Release {}: on hold.", release.name),
        Outcome::Rejected => format!("Release {}: not shipping.", release.name),
    };
    let lines: Vec<String> = verdicts
        .iter()
        .map(
            |v| match v.note.as_deref().filter(|n| !n.trim().is_empty()) {
                Some(n) => format!("{}: {} ({})", v.item_id, v.verdict.as_str(), n.trim()),
                None => format!("{}: {}", v.item_id, v.verdict.as_str()),
            },
        )
        .collect();
    format!("{head}\n{}", lines.join("\n"))
}

/// Record the ruling on the decision: a hold keeps it open (§6.2), anything
/// else settles it under the owner's name and tells DevOps.
fn settle(
    app: &Arc<AppState>,
    actor: &Actor<'_>,
    decision_id: &str,
    outcome: Outcome,
    summary: &str,
) -> anyhow::Result<()> {
    let decision = app
        .db
        .get_decision(decision_id)?
        .ok_or_else(|| not_found("the release's decision is missing"))?;
    if outcome == Outcome::Held {
        app.db.try_hold(decision_id, None)?;
        return Ok(());
    }
    if decision.state == DecisionState::Held {
        app.db.try_resume(decision_id)?;
    }
    let ruling = Ruling {
        option: None,
        text: summary.to_string(),
        reason: None,
        answered_at: now(),
        answered_by: answered_by(actor),
    };
    if !app.db.try_answer(decision_id, &ruling)? {
        return Err(conflict(
            "the release decision can't be answered in its state",
        ));
    }
    publish_settled(app, decision_id, None)?;
    Ok(())
}
