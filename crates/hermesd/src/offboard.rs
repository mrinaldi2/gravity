//! Off-board detection (H-135 G4): a bot that has been working for
//! `OFF_BOARD_AFTER` with no open task linked to a board card is flagged.
//! Visible, not blocking: a "Needs you" row on the dashboard, a badge on the
//! bot, and one note to the project's lead per episode. When the bot is the
//! lead, the row is all (ARCH-R57 S-c).
//!
//! - On-board: the bot holds an open task with a card, or its turn is a run
//!   of a routine that names one (G5).
//! - Exempt: a turn the owner started. A conversation needs no card until it
//!   becomes work (owner ruling 06ac8d95): once the turn changes files,
//!   builds, delegates or hands back an artifact, it counts again.
//! - The episode ends when the bot stops working or is back on the board;
//!   the next one is flagged afresh.
//!
//! Each daemon watches the bots it runs; a linked bot's stand-in is left to
//! its own machine.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use bus::{Bot, BotState, MessageKind, Project};
use chrono::{DateTime, Duration, Utc};

use crate::app::AppState;
use crate::board::model::Role;
use crate::chat::model::{ChatItem, ChatTurn, Trigger};
use crate::events::Push;
use crate::messaging::{self, daemon_sender, Dm};

/// How long a bot works off the board before it is flagged.
pub const OFF_BOARD_MINUTES: i64 = 10;
/// How often the bots are looked at.
const SWEEP_INTERVAL: std::time::Duration = std::time::Duration::from_secs(30);
/// Commands that make an owner's conversation a build.
const BUILD_COMMANDS: &[&str] = &[
    "cargo build",
    "cargo test",
    "cargo clippy",
    "cargo run",
    "pnpm ",
    "npm run",
    "npx ",
    "xcodebuild",
    "gradle",
    "dotnet build",
    "make",
];

#[derive(Debug, Clone, Copy)]
struct Episode {
    since: DateTime<Utc>,
    flagged: bool,
}

/// The bots working off the board on this machine, flagged or not yet.
#[derive(Default)]
pub struct OffBoard {
    bots: Mutex<HashMap<String, Episode>>,
}

impl OffBoard {
    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, Episode>> {
        self.bots.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// When the bot's flagged episode began; `None` while it isn't flagged.
    pub fn flagged_since(&self, bot_id: &str) -> Option<DateTime<Utc>> {
        self.lock()
            .get(bot_id)
            .filter(|e| e.flagged)
            .map(|e| e.since)
    }
}

/// Looks at every bot on a timer, for as long as the daemon runs.
pub async fn watch(app: Arc<AppState>) {
    let mut tick = tokio::time::interval(SWEEP_INTERVAL);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tick.tick().await;
        let app = app.clone();
        let _ = tokio::task::spawn_blocking(move || {
            if let Err(e) = sweep(&app, Utc::now()) {
                tracing::debug!(error = %e, "off-board sweep failed");
            }
        })
        .await;
    }
}

/// One look at every bot of every project with a board, as of `now`.
pub fn sweep(app: &Arc<AppState>, now: DateTime<Utc>) -> anyhow::Result<()> {
    for project in app.db.list_projects()? {
        let board = crate::mcp::has_board(app, &project.id);
        for bot in app.db.list_bots(Some(&project.id))? {
            if bot.peer_id.is_some() {
                continue;
            }
            let off = board && off_board(app, &bot)?;
            step(app, &project, &bot, off, now)?;
        }
    }
    Ok(())
}

/// Whether the bot is working, off the board, on a turn that counts.
fn off_board(app: &AppState, bot: &Bot) -> anyhow::Result<bool> {
    let (state, _) = app.supervisor.state(&bot.id);
    if !matches!(state, BotState::Working | BotState::WaitingForApproval) {
        return Ok(false);
    }
    for task in app.db.open_tasks_for(&bot.id)? {
        if app.db.task_card(&task.id)?.is_some() {
            return Ok(false);
        }
    }
    let turn = app.chat.open_turn(app, bot).unwrap_or_default();
    let routine_card = match turn.as_ref().map(|t| &t.trigger) {
        Some(Trigger::Routine { name, .. }) => routine_has_card(app, bot, name)?,
        _ => false,
    };
    Ok(counts(turn.as_ref(), routine_card))
}

