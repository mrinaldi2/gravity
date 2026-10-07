//! One conversation with each bot (H-192, UX-033): the owner's Chat is the
//! owner thread, wherever it is opened. A bot can't tell which screen the
//! owner wrote from and usually answers in its session, so when a turn the
//! owner started from chat ends without `message_owner`, its final text is
//! posted to the owner thread as the bot's message, once, redacted.
//!
//! Turns started any other way (a task, a bus message, a routine, the owner
//! typing in the terminal) are never posted: the owner saw those elsewhere or
//! they weren't to the owner at all.

use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};

use super::model::{AsideKind, ChatItem, ChatTurn, OwnerVia, Trigger};
use super::steps::OWNER;
use crate::app::AppState;
use crate::events::{Internal, Push};

/// How long after the `Stop` hook the transcript gets to show the turn: it
/// lands roughly 250 ms after the hook (see `activity::watch`).
const SETTLE: Duration = Duration::from_secs(10);
const POLL: Duration = Duration::from_millis(250);
/// A turn that ended this long before the hook is still answered, in case its
/// own hook was missed; older turns are history, never posted.
const LOOKBACK: chrono::Duration = chrono::Duration::minutes(2);
/// The clock slack between the hook and the transcript's own timestamp.
const SLACK: chrono::Duration = chrono::Duration::seconds(5);
/// How the owner's own chat reads when it lands mid-turn (`builder`'s
/// incoming aside). `user` is a reserved bot name, so no bot can be `USER`.
const OWNER_INCOMING: &str = "From USER · chat:";
/// What replaces the rest of an answer longer than the thread allows.
const CUT: &str = "\n\n… The rest is in Activity.";

/// Answers each finished turn as its `Stop` hook comes in.
pub async fn watch(app: Arc<AppState>) {
    use tokio::sync::broadcast::error::RecvError;
    let mut internal = app.events.subscribe_internal();
    loop {
        match internal.recv().await {
            Ok(Internal::BotDone { bot_id, .. }) => {
                tokio::spawn(settle(app.clone(), bot_id, Utc::now()));
            }
            Ok(_) => {}
            Err(RecvError::Lagged(skipped)) => {
                tracing::warn!(skipped, "answer capture lagged; the next turn catches up");
            }
            Err(RecvError::Closed) => break,
        }
    }
}

async fn settle(app: Arc<AppState>, bot_id: String, done_at: DateTime<Utc>) {
    let deadline = tokio::time::Instant::now() + SETTLE;
    loop {
        let (app2, id) = (app.clone(), bot_id.clone());
        match tokio::task::spawn_blocking(move || capture(&app2, &id, done_at)).await {
            Ok(Ok(false)) if tokio::time::Instant::now() < deadline => {
                tokio::time::sleep(POLL).await;
            }
            Ok(Err(e)) => {
                tracing::debug!(bot_id, error = %e, "answer capture failed");
                return;
            }
            _ => return,
        }
    }
}

/// Posts every recently finished turn's answer not posted yet. True once
/// the turn that just ended is in the transcript.
pub fn capture(app: &AppState, bot_id: &str, done_at: DateTime<Utc>) -> anyhow::Result<bool> {
    let Some(bot) = app.db.get_live_bot(bot_id)? else {
        return Ok(true);
    };
    if bot.is_linked() {
        return Ok(true); // answered on the bot's own computer
    }
    let mut seen = false;
    let mut posted = Vec::new();
    for turn in app.chat.finished_turns(app, &bot)? {
        let ended = turn.ended_at.unwrap_or(turn.started_at);
        if ended < done_at - LOOKBACK {
            continue;
        }
        seen |= ended >= done_at - SLACK;
        let Some(text) = answer_of(&turn) else {
            continue;
        };
        if !app.db.claim_owner_answer(&bot.id, &turn.id)? {
            continue;
        }
        let msg = match crate::owner_threads::message_owner(app, &bot, &body(&text), false) {
            Ok((msg, _)) => msg,
            Err(error) => {
                // Not posted: let a later pass try again (ARCH S1).
                app.db.release_owner_answer(&bot.id, &turn.id)?;
                return Err(error);
            }
        };
        app.db.set_owner_answer_num(&bot.id, &turn.id, msg.num)?;
        posted.push(ChatTurn {
            answer_num: Some(msg.num),
            ..turn
        });
    }
    if !posted.is_empty() {
        app.events.push(Push::ChatTurns {
            bot_id: bot.id.clone(),
            turns: posted,
        });
    }
    Ok(seen)
}

/// The turn's final text, when it is an answer the thread lacks: the owner
/// started the turn from chat, the bot didn't write to the owner in it, and
/// nothing but the owner's own chat reached it meanwhile. A task or another
/// bot's message landing mid-turn may be what the final text answers, so
/// that turn isn't posted (ARCH S4).
pub fn answer_of(turn: &ChatTurn) -> Option<String> {
    if !matches!(
        turn.trigger,
        Trigger::Owner {
            via: OwnerVia::Chat,
            ..
        }
    ) {
        return None;
    }
    let wrote_or_interrupted = turn.items.iter().any(|item| match item {
        ChatItem::Sent { msg_kind, .. } => msg_kind == OWNER,
        ChatItem::Aside {
            kind: AsideKind::Incoming,
            text,
            ..
        } => !text.starts_with(OWNER_INCOMING),
        _ => false,
    });
    if wrote_or_interrupted {
        return None;
    }
    turn.items.iter().rev().find_map(|item| match item {
        ChatItem::Text { markdown, .. } if !markdown.trim().is_empty() => {
            Some(markdown.trim().to_string())
        }
        _ => None,
    })
}

/// The answer as the thread stores it: secrets masked (H-141), and cut to the
/// thread's limit with a pointer to the rest.
fn body(text: &str) -> String {
    let text = crate::redact::secrets(text);
    let max = crate::owner_threads::BODY_MAX;
    if text.len() <= max {
        return text;
    }
    let mut end = max - CUT.len();
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}{CUT}", &text[..end])
}

/// The turns with each one's `answer_num` set from the bot's posted answers.
pub fn marked(app: &AppState, bot_id: &str, mut turns: Vec<ChatTurn>) -> Vec<ChatTurn> {
    if turns.is_empty() {
        return turns;
    }
    if let Ok(answers) = app.db.owner_answers(bot_id) {
        for turn in &mut turns {
            turn.answer_num = answers.get(&turn.id).copied();
        }
    }
    turns
}

#[cfg(test)]
#[path = "answers_tests.rs"]
mod tests;
