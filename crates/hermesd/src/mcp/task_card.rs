//! No work without a card on the board (H-125, owner ruling 0cdec563).
//!
//! - G1: every task a bot sends or spawns names a board card. A nested task
//!   inherits its parent's card; a root task without `item` is refused.
//! - G2: a root task (the sender holds no open task) is budgeted per card,
//!   under a ceiling across the project (ARCH-R57 M1). The chain, hop and
//!   loop checks are unchanged for every task.
//! - A parent with no card — opened before this rule, or forwarded by an
//!   older peer — does not block nested sends until it closes (M2, M3).
//! - A project with no board anywhere keeps the old per-chain budget.
//! - Off the board's home the card is checked there, by `board_call`, and a
//!   send is refused while the home is unreachable (M2).

use std::sync::Arc;

use bus::{Bot, Task};
use serde_json::{json, Value};

use crate::app::AppState;
use crate::board::feed::ChangeKind;
use crate::board::model::{ColumnCategory, Role};
use crate::peer::PeerError;

use super::caller;
use super::tasks::describe_tasks;

pub(super) const NEEDS_CARD: &str = "refusing send: every task needs a board card — pass \
     `item` (find one with item_query, or create one with item_create if none fits)";

/// A `send_message` of kind task, or a routine (H-135 G5), naming a card on
/// a board that lives on a peer: the card is checked at its home before the
/// call runs. `None` for every other call.
pub(super) async fn intercept(
    app: &Arc<AppState>,
    bot_id: &str,
    name: &str,
    args: &Value,
) -> Option<anyhow::Result<Value>> {
    let task = name == "send_message" && args.get("kind").and_then(Value::as_str) == Some("task");
    if !task && name != "create_routine" && name != "update_routine" {
        return None;
    }
    let id = item_text(args)?;
    let me = caller(app, bot_id).ok()?;
    if !off_home(app, &me.project_id) {
        return None;
    }
    if let Err(e) = check_at_home(app, &me, id).await {
        return Some(Err(e));
    }
    let checked = Some(id.to_string());
    Some(match name {
        "create_routine" => super::routines::create_routine_for(app, bot_id, args, checked),
        "update_routine" => super::routines::update_routine_for(app, bot_id, args, checked),
        _ => super::tools::send(app, bot_id, args, checked),
    })
}

fn item_text(args: &Value) -> Option<&str> {
    args.get("item")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
}

/// This project's board lives on a linked machine, not here.
fn off_home(app: &AppState, project_id: &str) -> bool {
    matches!(app.db.board_settings(project_id), Ok(None))
        && app
            .db
            .project_links(project_id)
            .is_ok_and(|links| !links.is_empty())
}

/// This project has a board, here or at a linked home this daemon has
/// heard from (remembered across restarts, ARCH-R59 a), so its tasks name
/// cards. A project with neither keeps the budgets it had before.
pub(crate) fn has_board(app: &AppState, project_id: &str) -> bool {
    !matches!(app.db.board_settings(project_id), Ok(None))
        || app.board_mirror.home_peer(project_id).is_some()
        || app.db.board_home(project_id).ok().flatten().is_some()
}

/// The `item` a task names, checked: it exists on the project's board and
/// is not Done or Cancelled. Works here and, for a spawn, at a peer home.
pub(super) async fn item(
    app: &Arc<AppState>,
    me: &Bot,
    args: &Value,
) -> anyhow::Result<Option<String>> {
    let Some(id) = item_text(args) else {
        return Ok(None);
    };
    if off_home(app, &me.project_id) {
        check_at_home(app, me, id).await?;
        return Ok(Some(id.to_string()));
    }
    local_item(app, me, id).map(Some)
}

/// An item on this machine's board, open for work.
pub(super) fn local_item(app: &AppState, me: &Bot, id: &str) -> anyhow::Result<String> {
    let item = app
        .db
        .get_item(id)?
        .filter(|_| app.db.item_project(id).ok().flatten().as_ref() == Some(&me.project_id))
        .ok_or_else(|| anyhow::anyhow!("no item {id} in this project"))?;
    if matches!(
        item.category,
        ColumnCategory::Done | ColumnCategory::Cancelled
    ) {
        anyhow::bail!(
            "refusing send: item {id} is {} — pick an open card, or create one with item_create",
            item.category.as_str()
        );
    }
    Ok(item.id)
}

