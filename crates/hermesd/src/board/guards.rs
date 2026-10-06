//! The move guards (H-017 §3): one pure function that says what stands
//! between an item and a column. `moves::item_move` runs it before it commits
//! and `moves::item_move_check` runs it for every column, so the UI shows
//! these texts and never re-implements a rule. Nothing here reads storage:
//! the caller passes the facts in a `Context`.

use std::collections::BTreeMap;

pub use ac::{check_ac, check_ac_edit, flag_ac, needs_evidence};
use conditions::conditions;
pub use conditions::{check_template, open_post_install, DOR_FIELDS};

use super::model::{
    BoardColumn, ColumnCategory as Cat, Item, ItemLink, LinkKind, PersonRole, Platform, Role,
    Unmet, WipScope,
};

/// The label a card carries while it sits over a WIP limit by override.
pub const WIP_OVERRIDE_LABEL: &str = "wip-override";

/// The override a returned item gets automatically when it puts its
/// assignee over the limit (H-017 rev 2.1 §1.3).
pub const RETURNED_OVER_WIP: &str = "returned: rework over WIP";

/// Who is acting, as the guards see it.
#[derive(Debug, Clone, PartialEq)]
pub enum Who {
    /// The owner, who may make any move a rule allows anyone.
    Owner,
    /// The daemon itself, carrying out an owner's release ruling.
    Daemon,
    /// A bot of the item's project, with its board roles.
    Bot { id: String, roles: Vec<Role> },
}

impl Who {
    fn has(&self, role: Role) -> bool {
        matches!(self, Who::Bot { roles, .. } if roles.contains(&role))
    }

    fn is(&self, bot_id: Option<&str>) -> bool {
        matches!(self, Who::Bot { id, .. } if Some(id.as_str()) == bot_id)
    }

    fn named(&self, item: &Item, role: PersonRole) -> Option<bool> {
        let named: Vec<&str> = item
            .people
            .iter()
            .filter(|p| p.role == role)
            .map(|p| p.bot_id.as_str())
            .collect();
        (!named.is_empty()).then(|| named.iter().any(|id| self.is(Some(id))))
    }

    /// A reviewer named on the item, or any reviewer when none is named. A
    /// bot tasked through the item (a linked task) is named too, its
    /// assignee aside (H-099).
    fn reviews(&self, item: &Item, ctx: &Context) -> bool {
        let tasked = matches!(self, Who::Bot { id, .. } if ctx.task_holders.contains(id))
            && !self.is(item.assignee.as_deref());
        tasked
            || self
                .named(item, PersonRole::Reviewer)
                .unwrap_or_else(|| self.has(Role::ReviewerArch) || self.has(Role::ReviewerUx))
    }

    fn verifies(&self, item: &Item) -> bool {
        self.has(Role::Tester) || self.named(item, PersonRole::Verifier) == Some(true)
    }

    fn leads(&self) -> bool {
        matches!(self, Who::Owner) || self.has(Role::Lead)
    }
}

/// An item that blocks the one being moved, and where it stands.
#[derive(Debug, Clone, PartialEq)]
pub struct Blocker {
    pub id: String,
    pub category: Cat,
}

/// How many items a column holds, the moving item left out; `assignee_items`
/// counts only those of the moving item's assignee.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ColumnLoad {
    pub items: u32,
    pub assignee_items: u32,
}

/// What the guards need to know besides the item.
#[derive(Debug, Clone, Default)]
pub struct Context {
    pub links: Vec<ItemLink>,
    pub blockers: Vec<Blocker>,
    /// The Definition of Ready fields of the item's template.
    pub ready: Vec<String>,
    pub required_machines: BTreeMap<Platform, Vec<String>>,
    pub load: BTreeMap<String, ColumnLoad>,
    /// The version of the open release package the item is in, if any.
    pub open_package: Option<String>,
    /// The bots holding an open task linked to the item, from the lead or
    /// the owner.
    pub task_holders: Vec<String>,
}

impl Context {
    fn has_link(&self, kinds: &[LinkKind]) -> bool {
        self.links.iter().any(|l| kinds.contains(&l.kind))
    }
}

/// The outcome a spike or a chore without code closes on: its newest
/// artifact link (H-154).
pub fn outcome_link(ctx: &Context) -> Option<&str> {
    ctx.links
        .iter()
        .filter(|l| l.kind == LinkKind::Artifact)
        .max_by_key(|l| l.at)
        .map(|l| l.target.as_str())
}

