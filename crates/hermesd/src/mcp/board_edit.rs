//! The board tools that change an item. Each runs in one board transaction:
//! load the item and the caller's roles, check the version, run the guard,
//! write (ARCH-R4 §3).

use std::sync::Arc;

use bus::contract::board as c;
use serde_json::{json, Value};

use crate::app::AppState;
use crate::board::contract::model_list;
use crate::board::guards::{self, Who};
use crate::board::model::{Item, ItemType, LinkKind, Platform, Priority, Size, Unmet};
use crate::board::moves::load_in;
use crate::db::{BoardTx, ItemEdit, NewItem, Write};

use super::board::{bot_id, conflict, item_out, out, own_item, refused, written, Me};
use super::board_schema::decode;

pub(super) fn call(
    app: &Arc<AppState>,
    me: &Me,
    name: &str,
    args: &Value,
) -> anyhow::Result<Value> {
    let project = me.bot.project_id.as_str();
    match name {
        "item_create" => create(app, me, decode("ItemCreateRequest", args, project)?),
        "item_update" => update(app, me, decode("ItemUpdateRequest", args, project)?),
        "item_comment" => comment(app, me, decode("ItemCommentRequest", args, project)?),
        "item_link" => link(app, me, decode("ItemLinkRequest", args, project)?),
        "item_unlink" => unlink(app, me, decode("ItemUnlinkRequest", args, project)?),
        "item_block" => block(app, me, decode("ItemBlockRequest", args, project)?),
        "item_unblock" => unblock(app, me, decode("ItemUnblockRequest", args, project)?),
        "item_assign" => assign(app, me, decode("ItemAssignRequest", args, project)?),
        "item_rank" => rank(app, me, decode("ItemRankRequest", args, project)?),
        "item_check_ac" => check_ac(app, me, decode("ItemCheckAcRequest", args, project)?),
        other => anyhow::bail!("unknown tool: {other}"),
    }
}

/// What a guarded edit decided: refused, stale, or written.
enum Edit {
    Refused(Vec<Unmet>),
    Stale(Item),
    Written(Write<Item>),
}

