//! The board tools that change an item. Each runs in one board transaction:
//! load the item and the caller's roles, check the version, run the guard,
//! write (ARCH-R4 §3).

use std::cell::Cell;
use std::sync::Arc;

use bus::contract::board as c;
use serde_json::{json, Value};

use crate::app::AppState;
use crate::board::contract::model_list;
use crate::board::feed::ChangeKind;
use crate::board::guards::{self, Who};
use crate::board::model::{
    ColumnCategory, Item, ItemType, LinkKind, Platform, Priority, Size, Unmet,
};
use crate::board::moves::load_in;
use crate::db::{BoardTx, ItemEdit, NewItem, Write};

use super::board::{bot_id, conflict, item_out, out, own_item, published, refused, written, Me};
use super::board_schema::decode;

pub(super) fn call(
    app: &Arc<AppState>,
    me: &Me,
    name: &str,
    args: &Value,
) -> anyhow::Result<Value> {
    let project = me.bot.project_id.as_str();
    match name {
        "item_create" => create(app, me, decode("ItemCreate", args, project)?),
        "item_update" => update(app, me, decode("ItemUpdate", args, project)?),
        "item_comment" => comment(app, me, decode("ItemAddComment", args, project)?),
        "item_link" => link(app, me, decode("ItemAddLink", args, project)?),
        "item_unlink" => unlink(app, me, decode("ItemRemoveLink", args, project)?),
        "item_block" => block(app, me, decode("ItemBlock", args, project)?),
        "item_unblock" => unblock(app, me, decode("ItemUnblock", args, project)?),
        "item_assign" => assign(app, me, decode("ItemAssign", args, project)?),
        "item_rank" => rank(app, me, decode("ItemRank", args, project)?),
        "item_check_ac" => check_ac(app, me, decode("ItemCheckAc", args, project)?),
        "item_flag_ac" => flag_ac(app, me, decode("ItemFlagAc", args, project)?),
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
    let project = me.bot.project_id.as_str();
    published(app, project, ChangeKind::ItemUpserted, None, || {
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
            Edit::Written(w) => Ok((written(w)?, id.to_string())),
        }
    })
}

fn create(app: &Arc<AppState>, me: &Me, req: c::ItemCreate) -> anyhow::Result<Value> {
    let unmet = guards::check_new(&req.title);
    if !unmet.is_empty() {
        return Err(refused(unmet));
    }
    if let Some(parent) = &req.parent_id {
        own_item(app, me, parent)?;
    }
    let platforms = model_list(req.platforms, Platform::from_wire)?;
    let project = me.bot.project_id.as_str();
    published(app, project, ChangeKind::ItemUpserted, None, || {
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
        let id = item.id.clone();
        Ok((json!({ "item": item_out(item)? }), id))
    })
}

fn update(app: &Arc<AppState>, me: &Me, req: c::ItemUpdate) -> anyhow::Result<Value> {
    if let Some(parent) = req.parent_id.as_deref().filter(|p| !p.is_empty()) {
        own_item(app, me, parent)?;
    }
    let platforms = req
        .platforms
        .map(|l| model_list(l.values, Platform::from_wire))
        .transpose()?;
    let priority = req.priority.map(Priority::from_wire).transpose()?;
    let criteria: Option<Vec<String>> = req
        .acceptance_criteria
        .map(|l| l.values.iter().map(|t| t.trim().to_string()).collect());
    if let Some(criteria) = &criteria {
        anyhow::ensure!(
            !criteria.iter().any(String::is_empty),
            "an acceptance criterion can't be blank"
        );
        let distinct: std::collections::HashSet<&String> = criteria.iter().collect();
        // A check is kept by text, so two equal texts would share one.
        anyhow::ensure!(
            distinct.len() == criteria.len(),
            "two acceptance criteria have the same text"
        );
    }
    let edit = ItemEdit {
        title: req.title.as_deref(),
        description: req.description.as_deref(),
        platforms: platforms.as_deref(),
        size: req.size.map(Size::from_wire).transpose()?.map(Some),
        priority,
        labels: req.labels.as_ref().map(|l| l.values.as_slice()),
        acceptance_criteria: criteria.as_deref(),
        // An empty parent clears it.
        parent_id: req
            .parent_id
            .as_deref()
            .map(|p| (!p.is_empty()).then_some(p)),
        item_type: req.r#type.map(ItemType::from_wire).transpose()?,
    };
    let retype = edit.item_type;
    let guard = |item: &Item, who: &Who| {
        // Raising to P0 is the lead's or the owner's call (H-017 §3).
        let to_p0 = priority == Some(Priority::P0) && item.priority != Priority::P0;
        // So is changing an item's type, e.g. feature → epic (H-099).
        let retyped = retype.is_some_and(|t| t != item.item_type);
        let mut unmet = if to_p0 || retyped {
            guards::check_lead(who)
        } else {
            Vec::new()
        };
        if criteria.is_some() {
            unmet.extend(guards::check_ac_edit(item));
        }
        unmet
    };
    let actor = me.actor();
    guarded(app, me, &req.id, req.expected_version, guard, |t| {
        t.update_item(&req.id, req.expected_version, &edit, &actor)
    })
}

