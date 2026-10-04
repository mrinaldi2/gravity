//! The conditions of each H-017 §3 row, after the actor check.

use std::collections::BTreeSet;

use super::{has_text, unmet, Context, Move, Rule, Who};
use crate::board::model::{
    ColumnCategory as Cat, Item, ItemType, LinkKind, Platform, Size, Unmet, VerificationResult,
};

pub(super) fn conditions(
    rule: Rule,
    item: &Item,
    mv: &Move<'_>,
    who: &Who,
    ctx: &Context,
    out: &mut Vec<Unmet>,
) {
    let code_links = [LinkKind::Branch, LinkKind::Pr];
    let mut need_reason = |text: &str| {
        if !has_text(mv.reason) {
            out.push(unmet("reason.required", text, Some("Give a reason.")));
        }
    };
    match rule {
        Rule::Rework => need_reason("Say what needs rework."),
        Rule::Reject => need_reason("Name the failing acceptance criterion or machine."),
        Rule::Cancel => need_reason("Say why it is cancelled."),
        Rule::Unlisted => need_reason("This isn't a usual move; say why."),
        Rule::Refine => definition_of_ready(item, ctx, out),
        Rule::Start => {
            if item.assignee.is_none() {
                out.push(unmet(
                    "start.assignee",
                    "Nobody is assigned.",
                    Some("Assign it first."),
                ));
            }
            if item.item_type == ItemType::Epic {
                out.push(unmet(
                    "start.epic",
                    "Epics don't go to Doing; their children do.",
                    None,
                ));
            }
            if !ctx.has_link(&[LinkKind::Task]) {
                out.push(unmet(
                    "start.task",
                    "No delegated task is linked.",
                    Some("Delegate it and link the task."),
                ));
            }
        }
        Rule::Submit => {
            let code = matches!(item.item_type, ItemType::Feature | ItemType::Bug);
            if code && !ctx.has_link(&code_links) {
                out.push(unmet(
                    "review.branch",
                    "No branch is linked.",
                    Some("Link the branch."),
                ));
            }
            if !ctx.has_link(&[LinkKind::Artifact]) {
                out.push(unmet(
                    "review.change_note",
                    "No change note is linked.",
                    Some("Link the change-note artifact."),
                ));
            }
        }
        Rule::Approve => independent_review(item, who, ctx, out),
        Rule::Package => ready_to_package(item, ctx, out),
        Rule::Finish => {
            let chore_without_code =
                item.item_type == ItemType::Chore && !ctx.has_link(&code_links);
            if item.item_type != ItemType::Spike && !chore_without_code {
                out.push(unmet(
                    "done.flow",
                    "Only spikes and chores without code skip the release.",
                    Some("Send it on to Verify."),
                ));
            }
            if !ctx.has_link(&[LinkKind::Artifact]) {
                out.push(unmet(
                    "done.outcome",
                    "No outcome artifact is linked.",
                    Some("Link the outcome."),
                ));
            }
        }
        Rule::Release => {}
    }
}

/// Inbox → Ready: the template's DoR fields, size not L, blockers done.
fn definition_of_ready(item: &Item, ctx: &Context, out: &mut Vec<Unmet>) {
    let section = |heading: &str| section_filled(&item.description, heading);
    for field in &ctx.ready {
        let (missing, text) = match field.as_str() {
            "acceptance_criteria" => (
                item.acceptance_criteria.is_empty(),
                "Add at least one acceptance criterion.",
            ),
            "platforms" => (item.platforms.is_empty(), "Pick its platforms."),
            "size" => (item.size.is_none(), "Size it."),
            "spec_link_for_ui_or_daemon" => (
                item.platforms
                    .iter()
                    .any(|p| matches!(p, Platform::Desktop | Platform::Ios | Platform::Daemon))
                    && !ctx.has_link(&[LinkKind::Artifact])
                    && !section("Design/spec link"),
                "Link the design or spec.",
            ),
            "steps_to_reproduce" => (
                !section("Steps to reproduce"),
                "Fill in the steps to reproduce.",
            ),
            "expected_actual" => (
                !section("Expected") || !section("Actual"),
                "Fill in what was expected and what happened.",
            ),
            "question" => (!section("Question"), "State the question to answer."),
            "timebox" => (!section("Timebox"), "Set the timebox."),
            "goal" => (!section("Goal"), "State the goal."),
            "description" => (
                item.description
                    .lines()
                    .all(|l| l.trim().is_empty() || l.starts_with('#')),
                "Describe what and why.",
            ),
            // Templates are editable; a field this build doesn't know blocks nothing.
            _ => (false, ""),
        };
        if missing {
            out.push(unmet(&format!("dor.{field}"), text, None));
        }
    }
    if item.size == Some(Size::L) {
        out.push(unmet(
            "dor.size",
            "Size L is too big for Ready.",
            Some("Split it into S or M items."),
        ));
    }
    for blocker in ctx.blockers.iter().filter(|b| b.category != Cat::Done) {
        out.push(unmet(
            "dor.blocked_by",
            format!("{} blocks it and isn't done.", blocker.id),
            None,
        ));
    }
}

/// True when the description has a `## <heading…>` section with text under it.
pub(crate) fn section_filled(description: &str, heading: &str) -> bool {
    let heading = heading.to_lowercase();
    let mut inside = false;
    for line in description.lines() {
        if let Some(title) = line.strip_prefix('#') {
            inside = title
                .trim_start_matches('#')
                .trim()
                .to_lowercase()
                .starts_with(&heading);
        } else if inside && !line.trim().is_empty() {
            return true;
        }
    }
    false
}

/// Review → Verify: the reviewer is neither the assignee nor a branch author.
/// The branch's author is whoever linked it; commit authors aren't known here.
fn independent_review(item: &Item, who: &Who, ctx: &Context, out: &mut Vec<Unmet>) {
    let Who::Bot { id, .. } = who else { return };
    if who.is(item.assignee.as_deref()) {
        out.push(unmet(
            "review.independent",
            "The reviewer can't be the assignee.",
            Some("Pick another reviewer."),
        ));
    }
    let author = format!("bot:{id}");
    if ctx
        .links
        .iter()
        .any(|l| matches!(l.kind, LinkKind::Branch | LinkKind::Pr) && l.created_by == author)
    {
        out.push(unmet(
            "review.author",
            "The reviewer can't be an author of the branch.",
            Some("Pick another reviewer."),
        ));
    }
}

/// Verify → Owner testing: required machines passed, AC checked, packaged.
fn ready_to_package(item: &Item, ctx: &Context, out: &mut Vec<Unmet>) {
    let open = item
        .acceptance_criteria
        .iter()
        .filter(|ac| !ac.checked)
        .count();
    if open > 0 {
        out.push(unmet(
            "verify.ac",
            format!("{open} acceptance criteria are not checked."),
            None,
        ));
    }
    let machines: BTreeSet<&String> = item
        .platforms
        .iter()
        .filter_map(|p| ctx.required_machines.get(p))
        .flatten()
        .collect();
    for machine in machines {
        let latest = item
            .verifications
            .iter()
            .filter(|v| &v.machine == machine)
            .max_by_key(|v| v.at);
        if latest.map(|v| v.result) != Some(VerificationResult::Pass) {
            out.push(unmet(
                "verify.machine",
                format!("{machine} hasn't reported a pass."),
                Some("Ask its tester to verify it."),
            ));
        }
    }
    if item.release_id.is_none() {
        out.push(unmet(
            "verify.release",
            "It isn't in a submitted release package.",
            Some("Add it to a package and submit it."),
        ));
    }
}
