//! The board tools (H-017 §4.1, H-020 §1.6): reads and moves here, edits in
//! `board_edit`. Arguments decode from the contract's request messages
//! (`board_schema`); every guarded change runs in one board transaction, and
//! a refusal is a tool error listing the same `Unmet` texts the UI shows.

use std::sync::Arc;

use bus::contract::board as c;
use serde_json::{json, Value};

use crate::actor::Actor;
use crate::app::AppState;
use crate::board::contract::model_list;
use crate::board::feed::{card_after_commit, Change, ChangeKind};
use crate::board::model::{Item, ItemType, Platform, Priority, Role, Unmet};
use crate::board::moves::{item_move as move_item, item_move_check, MoveRequest, Moved};
use crate::db::Write;

use super::board_schema::{decode, friendly, BOARD_TOOLS};
use super::caller;

/// The calling bot, in a project that has a board.
pub(super) struct Me {
    pub bot: bus::Bot,
}

impl Me {
    pub(super) fn actor(&self) -> Actor<'_> {
        bot_actor(&self.bot)
    }
}

/// The bot's board roles, or `None` when its project has no board.
pub(super) fn board_roles(
    app: &Arc<AppState>,
    bot: &bus::Bot,
) -> anyhow::Result<Option<Vec<Role>>> {
    if app.db.board_settings(&bot.project_id)?.is_none() {
        return Ok(None);
    }
    Ok(Some(
        app.db
            .project_roles(&bot.project_id)?
            .into_iter()
            .filter(|r| r.bot_id == bot.id)
            .map(|r| r.role)
            .collect(),
    ))
}

/// The roles `tools/list` shows a bot's tools for: its board roles, or
/// before its project has a board, the role the board will seed for it
/// (H-037). A running session keeps the list it read at its start, so it
/// already holds its tools when the owner starts the board.
pub(super) fn listed_roles(app: &Arc<AppState>, bot: &bus::Bot) -> anyhow::Result<Vec<Role>> {
    if let Some(roles) = board_roles(app, bot)? {
        return Ok(roles);
    }
    let is_lead = app
        .db
        .get_project(&bot.project_id)?
        .is_some_and(|p| p.lead_bot_id.as_deref() == Some(bot.id.as_str()));
    Ok(crate::board::defaults::seed_role(&bot.name, is_lead)
        .into_iter()
        .collect())
}

pub(super) fn is_board_tool(name: &str) -> bool {
    BOARD_TOOLS.iter().any(|t| t.name == name)
}

/// Run a board tool. Tools a bot's roles don't list are refused here too.
pub(super) fn call(
    app: &Arc<AppState>,
    bot_id: &str,
    name: &str,
    args: &Value,
) -> anyhow::Result<Value> {
    let bot = caller(app, bot_id)?;
    let roles =
        board_roles(app, &bot)?.ok_or_else(|| {
        anyhow::anyhow!(
            "this project has no board yet; the owner starts it from the Board tab on its home computer"
        )
    })?;
    let tool = BOARD_TOOLS
        .iter()
        .find(|t| t.name == name)
        .expect("checked by is_board_tool");
    anyhow::ensure!(
        tool.audience.admits(&roles),
        "{name} needs a board role you don't have; ask the lead"
    );
    let me = Me { bot };
    let project = me.bot.project_id.as_str();
    match name {
        "board_get" => board_get(app, &me),
        "item_get" => item_get(app, &me, decode("ItemGet", args, project)?),
        "item_query" => item_query(app, &me, decode("ItemQuery", args, project)?),
        "item_move" => item_move(app, &me, decode("ItemMove", args, project)?),
        "item_move_check" => check(app, &me, decode("ItemMoveCheck", args, project)?),
        _ => super::board_edit::call(app, &me, name, args),
    }
}

/// Contract output, with short enum names.
pub(super) fn out(value: impl serde::Serialize) -> anyhow::Result<Value> {
    Ok(friendly(serde_json::to_value(value)?))
}

pub(super) fn item_out(item: Item) -> anyhow::Result<Value> {
    out(c::Item::from(item))
}