/// A requested move.
pub struct Move<'a> {
    pub to: &'a BoardColumn,
    pub reason: Option<&'a str>,
    pub override_reason: Option<&'a str>,
}

/// The H-017 §3 row a move falls under.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rule {
    /// Inbox → Ready.
    Refine,
    /// Ready → Doing.
    Start,
    /// Doing → Review.
    Submit,
    /// Review → Verify.
    Approve,
    /// Review → Doing.
    Rework,
    /// Verify → Owner testing.
    Package,
    /// Verify → Doing.
    Reject,
    /// Doing/Review/Verify → Done, for spikes and non-code chores.
    Finish,
    /// Out of owner testing or deploying (cancelling included), into
    /// deploying, and into Done other than by `Finish`.
    Release,
    /// Ready → Inbox, to un-refine.
    Unrefine,
    /// Any → Cancelled.
    Cancel,
    /// Anything else: the owner, with a reason.
    Unlisted,
}

pub fn rule(from: Cat, to: Cat) -> Rule {
    use Cat::*;
    match (from, to) {
        // Nothing leaves a frozen package or a rollout but the daemon, not
        // even a cancel: B7's repackage and rollback paths move these items.
        (Approval | Deploying, _) => Rule::Release,
        (_, Cancelled) => Rule::Cancel,
        (Inbox, Ready) => Rule::Refine,
        (Ready, Inbox) => Rule::Unrefine,
        (Ready, Doing) => Rule::Start,
        (Doing, Review) => Rule::Submit,
        (Review, Verify) => Rule::Approve,
        (Review, Doing) => Rule::Rework,
        (Verify, Approval) => Rule::Package,
        (Verify, Doing) => Rule::Reject,
        (Doing | Review | Verify, Done) => Rule::Finish,
        (_, Deploying | Done) => Rule::Release,
        _ => Rule::Unlisted,
    }
}

/// The owner closing an item out of Verify that is in no release package
/// (ARCH-R22 F1): until B7 builds packages, nothing else takes shipped work
/// to Done. An item with a release, or in a package still being assembled
/// (ARCH-R24), stays the release's to move, and a move `Finish` already
/// allows needs no escape.
pub fn closes_without_release(item: &Item, mv: &Move<'_>, who: &Who, ctx: &Context) -> bool {
    would_close_without_release(item, mv, who, ctx) && ctx.open_package.is_none()
}

/// `closes_without_release` without the open package check.
fn would_close_without_release(item: &Item, mv: &Move<'_>, who: &Who, ctx: &Context) -> bool {
    *who == Who::Owner
        && item.category == Cat::Verify
        && mv.to.category == Cat::Done
        && item.release_id.is_none()
        && !conditions::finish(item, ctx).is_empty()
}

/// Work sent back for rework: never refused for WIP, and placed first.
pub fn is_return(rule: Rule) -> bool {
    matches!(rule, Rule::Rework | Rule::Reject)
}

pub(crate) fn unmet(code: &str, text: impl Into<String>, fix: Option<&str>) -> Unmet {
    Unmet {
        code: code.to_string(),
        text: text.into(),
        fix: fix.map(str::to_string),
    }
}

fn has_text(text: Option<&str>) -> bool {
    text.is_some_and(|t| !t.trim().is_empty())
}

/// Everything that stands between `item` and the move; empty means allowed.
pub fn evaluate(item: &Item, mv: &Move<'_>, who: &Who, ctx: &Context) -> Vec<Unmet> {
    if mv.to.key == item.column_key {
        return vec![unmet(
            "move.same_column",
            "It is already in this column.",
            None,
        )];
    }
    if *who == Who::Daemon {
        return Vec::new();
    }
    let rule = rule(item.category, mv.to.category);
    if rule == Rule::Release && conditions::closes_too_early(item, mv, ctx) {
        return vec![conditions::not_started()];
    }
    if rule == Rule::Release {
        return vec![unmet(
            "move.daemon_only",
            "Only the daemon moves items through owner testing and deploying, on the owner's ruling.",
            Some(if matches!(item.category, Cat::Approval | Cat::Deploying) {
                "Ask DevOps to repackage without it, or roll the release back."
            } else {
                "Add it to a release package; the owner's verdict moves it."
            }),
        )];
    }
    let mut out = Vec::new();
    if let Some(who_may) = refused(rule, item, who, ctx) {
        out.push(unmet(
            "role.not_allowed",
            format!("Only {who_may} can move it to {}.", mv.to.name),
            None,
        ));
    }
    conditions(rule, item, mv, who, ctx, &mut out);
    if mv.to.category == Cat::Done {
        conditions::post_install_done(item, &mut out);
    }
    if !is_return(rule) {
        out.extend(wip(item, mv, who, ctx));
    }
    out
}

