//! The rules for acceptance criteria outside moves: checking, flagging
//! post-install (H-116) and editing.

use super::{has_text, unmet, Who};
use crate::board::model::{ColumnCategory as Cat, Item, Role, Unmet};

/// Checking an acceptance criterion: a tester, a verifier named on the item,
/// the lead or the owner. A lead bot that doesn't verify gives the evidence
/// it checked against (H-116); it is posted as a comment.
pub fn check_ac(item: &Item, who: &Who, evidence: Option<&str>) -> Vec<Unmet> {
    if !(who.leads() || *who == Who::Daemon || who.verifies(item)) {
        vec![unmet(
            "role.not_allowed",
            "Only a tester, a named verifier or the lead can.",
            None,
        )]
    } else if needs_evidence(item, who) && !has_text(evidence) {
        vec![unmet(
            "ac.evidence",
            "The lead ticks a criterion with the evidence it was checked against.",
            Some("Give 'evidence': what shows it holds (a test, a log, a link)."),
        )]
    } else {
        Vec::new()
    }
}

/// A lead bot checking a criterion it doesn't verify as a tester or named
/// verifier.
pub fn needs_evidence(item: &Item, who: &Who) -> bool {
    who.has(Role::Lead) && !who.verifies(item)
}

/// Flagging an acceptance criterion post-install (H-116): the lead (or the
/// owner) until the item is done; its assignee until it reaches Verify. Past
/// Verify the item is in a submitted package, which skipped its flagged
/// criteria, so the flag can't be cleared there (ARCH-R53 M1).
pub fn flag_ac(item: &Item, who: &Who, post_install: bool) -> Vec<Unmet> {
    if matches!(item.category, Cat::Done | Cat::Cancelled) {
        return vec![unmet("ac.locked", "It is closed.", None)];
    }
    if !post_install && matches!(item.category, Cat::Approval | Cat::Deploying) {
        return vec![unmet(
            "ac.post_install_locked",
            "It is in a submitted package, which skipped this criterion as post-install.",
            Some("Tick it with item_check_ac once it is installed."),
        )];
    }
    let early = !matches!(item.category, Cat::Verify | Cat::Approval | Cat::Deploying);
    if who.leads() || (early && who.is(item.assignee.as_deref())) {
        Vec::new()
    } else {
        vec![unmet(
            "role.not_allowed",
            "Only the lead, or the assignee before Verify, can.",
            None,
        )]
    }
}

/// Changing acceptance criteria: until the item reaches Verify; after that
/// they are what testers and the owner check against.
pub fn check_ac_edit(item: &Item) -> Vec<Unmet> {
    if matches!(
        item.category,
        Cat::Verify | Cat::Approval | Cat::Deploying | Cat::Done | Cat::Cancelled
    ) {
        vec![unmet(
            "ac.locked",
            "Its acceptance criteria are fixed once it reaches Verify.",
            Some("Send it back to Doing first."),
        )]
    } else {
        Vec::new()
    }
}