/// Load, check the version, guard and write, in one transaction.
fn guarded(
    app: &Arc<AppState>,
    me: &Me,
    id: &str,
    expected: u64,
    guard: impl FnOnce(&Item, &Who) -> Vec<Unmet>,
    write: impl FnOnce(&BoardTx<'_>) -> anyhow::Result<Write<Item>>,
) -> anyhow::Result<Value> {
    own_item(app, me, id)?;
    let edit = app.db.board_tx(|t| {
        let (item, _, who) = load_in(t, id, &me.actor())?;
        if item.version != expected {
            return Ok(Edit::Stale(item));
        }
        let unmet = guard(&item, &who);
        if !unmet.is_empty() {
            return Ok(Edit::Refused(unmet));
        }
        Ok(Edit::Written(write(t)?))
    })?;
    match edit {
        Edit::Refused(unmet) => Err(refused(unmet)),
        Edit::Stale(item) => Err(conflict(item)),
        Edit::Written(w) => written(w),
    }
}

fn create(app: &Arc<AppState>, me: &Me, req: c::ItemCreateRequest) -> anyhow::Result<Value> {
    let unmet = guards::check_new(&req.title);
    if !unmet.is_empty() {
        return Err(refused(unmet));
    }
    if let Some(parent) = &req.parent_id {
        own_item(app, me, parent)?;
    }
    let platforms = model_list(req.platforms, Platform::from_wire)?;
    let item = app.db.create_item(
        &NewItem {
            project_id: &me.bot.project_id,
            item_type: ItemType::from_wire(req.r#type)?,
            title: req.title.trim(),
            description: req.description.as_deref().unwrap_or_default(),
            platforms: &platforms,
            size: req.size.map(Size::from_wire).transpose()?,
            priority: req
                .priority
                .map(Priority::from_wire)
                .transpose()?
                .unwrap_or(Priority::P2),
            labels: &req.labels,
            parent_id: req.parent_id.as_deref(),
            acceptance_criteria: &req.acceptance_criteria,
        },
        &me.actor(),
    )?;
    Ok(json!({ "item": item_out(item)? }))
}

fn update(app: &Arc<AppState>, me: &Me, req: c::ItemUpdateRequest) -> anyhow::Result<Value> {
    if let Some(parent) = req.parent_id.as_deref().filter(|p| !p.is_empty()) {
        own_item(app, me, parent)?;
    }
    let platforms = req
        .platforms
        .map(|l| model_list(l.values, Platform::from_wire))
        .transpose()?;
    let priority = req.priority.map(Priority::from_wire).transpose()?;
    let edit = ItemEdit {
        title: req.title.as_deref(),
        description: req.description.as_deref(),
        platforms: platforms.as_deref(),
        size: req.size.map(Size::from_wire).transpose()?.map(Some),
        priority,
        labels: req.labels.as_ref().map(|l| l.values.as_slice()),
        // An empty parent clears it.
        parent_id: req
            .parent_id
            .as_deref()
            .map(|p| (!p.is_empty()).then_some(p)),
    };
    let guard = |item: &Item, who: &Who| {
        // Raising to P0 is the lead's or the owner's call (H-017 §3).
        let to_p0 = priority == Some(Priority::P0) && item.priority != Priority::P0;
        if to_p0 {
            guards::check_lead(who)
        } else {
            Vec::new()
        }
    };
    let actor = me.actor();
    guarded(app, me, &req.id, req.expected_version, guard, |t| {
        t.update_item(&req.id, req.expected_version, &edit, &actor)
    })
}

fn comment(app: &Arc<AppState>, me: &Me, req: c::ItemCommentRequest) -> anyhow::Result<Value> {
    own_item(app, me, &req.id)?;
    anyhow::ensure!(!req.body.trim().is_empty(), "'body' is empty");
    let comment =
        app.db
            .add_item_comment(&req.id, &req.body, req.reply_to.as_deref(), &me.actor())?;
    Ok(json!({ "comment": out(c::ItemComment::from(comment))? }))
}

fn link(app: &Arc<AppState>, me: &Me, req: c::ItemLinkRequest) -> anyhow::Result<Value> {
    own_item(app, me, &req.id)?;
    let kind = LinkKind::from_wire(req.kind)?;
    if matches!(
        kind,
        LinkKind::ItemBlocks | LinkKind::ItemRelates | LinkKind::ItemDuplicates
    ) {
        own_item(app, me, &req.r#ref)?;
        anyhow::ensure!(req.r#ref != req.id, "an item can't link to itself");
    }
    let link =
        app.db
            .add_item_link(&req.id, kind, &req.r#ref, req.label.as_deref(), &me.actor())?;
    Ok(json!({ "link": out(c::ItemLink::from(link))? }))
}

fn unlink(app: &Arc<AppState>, me: &Me, req: c::ItemUnlinkRequest) -> anyhow::Result<Value> {
    own_item(app, me, &req.id)?;
    let kind = LinkKind::from_wire(req.kind)?;
    let removed = app
        .db
        .remove_item_link(&req.id, kind, &req.r#ref, &me.actor())?;
    Ok(json!({ "removed": removed }))
}

fn block(app: &Arc<AppState>, me: &Me, req: c::ItemBlockRequest) -> anyhow::Result<Value> {
    if let Some(by) = &req.by {
        own_item(app, me, by)?;
    }
    let actor = me.actor();
    let reason = req.reason.trim();
    let guard = |item: &Item, who: &Who| guards::check_block(item, who, true, Some(reason));
    guarded(app, me, &req.id, req.expected_version, guard, |t| {
        t.block_item(
            &req.id,
            req.expected_version,
            Some((req.by.as_deref(), reason)),
            &actor,
        )
    })
}

fn unblock(app: &Arc<AppState>, me: &Me, req: c::ItemUnblockRequest) -> anyhow::Result<Value> {
    let actor = me.actor();
    let guard = |item: &Item, who: &Who| guards::check_block(item, who, false, None);
    guarded(app, me, &req.id, req.expected_version, guard, |t| {
        t.block_item(&req.id, req.expected_version, None, &actor)
    })
}

fn assign(app: &Arc<AppState>, me: &Me, req: c::ItemAssignRequest) -> anyhow::Result<Value> {
    let assignee = req.bot.as_deref().map(|b| bot_id(app, me, b)).transpose()?;
    let actor = me.actor();
    let guard = |_: &Item, who: &Who| guards::check_lead(who);
    guarded(app, me, &req.id, req.expected_version, guard, |t| {
        t.assign_item(&req.id, req.expected_version, assignee.as_deref(), &actor)
    })
}

fn rank(app: &Arc<AppState>, me: &Me, req: c::ItemRankRequest) -> anyhow::Result<Value> {
    for other in [&req.after, &req.before].into_iter().flatten() {
        own_item(app, me, other)?;
    }
    let actor = me.actor();
    let guard = |_: &Item, who: &Who| guards::check_lead(who);
    guarded(app, me, &req.id, req.expected_version, guard, |t| {
        let (after, before) = (req.after.as_deref(), req.before.as_deref());
        t.rank_item(&req.id, req.expected_version, after, before, &actor)
    })
}

fn check_ac(app: &Arc<AppState>, me: &Me, req: c::ItemCheckAcRequest) -> anyhow::Result<Value> {
    let passed = c::VerificationResult::try_from(req.result) == Ok(c::VerificationResult::Pass);
    let actor = me.actor();
    guarded(
        app,
        me,
        &req.id,
        req.expected_version,
        guards::check_ac,
        |t| {
            t.check_ac(
                &req.id,
                req.expected_version,
                req.index,
                passed,
                req.machine.as_deref(),
                &actor,
            )
        },
    )
}