/// A refusal as a tool error: every unmet guard, with its fix.
pub(super) fn refused(unmet: Vec<Unmet>) -> anyhow::Error {
    let lines: Vec<String> = unmet
        .into_iter()
        .map(|u| match u.fix {
            Some(fix) => format!("- {} ({}) Fix: {fix}", u.text, u.code),
            None => format!("- {} ({})", u.text, u.code),
        })
        .collect();
    anyhow::anyhow!("refused:\n{}", lines.join("\n"))
}

/// A versioned write's result: the item, or a conflict naming the current one.
pub(super) fn written(write: Write<Item>) -> anyhow::Result<Value> {
    match write {
        Write::Done(item) => Ok(json!({ "item": item_out(item)? })),
        Write::Conflict(current) => Err(conflict(*current)),
    }
}

pub(super) fn conflict(current: Item) -> anyhow::Error {
    let version = current.version;
    let item = item_out(current).map(|v| v.to_string()).unwrap_or_default();
    anyhow::anyhow!(
        "conflict: the item changed since you read it; it is now version {version}. \
         Check it and retry with expected_version {version}.\n{item}"
    )
}

/// Run a bot's board write holding the feed (B4), and push the changed
/// item's card once it has committed, so open boards see bots' changes. The
/// write returns the item it changed; an error (a refusal, a conflict)
/// changed nothing and pushes nothing.
pub(super) fn published<T>(
    app: &Arc<AppState>,
    project_id: &str,
    kind: ChangeKind,
    from_column: Option<String>,
    write: impl FnOnce() -> anyhow::Result<(T, String)>,
) -> anyhow::Result<T> {
    let mut feed = app.board.writer();
    let (out, item_id) = write()?;
    // Committed: from here on nothing may fail, or the push is lost.
    let card = card_after_commit(&app.db, &item_id);
    feed.publish(Change {
        project_id,
        kind,
        item_id: &item_id,
        card,
        from_column,
    });
    Ok(out)
}

/// An item of the caller's project; another project's items don't exist here.
pub(super) fn own_item(app: &Arc<AppState>, me: &Me, id: &str) -> anyhow::Result<()> {
    match app.db.item_project(id)? {
        Some(p) if p == me.bot.project_id => Ok(()),
        _ => anyhow::bail!("no item {id} in this project"),
    }
}

/// The optional `item` argument of a bus tool (`send_message`,
/// `raise_decision`, `complete_task`): an item of the caller's project.
pub(super) fn item_arg(
    app: &Arc<AppState>,
    bot: &bus::Bot,
    args: &Value,
) -> anyhow::Result<Option<String>> {
    let Some(id) = args
        .get("item")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
    else {
        return Ok(None);
    };
    match app.db.item_project(id)? {
        Some(p) if p == bot.project_id => Ok(Some(id.to_string())),
        _ => anyhow::bail!("no item {id} in this project"),
    }
}

pub(super) fn bot_actor(bot: &bus::Bot) -> Actor<'_> {
    Actor::Bot {
        id: &bot.id,
        project_id: &bot.project_id,
    }
}

/// A bot of the caller's project, by id or name.
pub(super) fn bot_id(app: &Arc<AppState>, me: &Me, name_or_id: &str) -> anyhow::Result<String> {
    let bots = app.db.list_bots(Some(&me.bot.project_id))?;
    bots.iter()
        .find(|b| b.id == name_or_id)
        .or_else(|| {
            bots.iter()
                .find(|b| b.name.eq_ignore_ascii_case(name_or_id))
        })
        .map(|b| b.id.clone())
        .ok_or_else(|| anyhow::anyhow!("no bot '{name_or_id}' in this project"))
}

fn board_get(app: &Arc<AppState>, me: &Me) -> anyhow::Result<Value> {
    let project = &me.bot.project_id;
    let settings = app.db.board_settings(project)?.expect("checked in call");
    let cards = app.db.board_cards(project)?;
    let columns = app
        .db
        .board_columns(project)?
        .into_iter()
        .filter(|col| col.visible)
        .map(|col| {
            let count = cards
                .iter()
                .filter(|card| card.column_key == col.key)
                .count();
            let mut v = out(c::BoardColumn::from(col))?;
            v["count"] = json!(count);
            Ok(v)
        })
        .collect::<anyhow::Result<Vec<_>>>()?;
    let cards = cards
        .into_iter()
        .map(|card| out(c::ItemCard::from(card)))
        .collect::<anyhow::Result<Vec<_>>>()?;
    Ok(json!({ "key": settings.key, "columns": columns, "cards": cards }))
}