fn routine_has_card(app: &AppState, bot: &Bot, name: &str) -> anyhow::Result<bool> {
    for routine in app.db.list_routines(Some(&bot.id))? {
        if routine.name == name {
            return Ok(app.db.routine_card(&routine.id)?.is_some());
        }
    }
    Ok(false)
}

/// Whether a turn, when its bot holds no task with a card, is off-board
/// work. A turn not read yet (no transcript) counts.
pub(crate) fn counts(turn: Option<&ChatTurn>, routine_card: bool) -> bool {
    let Some(turn) = turn else {
        return true;
    };
    match &turn.trigger {
        Trigger::Owner { .. } => became_work(turn),
        Trigger::Routine { .. } => !routine_card,
        _ => true,
    }
}

/// An owner's conversation that turned into work (ruling 06ac8d95): files
/// changed, a build, a delegation, or an artifact handed back.
pub(crate) fn became_work(turn: &ChatTurn) -> bool {
    if turn.stats.edits > 0 {
        return true;
    }
    turn.items.iter().any(|item| match item {
        ChatItem::Sent { msg_kind, .. } => msg_kind == "task",
        ChatItem::Completed { artifacts, .. } => !artifacts.is_empty(),
        ChatItem::Step(step) => {
            step.tool.ends_with("spawn_worker")
                || (step.tool == "Bash"
                    && step.subtitle.as_deref().is_some_and(|command| {
                        BUILD_COMMANDS
                            .iter()
                            .any(|build| command.trim_start().starts_with(build))
                    }))
        }
        _ => false,
    })
}

/// Moves the bot's episode on: opens it, flags it once it is old enough, or
/// ends it.
fn step(
    app: &Arc<AppState>,
    project: &Project,
    bot: &Bot,
    off: bool,
    now: DateTime<Utc>,
) -> anyhow::Result<()> {
    let flag = {
        let mut bots = app.off_board.lock();
        if !off {
            if bots.remove(&bot.id).is_some_and(|e| e.flagged) {
                tracing::info!(bot = %bot.name, "back on the board");
                app.events.push(Push::BotUpdated { bot: bot.clone() });
            }
            return Ok(());
        }
        let episode = bots.entry(bot.id.clone()).or_insert(Episode {
            since: now,
            flagged: false,
        });
        let due = !episode.flagged && now - episode.since >= Duration::minutes(OFF_BOARD_MINUTES);
        if due {
            episode.flagged = true;
        }
        due
    };
    if flag {
        tracing::info!(bot = %bot.name, "working off-board");
        app.events.push(Push::BotUpdated { bot: bot.clone() });
        tell_lead(app, project, bot)?;
    }
    Ok(())
}

/// One note to the project's lead; none when the bot is the lead, or the
/// project has none.
fn tell_lead(app: &AppState, project: &Project, bot: &Bot) -> anyhow::Result<()> {
    let Some(lead) = lead(app, project)? else {
        return Ok(());
    };
    if lead == bot.id {
        return Ok(());
    }
    let body = format!(
        "{} has been working for {OFF_BOARD_MINUTES} minutes with no task on the board. \
         Give it a card (a task with `item`) or have it stop.",
        bot.name
    );
    messaging::send_dm(
        &app.db,
        &app.events,
        Dm::new(&lead, &daemon_sender(), MessageKind::Note, &body),
    )?;
    Ok(())
}

/// The project's lead: the one named on the project, else a bot with the
/// Lead role.
fn lead(app: &AppState, project: &Project) -> anyhow::Result<Option<String>> {
    if let Some(lead) = &project.lead_bot_id {
        return Ok(Some(lead.clone()));
    }
    Ok(app
        .db
        .project_roles(&project.id)?
        .into_iter()
        .find(|r| r.role == Role::Lead)
        .map(|r| r.bot_id))
}

#[cfg(test)]
mod tests;
