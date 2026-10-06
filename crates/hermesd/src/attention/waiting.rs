//! Who waits on the owner among the project's bots here: their permission
//! prompts with a card, and bots waiting for input or on a prompt only
//! their terminal shows (H-172).

use std::collections::{HashMap, HashSet};

use bus::contract::home::{attention_row::Target, AttentionKind, BotRef};
use bus::BotState;

use super::rows::{Builder, Part};
use super::weight;
use crate::app::AppState;
use crate::supervisor::APPROVAL_NOTIFICATION;

/// Permission prompts of the project's bots here, and its bots waiting for
/// the owner: for their input, or on a permission prompt that has no card
/// (H-172). Stand-ins run elsewhere: their computer counts them.
pub(super) fn prompts_and_waiting(app: &AppState, b: &mut Builder<'_>) -> anyhow::Result<()> {
    let bots: HashMap<String, bus::Bot> = app
        .db
        .list_bots(Some(b.project_id))?
        .into_iter()
        .filter(|bot| !bot.is_linked())
        .map(|bot| (bot.id.clone(), bot))
        .collect();
    for prompt in app.approvals.list(None) {
        let Some(bot) = bots.get(&prompt.bot_id) else {
            continue;
        };
        let part = Part {
            kind: AttentionKind::PermissionPrompt,
            target_id: prompt.id.clone(),
            title: format!("{} asks: {}", bot.name, prompt.summary),
            created_at: prompt.created_at,
            target: Some(Target::RequestId(prompt.id.clone())),
        };
        b.push(part, weight(AttentionKind::PermissionPrompt), None);
    }
    let mut waiting: Vec<(&bus::Bot, BotState, String)> = bots
        .values()
        .filter_map(|bot| {
            let (state, reason) = app.supervisor.state(&bot.id);
            let waits = match state {
                BotState::WaitingForUser => true,
                // A prompt with a card is its own row above; one answered
                // only in the bot's terminal has none (H-172).
                BotState::WaitingForApproval => app.approvals.list(Some(&bot.id)).is_empty(),
                _ => false,
            };
            waits.then_some((bot, state, reason))
        })
        .collect();
    waiting.sort_by(|a, b| a.0.name.cmp(&b.0.name));
    for (bot, state, reason) in waiting {
        let title = match state {
            BotState::WaitingForApproval => approval_title(&bot.name, &reason),
            _ if reason.is_empty() => format!("{} is waiting for you", bot.name),
            _ => format!("{} is waiting for you: {reason}", bot.name),
        };
        let part = Part {
            kind: AttentionKind::BotWaiting,
            target_id: bot.id.clone(),
            title,
            created_at: app.overview.waiting_since(&bot.id),
            target: Some(Target::Bot(BotRef {
                daemon_id: b.me.clone(),
                bot_id: bot.id.clone(),
                name: bot.name.clone(),
            })),
        };
        b.push(part, weight(AttentionKind::BotWaiting), None);
    }
    Ok(())
}

/// The project's bots here waiting on the owner: for their input, or on a
/// permission prompt, with a card or only in their terminal (H-172). The
/// projects home's `bots_waiting` counts these (H-161), so it agrees with
/// the rows above. Stand-ins run elsewhere: their computer counts them.
pub(crate) fn waiting_bots(app: &AppState, project_id: &str) -> anyhow::Result<Vec<bus::Bot>> {
    let prompted: HashSet<String> = app
        .approvals
        .list(None)
        .into_iter()
        .map(|p| p.bot_id)
        .collect();
    Ok(app
        .db
        .list_bots(Some(project_id))?
        .into_iter()
        .filter(|bot| !bot.is_linked())
        .filter(|bot| {
            matches!(
                app.supervisor.state(&bot.id).0,
                BotState::WaitingForUser | BotState::WaitingForApproval
            ) || prompted.contains(&bot.id)
        })
        .collect())
}

/// "<bot> needs approval", with the command when the bot's state names one
/// (the hook's summary, masked); a bare notification names none. `title`
/// masks token-like values again, whatever set the reason.
fn approval_title(bot: &str, reason: &str) -> String {
    let command = reason.trim();
    if command.is_empty() || command == APPROVAL_NOTIFICATION {
        format!("{bot} needs approval")
    } else {
        format!("{bot} needs approval: {command}")
    }
}