fn comment(app: &Arc<AppState>, me: &Me, req: c::ItemAddComment) -> anyhow::Result<Value> {
    own_item(app, me, &req.id)?;
    anyhow::ensure!(!req.body.trim().is_empty(), "'body' is empty");
    published(
        app,
        &me.bot.project_id,
        ChangeKind::ItemUpserted,
        None,
        || {
            let reply_to = req.reply_to.as_deref();
            let comment = app
                .db
                .add_item_comment(&req.id, &req.body, reply_to, &me.actor())?;
            let out = json!({ "comment": out(c::ItemComment::from(comment))? });
            Ok((out, req.id.clone()))
        },
    )
}

fn link(app: &Arc<AppState>, me: &Me, req: c::ItemAddLink) -> anyhow::Result<Value> {
    own_item(app, me, &req.id)?;
    let kind = LinkKind::from_wire(req.kind)?;
    if matches!(
        kind,
        LinkKind::ItemBlocks | LinkKind::ItemRelates | LinkKind::ItemDuplicates
    ) {
        own_item(app, me, &req.r#ref)?;
        anyhow::ensure!(req.r#ref != req.id, "an item can't link to itself");
    }
    published(
        app,
        &me.bot.project_id,
        ChangeKind::ItemUpserted,
        None,
        || {
            let label = req.label.as_deref();
            let link = app
                .db
                .add_item_link(&req.id, kind, &req.r#ref, label, &me.actor())?;
            Ok((
                json!({ "link": out(c::ItemLink::from(link))? }),
                req.id.clone(),
            ))
        },
    )
}

fn unlink(app: &Arc<AppState>, me: &Me, req: c::ItemRemoveLink) -> anyhow::Result<Value> {
    own_item(app, me, &req.id)?;
    let kind = LinkKind::from_wire(req.kind)?;
    published(
        app,
        &me.bot.project_id,
        ChangeKind::ItemUpserted,
        None,
        || {
            let removed = app
                .db
                .remove_item_link(&req.id, kind, &req.r#ref, &me.actor())?;
            Ok((json!({ "removed": removed }), req.id.clone()))
        },
    )
}

fn block(app: &Arc<AppState>, me: &Me, req: c::ItemBlock) -> anyhow::Result<Value> {
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

fn unblock(app: &Arc<AppState>, me: &Me, req: c::ItemUnblock) -> anyhow::Result<Value> {
    let actor = me.actor();
    let guard = |item: &Item, who: &Who| guards::check_block(item, who, false, None);
    guarded(app, me, &req.id, req.expected_version, guard, |t| {
        t.block_item(&req.id, req.expected_version, None, &actor)
    })
}

fn assign(app: &Arc<AppState>, me: &Me, req: c::ItemAssign) -> anyhow::Result<Value> {
    let assignee = req.bot.as_deref().map(|b| bot_id(app, me, b)).transpose()?;
    let actor = me.actor();
    let guard = |_: &Item, who: &Who| guards::check_lead(who);
    guarded(app, me, &req.id, req.expected_version, guard, |t| {
        t.assign_item(&req.id, req.expected_version, assignee.as_deref(), &actor)
    })
}

fn rank(app: &Arc<AppState>, me: &Me, req: c::ItemRank) -> anyhow::Result<Value> {
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

fn check_ac(app: &Arc<AppState>, me: &Me, req: c::ItemCheckAc) -> anyhow::Result<Value> {
    let passed = c::VerificationResult::try_from(req.result) == Ok(c::VerificationResult::Pass);
    let actor = me.actor();
    let evidence = req.evidence.as_deref().map(str::trim);
    // The lead's evidence goes on the item as a comment, in the same write.
    let lead_check = Cell::new(false);
    let guard = |item: &Item, who: &Who| {
        lead_check.set(guards::needs_evidence(item, who));
        guards::check_ac(item, who, evidence)
    };
    guarded(app, me, &req.id, req.expected_version, guard, |t| {
        let write = t.check_ac(
            &req.id,
            req.expected_version,
            req.index,
            passed,
            req.machine.as_deref(),
            &actor,
        )?;
        if let (true, Some(evidence)) = (lead_check.get(), evidence) {
            let verb = if passed { "ticked" } else { "marked failed" };
            let body = format!(
                "Acceptance criterion {} {verb} by the lead. Evidence: {evidence}",
                req.index + 1
            );
            t.add_item_comment(&req.id, &body, None, &actor)?;
        }
        Ok(write)
    })
}

fn flag_ac(app: &Arc<AppState>, me: &Me, req: c::ItemFlagAc) -> anyhow::Result<Value> {
    let actor = me.actor();
    guarded(
        app,
        me,
        &req.id,
        req.expected_version,
        guards::flag_ac,
        |t| {
            t.flag_ac(
                &req.id,
                req.expected_version,
                req.index,
                req.post_install,
                &actor,
            )
        },
    )
}

/// An item_move reply. Into Verify it lists the criteria nobody has ticked,
/// so the mover sees what the testers still have to prove (H-116).
pub(super) fn moved(item: Item) -> anyhow::Result<Value> {
    let unticked: Vec<Value> = if item.category == ColumnCategory::Verify {
        item.acceptance_criteria
            .iter()
            .filter(|ac| !ac.checked)
            .map(|ac| json!({ "index": ac.idx, "text": ac.text, "post_install": ac.post_install }))
            .collect()
    } else {
        Vec::new()
    };
    let mut reply = json!({ "item": item_out(item)? });
    if !unticked.is_empty() {
        reply["unticked_ac"] = json!(unticked);
        reply["note"] = json!(
            "These acceptance criteria aren't ticked yet. Tick each one already verified \
             with item_check_ac; flag those provable only after install with item_flag_ac."
        );
    }
    Ok(reply)
}