fn item_get(app: &Arc<AppState>, me: &Me, req: c::ItemGet) -> anyhow::Result<Value> {
    own_item(app, me, &req.id)?;
    let item = app
        .db
        .get_item(&req.id)?
        .ok_or_else(|| anyhow::anyhow!("no item {}", req.id))?;
    let links = app
        .db
        .item_links(&req.id)?
        .into_iter()
        .map(c::ItemLink::from)
        .collect::<Vec<_>>();
    let mut history = app.db.item_events(&req.id)?;
    // The latest few; the drawer pages the rest.
    let skip = history.len().saturating_sub(20);
    let history: Vec<c::ItemEvent> = history.drain(skip..).map(Into::into).collect();
    Ok(json!({ "item": item_out(item)?, "links": out(links)?, "history": out(history)? }))
}

fn item_query(app: &Arc<AppState>, me: &Me, req: c::ItemQuery) -> anyhow::Result<Value> {
    let project = &me.bot.project_id;
    let cards = match req.text.as_deref().filter(|t| !t.trim().is_empty()) {
        Some(text) => app.db.search_items(project, text)?,
        None => app.db.board_cards(project)?,
    };
    let assignee = req
        .assignee
        .as_deref()
        .map(|a| bot_id(app, me, a))
        .transpose()?;
    let types = model_list(req.types, ItemType::from_wire)?;
    let priorities = model_list(req.priorities, Priority::from_wire)?;
    let platforms = model_list(req.platforms, Platform::from_wire)?;
    let cards: Vec<c::ItemCard> = cards
        .into_iter()
        .filter(|card| req.column_keys.is_empty() || req.column_keys.contains(&card.column_key))
        .filter(|card| types.is_empty() || types.contains(&card.item_type))
        .filter(|card| priorities.is_empty() || priorities.contains(&card.priority))
        .filter(|card| platforms.is_empty() || platforms.iter().any(|p| card.platforms.contains(p)))
        .filter(|card| assignee.is_none() || card.assignee == assignee)
        .filter(|card| req.blocked.is_none_or(|b| card.blocked == b))
        .map(c::ItemCard::from)
        .collect();
    Ok(json!({ "cards": out(cards)? }))
}

fn item_move(app: &Arc<AppState>, me: &Me, req: c::ItemMove) -> anyhow::Result<Value> {
    own_item(app, me, &req.id)?;
    let request = MoveRequest {
        id: &req.id,
        to: &req.to,
        expected_version: req.expected_version,
        reason: req.reason.as_deref(),
        override_reason: req.override_reason.as_deref(),
    };
    let from = app.db.get_item(&req.id)?.map(|item| item.column_key);
    let project = me.bot.project_id.as_str();
    published(
        app,
        project,
        ChangeKind::ItemMoved,
        from,
        || match move_item(&app.db, &request, &me.actor())? {
            Moved::Done(item) => Ok((json!({ "item": item_out(*item)? }), req.id.clone())),
            Moved::Refused(unmet) => Err(refused(unmet)),
            Moved::Conflict(current) => Err(conflict(*current)),
        },
    )
}

fn check(app: &Arc<AppState>, me: &Me, req: c::ItemMoveCheck) -> anyhow::Result<Value> {
    own_item(app, me, &req.id)?;
    let columns = item_move_check(&app.db, &req.id, &me.actor())?
        .into_iter()
        .map(|(col, unmet)| {
            let unmet: Vec<c::Unmet> = unmet.into_iter().map(c::Unmet::from).collect();
            Ok(json!({ "column": col.key, "allowed": unmet.is_empty(), "unmet": out(unmet)? }))
        })
        .collect::<anyhow::Result<Vec<_>>>()?;
    Ok(json!({ "columns": columns }))
}
