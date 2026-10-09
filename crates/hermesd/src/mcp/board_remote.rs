//! Board tools for a bot whose project's board lives on a peer (B9, H-020
//! §1.3): testers on imac and win-pc. The call goes to the board's home as
//! `board_call` and runs there as the bot's stand-in. While the home is
//! unreachable the board is read-only, from the last snapshot mirrored here.

use std::sync::Arc;

use bus::contract::board as c;
use bus::Bot;
use serde_json::{json, Value};

use crate::app::AppState;
use crate::board::mirror::Mirrored;
use crate::peer::board::home_name;
use crate::peer::PeerError;

use super::board::{is_board_tool, out};
use super::caller;

/// The tool's result when the bot's board lives on a peer, or `None` to
/// handle it here as usual (this project has a board, or no link).
pub(super) async fn intercept(
    app: &Arc<AppState>,
    bot_id: &str,
    name: &str,
    args: &Value,
) -> Option<anyhow::Result<Value>> {
    if !is_board_tool(name) {
        return None;
    }
    let bot = caller(app, bot_id).ok()?;
    if app.db.board_settings(&bot.project_id).ok()?.is_some() {
        return None;
    }
    let mut links = app.db.project_links(&bot.project_id).ok()?;
    if links.is_empty() {
        return None;
    }
    // The known home first; the others answer that they hold no board.
    let home = app.board_mirror.home_peer(&bot.project_id);
    links.sort_by_key(|link| Some(&link.peer_id) != home.as_ref());
    if name == "install_quiesce" {
        return Some(quiesce_here(app, &bot, &links, args).await);
    }
    if let Some(mut result) = forward(app, &bot, &links, name, args).await {
        // The checkout of a check this computer ran goes once it's reported.
        if let (Ok(answer), "check_report") = (&result, name) {
            crate::prs::check_checkout::after_report(&bot, answer);
        }
        // How to run it here, with this computer's own binary.
        if let (Ok(answer), "install_release") = (&mut result, name) {
            super::releases::with_command(answer);
        }
        return Some(result);
    }
    Some(match app.board_mirror.get(&bot.project_id) {
        Some(board) => offline(app, &bot, &board, name, args),
        None => Err(anyhow::anyhow!(
            "this project's board lives on a linked computer that can't be reached, or \
             hasn't been started yet; try again later"
        )),
    })
}

/// `name` run on the board's home as the bot's stand-in, or `None` when no
/// linked computer holds the board or can be reached.
async fn forward(
    app: &Arc<AppState>,
    bot: &Bot,
    links: &[bus::ProjectLink],
    name: &str,
    args: &Value,
) -> Option<anyhow::Result<Value>> {
    for link in links {
        let frame = json!({
            "type": "board_call", "project_id": link.project_id,
            "bot_id": bot.id, "tool": name, "args": args,
        });
        match app.peers.request(&link.peer_id, frame).await {
            Ok(mut result) => {
                crate::peer::board::ids_from_home(app, link, &mut result);
                // The home's stale-write answer, now in this computer's ids.
                if let Some(conflict) = super::Conflict::from_result(&result) {
                    return Some(Err(conflict.into()));
                }
                return Some(Ok(result));
            }
            Err(PeerError::Refused { code, .. }) if code == "no_board" => {}
            Err(PeerError::Offline) => {}
            Err(e) => return Some(Err(anyhow::anyhow!("{e}"))),
        }
    }
    None
}

