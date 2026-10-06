//! Closing on an outcome, past code and whose word closes it (H-154,
//! ARCH-R62 M1, M2).

use super::*;

/// A spike in Verify with its outcome linked, as the lead sees it.
fn spike_with_outcome() -> Case {
    let mut c = case(Cat::Verify);
    c.item.item_type = ItemType::Spike;
    c.who = bot("lead", LEAD);
    c.ctx.links.push(link(LinkKind::Artifact, "dev"));
    c
}

#[test]
fn code_once_linked_keeps_the_release_path_whatever_the_type_is_now() {
    let mut c = spike_with_outcome();
    assert!(run(&c, Cat::Done).is_empty());
    // A branch was linked once, then the item retyped or the branch removed:
    // the history still says it had code.
    c.ctx.ever_had_code = true;
    assert_eq!(run(&c, Cat::Done), ["done.flow"]);
    c.item.item_type = ItemType::Chore;
    assert_eq!(run(&c, Cat::Done), ["done.flow"]);
    // Not started, it gets the release path's refusal, not "move it on".
    let mut early = case(Cat::Ready);
    early.item.item_type = ItemType::Spike;
    early.ctx.ever_had_code = true;
    assert_eq!(run(&early, Cat::Done), ["move.daemon_only"]);
}

#[test]
fn nobody_closes_what_they_did_themselves_but_the_owner_still_does() {
    let mut c = spike_with_outcome();
    // The lead assigned to its own spike is the one who did it.
    c.item.assignee = Some("lead".into());
    assert_eq!(run(&c, Cat::Done), ["role.not_allowed"]);
    // So is a reviewer who is the assignee.
    c.item.assignee = Some("arch".into());
    c.who = bot("arch", &[Role::ReviewerArch]);
    assert_eq!(run(&c, Cat::Done), ["role.not_allowed"]);
    // Anyone else of the two closes it, and the owner always may.
    c.who = bot("lead", LEAD);
    assert!(run(&c, Cat::Done).is_empty());
    c.who = Who::Owner;
    assert!(run(&c, Cat::Done).is_empty());
}