async fn check_at_home(app: &Arc<AppState>, me: &Bot, id: &str) -> anyhow::Result<()> {
    let mut reached = false;
    for link in app.db.project_links(&me.project_id)? {
        let frame = json!({
            "type": "board_call", "project_id": link.project_id,
            "bot_id": me.id, "tool": "item_get", "args": { "id": id },
        });
        match app.peers.request(&link.peer_id, frame).await {
            Ok(result) => {
                let category = result
                    .pointer("/item/category")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_lowercase();
                if category.contains("done") || category.contains("cancelled") {
                    anyhow::bail!(
                        "refusing send: item {id} is closed on the board — pick an open \
                         card, or create one with item_create"
                    );
                }
                return Ok(());
            }
            Err(PeerError::Refused { code, .. }) if code == "no_board" => reached = true,
            Err(PeerError::Offline) => {}
            Err(e) => anyhow::bail!("refusing send: can't use card {id}: {e}"),
        }
    }
    if reached {
        anyhow::bail!("no item {id} on this project's board");
    }
    anyhow::bail!(
        "refusing send: board home offline: can't check the card {id} — try again when \
         it is back"
    )
}

/// The open task a send extends: `parent_task` when given, else the
/// caller's newest (M3). `None` makes the send a root task.
pub(super) fn parent_task(app: &AppState, me: &Bot, args: &Value) -> anyhow::Result<Option<Task>> {
    match args.get("parent_task").and_then(Value::as_str) {
        Some(id) => app
            .db
            .get_task(id)?
            .filter(|t| t.to_bot_id == me.id && t.state == bus::TaskState::Open)
            .map(Some)
            .ok_or_else(|| anyhow::anyhow!("no open task {id} assigned to you")),
        None => app.db.newest_open_task_for(&me.id),
    }
}

/// The card a new task is for: the one named, else the parent's. `None`
/// under a parent that has no card, which is allowed and logged, or in a
/// project without a board.
pub(super) fn card_for(
    app: &AppState,
    me: &Bot,
    item: Option<String>,
    parent: Option<&Task>,
) -> anyhow::Result<Option<String>> {
    if item.is_some() {
        return Ok(item);
    }
    let Some(parent) = parent else {
        if !has_board(app, &me.project_id) {
            // No board here or on a linked machine: nothing to name.
            return Ok(None);
        }
        anyhow::bail!("{NEEDS_CARD}");
    };
    let card = app.db.task_card(&parent.id)?;
    if card.is_none() {
        tracing::info!(
            bot = %me.name, parent = %parent.id,
            "nested task under a task with no card (opened before H-125, or from an older peer)"
        );
    }
    Ok(card)
}

/// A root task's budgets: per card, then across the project (M1).
pub(super) fn check_root(app: &Arc<AppState>, me: &Bot, card: &str) -> anyhow::Result<()> {
    let project = app.db.get_project(&me.project_id)?;
    let limits = app
        .cfg
        .tasks
        .for_project(project.as_ref().map_or("", |p| p.name.as_str()));
    let open = app.db.open_tasks_delegated_by(&me.id, Some(&me.id))?;
    let mut on_card = Vec::new();
    for task in &open {
        if app.db.task_card(&task.id)?.as_deref() == Some(card) {
            on_card.push(task.clone());
        }
    }
    if on_card.len() as i64 >= limits.per_card {
        anyhow::bail!(
            "refusing send: you already have {} open tasks on card {card} — wait for a \
             result, ask a delegate for status with kind 'reply', or close one with \
             cancel_task. Never move the work into a note. Open now: {}",
            limits.per_card,
            describe_tasks(app, &on_card)?
        );
    }
    let is_lead = project.and_then(|p| p.lead_bot_id).as_deref() == Some(me.id.as_str())
        || app
            .db
            .project_roles(&me.project_id)?
            .iter()
            .any(|r| r.role == Role::Lead && r.bot_id == me.id);
    let ceiling = if is_lead {
        limits.root_lead
    } else {
        limits.root
    };
    if open.len() as i64 >= ceiling {
        anyhow::bail!(
            "refusing send: you already have {ceiling} open tasks across this project — \
             the board is telling you to finish something: wait for a result, or close \
             one with cancel_task. Open now: {}",
            describe_tasks(app, &open)?
        );
    }
    Ok(())
}

/// Record a new task's card: linked on the item when the board is here, so
/// it shows in the item's history; otherwise named on the task alone.
pub(super) fn attach(
    app: &Arc<AppState>,
    me: &Bot,
    task_id: &str,
    card: &str,
) -> anyhow::Result<()> {
    if app.db.item_project(card)?.as_deref() != Some(me.project_id.as_str()) {
        return app.db.set_task_card(task_id, card);
    }
    let actor = super::board::bot_actor(me);
    super::board::published(app, &me.project_id, ChangeKind::ItemUpserted, None, || {
        app.db.link_task_item(task_id, card, &actor)?;
        Ok(((), card.to_string()))
    })
}