/// `install_quiesce` pauses the computer the install runs on, this one
/// (H-166): the home only checks the deploy task, as `install_release`
/// does, and never pauses itself for another computer's install.
async fn quiesce_here(
    app: &Arc<AppState>,
    bot: &Bot,
    links: &[bus::ProjectLink],
    args: &Value,
) -> anyhow::Result<Value> {
    let text = |key: &str| args[key].as_str().map(str::trim).filter(|s| !s.is_empty());
    let ask = crate::quiesce::tool::Ask {
        action: text("action").unwrap_or_default(),
        release_id: text("release_id").unwrap_or_default(),
        version: text("version"),
        binary_sha256: text("binary_sha256"),
    };
    let answer = if ask.action == "status" {
        Value::Null
    } else {
        crate::quiesce::tool::may_pause(app, bot)?;
        let gate = json!({ "release_id": ask.release_id });
        forward(app, bot, links, "install_release", &gate)
            .await
            .unwrap_or_else(|| {
                Err(anyhow::anyhow!(
                    "the board's home can't be reached to check your deploy task; try again \
                     when it's back"
                ))
            })?
    };
    crate::quiesce::tool::run(app, bot, &ask, &|| Ok(answer.clone()))
}

/// The home is unreachable: reads from the last snapshot, writes refused.
fn offline(
    app: &AppState,
    bot: &Bot,
    board: &Mirrored,
    name: &str,
    args: &Value,
) -> anyhow::Result<Value> {
    let home = home_name(app, &board.peer_id);
    let note = format!(
        "{home} holds this board and can't be reached: this is the board as last seen, read-only"
    );
    let snapshot = &board.snapshot;
    match name {
        "board_get" => {
            let columns = snapshot
                .columns
                .iter()
                .filter(|col| col.visible)
                .map(|col| {
                    let mut v = out(col)?;
                    v["count"] = json!(cards_in(snapshot, &col.key).count());
                    Ok(v)
                })
                .collect::<anyhow::Result<Vec<_>>>()?;
            let key = snapshot.settings.as_ref().map(|s| s.key.clone());
            Ok(json!({
                "key": key, "columns": columns, "cards": out(&snapshot.cards)?,
                "offline": note,
            }))
        }
        "item_query" => {
            let columns = string_list(args, "column_keys");
            let text = args
                .get("text")
                .and_then(Value::as_str)
                .map(str::to_lowercase);
            let assignee = args
                .get("assignee")
                .and_then(Value::as_str)
                .map(|a| bot_by_name(app, bot, a));
            let cards: Vec<&c::ItemCard> = snapshot
                .cards
                .iter()
                .filter(|card| columns.is_empty() || columns.contains(&card.column_key))
                .filter(|card| {
                    text.as_ref().is_none_or(|t| {
                        card.title.to_lowercase().contains(t) || card.id.to_lowercase() == *t
                    })
                })
                .filter(|card| assignee.is_none() || card.assignee == assignee)
                .collect();
            Ok(json!({ "cards": out(cards)?, "offline": note }))
        }
        "item_get" => {
            let id = args.get("id").and_then(Value::as_str).unwrap_or_default();
            let card = snapshot
                .cards
                .iter()
                .find(|card| card.id == id)
                .ok_or_else(|| anyhow::anyhow!("no item {id} in this project"))?;
            Ok(json!({
                "card": out(card)?,
                "offline": format!("{note}; its history and links are on {home}"),
            }))
        }
        _ => anyhow::bail!(
            "The board lives on {home}, which is unreachable. Try again when it's back."
        ),
    }
}

fn cards_in<'a>(
    board: &'a c::BoardSnapshot,
    column: &'a str,
) -> impl Iterator<Item = &'a c::ItemCard> {
    board
        .cards
        .iter()
        .filter(move |card| card.column_key == column)
}

fn string_list(args: &Value, key: &str) -> Vec<String> {
    args.get(key)
        .and_then(Value::as_array)
        .map(|v| {
            v.iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// A bot of the caller's project by id or name, as this daemon knows it.
fn bot_by_name(app: &AppState, me: &Bot, name_or_id: &str) -> String {
    app.db
        .list_bots(Some(&me.project_id))
        .unwrap_or_default()
        .into_iter()
        .find(|b| b.id == name_or_id || b.name.eq_ignore_ascii_case(name_or_id))
        .map_or_else(|| name_or_id.to_string(), |b| b.id)
}
