//! Owner threads (H-128 R2.2, D6): bots write to the owner with
//! `message_owner`, and ask with `asks: true` or `item_comment asks_owner`.
//! A bot's thread is its DM conversation's messages from the owner and its
//! own notes to the owner; a question is an `owner_question` attention row
//! until the owner answers it or dismisses it.
//!
//! A bot's own computer holds its thread and its questions. Another computer
//! reads them through the bot's stand-in (`remote`).

use std::sync::Arc;

use bus::contract::home::{
    AttentionDismissed, BotRef, MessageBrief, OwnerThread, OwnerThreadMarked, OwnerThreadPage,
    ThreadMessage,
};
use bus::{Message, MessageKind, SenderKind};
use serde_json::json;

use crate::app::AppState;
use crate::attention::{cut, timestamp};
use crate::db::{Asked, OwnerQuestion};
use crate::decisions::{invalid, not_found};
use crate::events::Push;

mod cards;
mod remote;

pub use cards::card_changed;
pub use remote::{get, read, receive_updated, serve, threads};

/// The longest message a bot may send the owner (R2.2): long content goes
/// in an artifact.
pub const BODY_MAX: usize = 4096;
/// The longest text a thread's last-message brief carries.
const BRIEF_MAX: usize = 200;
/// A page's default and largest size.
const PAGE_DEFAULT: u32 = 50;
const PAGE_MAX: u32 = 200;

/// The bot as clients find it: on its own computer, in its id there (R2.3).
pub fn bot_ref(app: &AppState, bot: &bus::Bot) -> BotRef {
    let (daemon_id, bot_id) = match (&bot.peer_id, &bot.remote_bot_id) {
        (Some(peer_id), Some(remote)) => (
            app.db
                .get_peer(peer_id)
                .ok()
                .flatten()
                .and_then(|p| p.daemon_id)
                .unwrap_or_default(),
            remote.clone(),
        ),
        _ => (app.db.daemon_id().unwrap_or_default(), bot.id.clone()),
    };
    BotRef {
        daemon_id,
        bot_id,
        name: bot.name.clone(),
    }
}

/// `message_owner`: a note from the bot in its own thread, and a question
/// when it `asks`. Nothing is delivered: the owner reads it in the thread.
/// The tool has checked the body's size ([`BODY_MAX`]) before its `[card]`
/// lead was added.
pub fn message_owner(
    app: &AppState,
    bot: &bus::Bot,
    body: &str,
    asks: bool,
) -> anyhow::Result<(Message, Option<OwnerQuestion>)> {
    let body = body.trim();
    if body.is_empty() {
        return Err(invalid("'body' is empty"));
    }
    let conv = app
        .db
        .dm_conversation(&bot.id)?
        .ok_or_else(|| anyhow::anyhow!("{} has no conversation", bot.name))?;
    let sender = crate::mcp::bot_sender(bot);
    let msg = app
        .db
        .insert_message(&conv.id, &sender, MessageKind::Note, body, None, None)?;
    app.events.push(Push::MessageNew {
        message: msg.clone(),
    });
    let question = if asks {
        let title = format!("{} asks: {body}", bot.name);
        Some(
            app.db
                .add_owner_question(bot, Asked::Thread(msg.num), &title)?,
        )
    } else {
        None
    };
    updated(app, bot);
    Ok((msg, question))
}

/// `item_comment asks_owner`: a question on the card, after the comment.
pub fn ask_on_card(
    app: &AppState,
    bot: &bus::Bot,
    item_id: &str,
    body: &str,
) -> anyhow::Result<OwnerQuestion> {
    let title = format!("{} asks on {item_id}: {body}", bot.name);
    let question = app
        .db
        .add_owner_question(bot, Asked::Card(item_id), &title)?;
    updated(app, bot);
    Ok(question)
}

/// Tells clients, and every computer the bot is linked to, that its thread
/// or its questions changed.
pub fn updated(app: &AppState, bot: &bus::Bot) {
    app.events.push(Push::OwnerThreadUpdated {
        bot: bot_ref(app, bot),
        project_id: bot.project_id.clone(),
    });
    if bot.is_linked() {
        return;
    }
    for peer_id in app.db.peers_exposed_to(&bot.id).unwrap_or_default() {
        app.peers.notify(
            &peer_id,
            json!({ "type": "owner_thread_updated", "bot_id": bot.id }),
        );
    }
}

/// The open questions of a project or a bot here. A card question also
/// closes when the owner has commented on the card or the card is closed.
pub fn open_questions(
    app: &AppState,
    project_id: Option<&str>,
    bot_id: Option<&str>,
) -> anyhow::Result<Vec<OwnerQuestion>> {
    let mut open = Vec::new();
    for q in app.db.open_owner_questions(project_id, bot_id)? {
        if let Some(item_id) = &q.item_id {
            if cards::answered(app, item_id, &q)? {
                continue;
            }
        }
        open.push(q);
    }
    Ok(open)
}