/// Who may make the move, when `who` may not.
fn refused(rule: Rule, item: &Item, who: &Who, ctx: &Context) -> Option<&'static str> {
    if *who == Who::Owner {
        return None;
    }
    let assignee = who.is(item.assignee.as_deref());
    let (ok, who_may) = match rule {
        Rule::Refine | Rule::Unrefine | Rule::Cancel => (who.leads(), "the lead or the owner"),
        Rule::Start => (
            who.leads() || assignee,
            "the lead, or the assignee the lead picked",
        ),
        Rule::Submit => (assignee, "the assignee"),
        Rule::Approve => (who.reviews(item, ctx), "a reviewer"),
        Rule::Rework => (
            who.leads() || who.reviews(item, ctx),
            "a reviewer or the lead",
        ),
        Rule::Package => (who.has(Role::Devops), "DevOps"),
        Rule::Reject => (who.leads() || who.verifies(item), "a tester or the lead"),
        // A spike or a chore without code closes on someone else's word
        // (H-154): the lead, or a reviewer who isn't the one who did it.
        Rule::Finish => (
            who.leads() || (who.reviews(item, ctx) && !assignee),
            "the lead, or a reviewer other than the assignee",
        ),
        Rule::Release | Rule::Unlisted => (false, "the owner"),
    };
    (!ok).then_some(who_may)
}

/// The WIP limit the move would break, if any: (limit, whose items count).
pub fn over_limit(item: &Item, to: &BoardColumn, ctx: &Context) -> Option<(u32, String)> {
    let limit = to.wip_limit?;
    let load = ctx.load.get(&to.key).copied().unwrap_or_default();
    let (count, whose) = match to.wip_scope {
        WipScope::Column => (load.items, String::new()),
        WipScope::PerAssignee => (
            load.assignee_items,
            format!(" for {}", item.assignee.as_ref()?),
        ),
    };
    (count >= limit).then_some((limit, whose))
}

/// Over a WIP limit is refused unless the lead or the owner gives a reason.
fn wip(item: &Item, mv: &Move<'_>, who: &Who, ctx: &Context) -> Option<Unmet> {
    let (limit, whose) = over_limit(item, mv.to, ctx)?;
    if who.leads() && has_text(mv.override_reason) {
        return None;
    }
    Some(unmet(
        "wip.full",
        format!("{} is at its WIP limit of {limit}{whose}.", mv.to.name),
        Some(if who.leads() {
            "Give an override reason; it shows on the card."
        } else {
            "Finish something first, or ask the lead to override."
        }),
    ))
}

/// — → Inbox: a new item needs a title (its type is always given).
pub fn check_new(title: &str) -> Vec<Unmet> {
    let mut out = Vec::new();
    if title.trim().is_empty() {
        out.push(unmet("new.title", "It needs a title.", None));
    }
    out
}

/// Setting or clearing `blocked`: the assignee, the lead or the owner, with a
/// reason when setting.
pub fn check_block(item: &Item, who: &Who, setting: bool, reason: Option<&str>) -> Vec<Unmet> {
    let mut out = Vec::new();
    if !(who.leads() || *who == Who::Daemon || who.is(item.assignee.as_deref())) {
        out.push(unmet(
            "role.not_allowed",
            "Only the assignee, the lead or the owner can.",
            None,
        ));
    }
    if setting && !has_text(reason) {
        out.push(unmet(
            "reason.required",
            "Say what blocks it.",
            Some("Give a reason."),
        ));
    }
    out
}

/// Ranking, P0, and editing WIP limits, columns or templates: the lead or the owner.
pub fn check_lead(who: &Who) -> Vec<Unmet> {
    if who.leads() || *who == Who::Daemon {
        Vec::new()
    } else {
        vec![unmet(
            "role.not_allowed",
            "Only the lead or the owner can.",
            None,
        )]
    }
}

mod ac;
mod conditions;
#[cfg(test)]
mod tests;
