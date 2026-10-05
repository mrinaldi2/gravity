//! How an owner action ended, told to the bot that proposed it, and the
//! proposer's copy of one that runs on a linked computer (H-117 R1, R3).

use crate::actor::Actor;
use crate::app::AppState;
use crate::decisions::conflict;

use super::model::{OwnerAction, State};
use super::{changed, load};

/// The proposer's copy of an action that runs on a linked computer takes
/// the target's latest (R3); the bot hears how it ended once.
pub fn follow(app: &AppState, copy: &OwnerAction) -> anyhow::Result<OwnerAction> {
    let before = load(app, None, &copy.id)?;
    if !app.db.mirror_owner_action(copy)? {
        return Err(conflict(format!(
            "owner action {} differs from the linked computer's",
            copy.id
        )));
    }
    let now = load(app, None, &copy.id)?;
    if now.state != before.state {
        changed(app, &now);
        tell(app, &now);
    }
    Ok(now)
}

/// Whether it runs on a linked computer rather than here.
pub fn is_elsewhere(app: &AppState, a: &OwnerAction) -> bool {
    app.db
        .daemon_id()
        .is_ok_and(|here| a.proposal.target_machine != here)
}

fn first_line(content: &str) -> String {
    let line = content.lines().next().unwrap_or_default();
    line.chars().take(80).collect()
}

/// What the proposing bot is told once an action ends, if it ended.
fn outcome(a: &OwnerAction) -> Option<String> {
    let what = format!(
        "owner action {} ({})",
        a.id,
        first_line(&a.proposal.content)
    );
    match a.state {
        State::Rejected => Some(format!(
            "The owner rejected {what}{}",
            a.reject_reason
                .as_deref()
                .map(|r| format!(": {r}"))
                .unwrap_or_default()
        )),
        State::Succeeded | State::Failed | State::TimedOut => {
            let tail = a.output_tail.as_deref().unwrap_or_default();
            let skip = tail.chars().count().saturating_sub(3000);
            let short: String = tail.chars().skip(skip).collect();
            Some(format!(
                "Owner action {} ({}) {}{}.\n{short}",
                a.id,
                first_line(&a.proposal.content),
                a.state.as_str().replace('_', " "),
                a.exit_code
                    .map(|c| format!(", exit {c}"))
                    .unwrap_or_default(),
            ))
        }
        _ => None,
    }
}

/// How it ended, as a note to the proposing bot and a comment on its card
/// or decision. A peer's copy says nothing here: the proposer's daemon tells
/// its own bot when it follows the copy.
pub(super) fn tell(app: &AppState, a: &OwnerAction) {
    if a.offered_by().is_some() {
        return;
    }
    let Some(body) = outcome(a) else { return };
    let body = body.as_str();
    if let Some(bot_id) = a.proposal.proposed_by.strip_prefix("bot:") {
        let sender = crate::messaging::user_sender();
        let dm = crate::messaging::Dm::new(bot_id, &sender, bus::MessageKind::Note, body);
        if let Err(e) = crate::messaging::send_dm(&app.db, &app.events, dm) {
            tracing::warn!(error = %e, "owner action note failed");
        }
    }
    if let Some(item) = &a.proposal.item_id {
        let _ = app.db.add_item_comment(item, body, None, &Actor::User);
    }
    if let Some(decision) = &a.proposal.decision_id {
        let _ = app
            .db
            .insert_decision_comment(decision, bus::CommentAuthorKind::User, None, body);
    }
}