/// `attention_dismiss` of an owner question here.
pub fn dismiss(app: &AppState, row_id: &str) -> anyhow::Result<AttentionDismissed> {
    let me = app.db.daemon_id()?;
    let mut parts = row_id.splitn(3, ':');
    let (_, daemon_id, question_id) = (parts.next(), parts.next(), parts.next());
    let (Some(daemon_id), Some(question_id)) = (daemon_id, question_id) else {
        return Err(invalid(format!("{row_id} isn't a row id")));
    };
    if daemon_id != me {
        return Err(invalid(
            "that question is on another computer: dismiss it there (the row's daemon_id)",
        ));
    }
    let question = app
        .db
        .owner_question(question_id)?
        .ok_or_else(|| not_found(format!("no open owner question {row_id}")))?;
    if app.db.dismiss_owner_question(question_id)? {
        if let Err(e) = cards::tell_dismissed(app, &question) {
            tracing::warn!(question_id, error = %e, "couldn't tell the bot its question was dismissed");
        }
    }
    if let Some(bot) = app.db.get_bot(&question.bot_id)? {
        updated(app, &bot);
    }
    Ok(AttentionDismissed {
        id: row_id.to_string(),
    })
}

/// The thread entry of a bot that runs here.
pub fn entry(app: &AppState, bot: &bus::Bot) -> anyhow::Result<OwnerThread> {
    let asked = asked_nums(app, &bot.id)?;
    Ok(OwnerThread {
        bot: Some(bot_ref(app, bot)),
        project_id: bot.project_id.clone(),
        last: app.db.owner_thread_last(&bot.id)?.map(|m| MessageBrief {
            num: m.num,
            from_owner: m.sender.kind == SenderKind::User,
            text: cut(&m.body, BRIEF_MAX),
            at: Some(timestamp(m.created_at)),
            asks: asked.iter().any(|(num, _)| *num == m.num),
        }),
        unread: app.db.owner_unread(&bot.id)?,
        open_question: !open_questions(app, None, Some(&bot.id))?.is_empty(),
    })
}

/// The thread messages that asked, and whether each question is open.
fn asked_nums(app: &AppState, bot_id: &str) -> anyhow::Result<Vec<(i64, bool)>> {
    let open: Vec<String> = open_questions(app, None, Some(bot_id))?
        .into_iter()
        .map(|q| q.id)
        .collect();
    Ok(app
        .db
        .thread_questions(bot_id)?
        .into_iter()
        .map(|(num, id)| (num, open.contains(&id)))
        .collect())
}

/// A page of the thread of a bot that runs here.
pub fn page(
    app: &AppState,
    bot: &bus::Bot,
    before_num: Option<i64>,
    limit: Option<u32>,
) -> anyhow::Result<OwnerThreadPage> {
    let limit = limit.unwrap_or(PAGE_DEFAULT).clamp(1, PAGE_MAX);
    let (messages, has_more) = app.db.owner_thread_page(&bot.id, before_num, limit)?;
    let asked = asked_nums(app, &bot.id)?;
    Ok(OwnerThreadPage {
        bot: Some(bot_ref(app, bot)),
        project_id: bot.project_id.clone(),
        messages: messages
            .into_iter()
            .map(|m| {
                let question = asked.iter().find(|(num, _)| *num == m.num);
                ThreadMessage {
                    num: m.num,
                    id: m.id,
                    from_owner: m.sender.kind == SenderKind::User,
                    text: m.body,
                    at: Some(timestamp(m.created_at)),
                    asks: question.is_some(),
                    open: question.is_some_and(|(_, open)| *open),
                }
            })
            .collect(),
        last_read_num: app.db.owner_last_read(&bot.id)?,
        has_more,
    })
}

/// Marks the thread of a bot that runs here read up to `up_to_num`.
pub fn mark_read(
    app: &AppState,
    bot: &bus::Bot,
    up_to_num: i64,
) -> anyhow::Result<OwnerThreadMarked> {
    let last_read_num = app.db.mark_owner_read(&bot.id, up_to_num)?;
    updated(app, bot);
    Ok(OwnerThreadMarked {
        bot_id: bot.id.clone(),
        last_read_num,
        unread: app.db.owner_unread(&bot.id)?,
    })
}

/// The live bot a client names: by its id here, or by its own computer's id
/// (`BotRef.bot_id`) when it is a stand-in here.
pub fn resolve(app: &Arc<AppState>, bot_id: &str) -> anyhow::Result<bus::Bot> {
    if let Some(bot) = app.db.get_live_bot(bot_id)? {
        return Ok(bot);
    }
    app.db
        .list_bots(None)?
        .into_iter()
        .find(|b| b.remote_bot_id.as_deref() == Some(bot_id))
        .ok_or_else(|| not_found(format!("no bot {bot_id}")))
}
