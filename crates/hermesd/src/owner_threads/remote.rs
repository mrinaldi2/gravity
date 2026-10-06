//! Owner threads across computers (R2.2 "Linked bots"). A client asks any
//! computer; a stand-in's thread is read from the bot's own computer through
//! a peer request of the same name, and the bot's computer tells linked
//! peers `owner_thread_updated` so they push it to their clients.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use bus::contract::home::{OwnerThread, OwnerThreadMarked, OwnerThreadPage, OwnerThreads};
use bus::Peer;
use serde_json::{json, Value};

use super::{bot_ref, entry, mark_read, page, resolve};
use crate::app::AppState;
use crate::events::Push;

/// How long a peer may take to answer for its bots' threads.
const PEER_TIMEOUT: Duration = Duration::from_secs(3);

/// `owner_threads`: one entry per bot of this computer's projects, stand-ins
/// included; sorted open questions first, then the newest message first.
/// A stand-in whose computer doesn't answer is listed without a message.
pub async fn threads(app: Arc<AppState>) -> anyhow::Result<OwnerThreads> {
    let mut threads = Vec::new();
    let mut by_peer: BTreeMap<String, Vec<bus::Bot>> = BTreeMap::new();
    for bot in app.db.list_bots(None)? {
        match (&bot.peer_id, bot.is_linked()) {
            (Some(peer_id), true) => by_peer.entry(peer_id.clone()).or_default().push(bot),
            _ => threads.push(entry(&app, &bot)?),
        }
    }
    let asks = by_peer
        .into_iter()
        .map(|(peer_id, bots)| stand_in_threads(app.clone(), peer_id, bots));
    for part in futures::future::join_all(asks).await {
        threads.extend(part);
    }
    threads.sort_by(|a, b| {
        let at =
            |t: &OwnerThread| crate::attention::key(t.last.as_ref().and_then(|m| m.at.as_ref()));
        b.open_question
            .cmp(&a.open_question)
            .then_with(|| {
                let (a, b) = (at(a), at(b));
                // Newest first; none last.
                a.0.cmp(&b.0).then_with(|| (b.1, b.2).cmp(&(a.1, a.2)))
            })
            .then_with(|| {
                a.bot
                    .as_ref()
                    .map(|r| &r.name)
                    .cmp(&b.bot.as_ref().map(|r| &r.name))
            })
    });
    Ok(OwnerThreads { threads })
}

/// The stand-ins' entries, as their own computer answers for them.
async fn stand_in_threads(
    app: Arc<AppState>,
    peer_id: String,
    bots: Vec<bus::Bot>,
) -> Vec<OwnerThread> {
    let remote_ids: Vec<String> = bots
        .iter()
        .filter_map(|b| b.remote_bot_id.clone())
        .collect();
    let frame = json!({ "type": "owner_threads", "bot_ids": remote_ids });
    let answer = tokio::time::timeout(PEER_TIMEOUT, app.peers.request(&peer_id, frame)).await;
    let answered: Vec<OwnerThread> = match answer {
        Ok(Ok(value)) => serde_json::from_value::<OwnerThreads>(value)
            .map(|t| t.threads)
            .unwrap_or_default(),
        _ => Vec::new(),
    };
    bots.iter()
        .map(|bot| {
            let remote = bot.remote_bot_id.as_deref().unwrap_or_default();
            let found = answered
                .iter()
                .find(|t| t.bot.as_ref().is_some_and(|r| r.bot_id == remote));
            OwnerThread {
                bot: Some(bot_ref(&app, bot)),
                project_id: bot.project_id.clone(),
                ..found.cloned().unwrap_or_default()
            }
        })
        .collect()
}

/// `owner_thread_get`, here or on the bot's own computer.
pub async fn get(
    app: Arc<AppState>,
    bot_id: &str,
    before_num: Option<i64>,
    limit: Option<u32>,
) -> anyhow::Result<OwnerThreadPage> {
    let bot = resolve(&app, bot_id)?;
    if !bot.is_linked() {
        return page(&app, &bot, before_num, limit);
    }
    let frame = json!({
        "type": "owner_thread_get", "before_num": before_num, "limit": limit,
    });
    let value = crate::peer::chat::ask(&app, &bot, frame).await?;
    let mut answer: OwnerThreadPage = serde_json::from_value(value)?;
    answer.project_id = bot.project_id.clone();
    answer.bot = Some(bot_ref(&app, &bot));
    Ok(answer)
}

/// `owner_thread_read`, here or on the bot's own computer.
pub async fn read(
    app: Arc<AppState>,
    bot_id: &str,
    up_to_num: i64,
) -> anyhow::Result<OwnerThreadMarked> {
    let bot = resolve(&app, bot_id)?;
    if !bot.is_linked() {
        return mark_read(&app, &bot, up_to_num);
    }
    let frame = json!({ "type": "owner_thread_read", "up_to_num": up_to_num });
    let value = crate::peer::chat::ask(&app, &bot, frame).await?;
    let mut answer: OwnerThreadMarked = serde_json::from_value(value)?;
    answer.bot_id = bot.id.clone();
    Ok(answer)
}

/// On the bot's computer: a peer asks for threads of bots linked to it, in
/// this computer's bot ids.
pub fn serve(app: &AppState, peer: &Peer, frame: &Value) -> anyhow::Result<Value> {
    let linked = |bot_id: &str| -> anyhow::Result<bus::Bot> {
        let bot = app
            .db
            .get_live_bot(bot_id)?
            .filter(|b| !b.is_linked())
            .ok_or_else(|| anyhow::anyhow!("no bot with id {bot_id} runs here"))?;
        anyhow::ensure!(
            app.db.is_exposed_to_peer(&peer.id, &bot.id)?,
            "{} is not linked to this daemon",
            bot.name
        );
        Ok(bot)
    };
    let text = |key: &str| frame[key].as_str().unwrap_or_default().to_string();
    match frame["type"].as_str().unwrap_or_default() {
        "owner_threads" => {
            let mut threads = Vec::new();
            for id in frame["bot_ids"].as_array().into_iter().flatten() {
                if let Ok(bot) = linked(id.as_str().unwrap_or_default()) {
                    threads.push(entry(app, &bot)?);
                }
            }
            Ok(serde_json::to_value(OwnerThreads { threads })?)
        }
        "owner_thread_get" => {
            let bot = linked(&text("bot_id"))?;
            let limit = frame["limit"].as_u64().and_then(|n| u32::try_from(n).ok());
            Ok(serde_json::to_value(page(
                app,
                &bot,
                frame["before_num"].as_i64(),
                limit,
            )?)?)
        }
        "owner_thread_read" => {
            let bot = linked(&text("bot_id"))?;
            let up_to = frame["up_to_num"].as_i64().unwrap_or_default();
            Ok(serde_json::to_value(mark_read(app, &bot, up_to)?)?)
        }
        other => anyhow::bail!("unknown owner thread request '{other}'"),
    }
}

/// `owner_thread_updated {bot_id}` from the bot's computer: each stand-in
/// of it here tells its clients.
pub fn receive_updated(app: &AppState, peer_id: &str, frame: &Value) {
    let remote = frame["bot_id"].as_str().unwrap_or_default();
    for bot in app.db.linked_bots_for(peer_id, remote).unwrap_or_default() {
        app.events.push(Push::OwnerThreadUpdated {
            bot: bot_ref(app, &bot),
            project_id: bot.project_id.clone(),
        });
    }
}
