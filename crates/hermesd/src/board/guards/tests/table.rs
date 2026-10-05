//! The H-017 §3 table: one row per rule, with a case that passes, the change
//! that breaks it and the codes that change must produce. Every row also
//! checks that a bot with no role on the item is turned away.

use super::*;

struct Row {
    rule: &'static str,
    from: Cat,
    to: Cat,
    /// Who may, and what makes the move pass.
    ok: fn(&mut Case),
    /// What breaks it.
    bad: fn(&mut Case),
    expect: &'static [&'static str],
}

const ROWS: &[Row] = &[
    Row {
        rule: "Inbox → Ready: DoR filled, size ≠ L, blockers done",
        from: Cat::Inbox,
        to: Cat::Ready,
        ok: |c| {
            c.who = bot("lead", LEAD);
            c.ctx.ready = [
                "acceptance_criteria",
                "platforms",
                "size",
                "spec_link_for_ui_or_daemon",
            ]
            .map(String::from)
            .to_vec();
            c.ctx.links.push(link(LinkKind::Artifact, "arch"));
            c.ctx.blockers.push(Blocker {
                id: "H-0".into(),
                category: Cat::Done,
            });
        },
        bad: |c| {
            c.item.acceptance_criteria.clear();
            c.item.size = Some(Size::L);
            c.ctx.links.clear();
            c.ctx.blockers.push(Blocker {
                id: "H-2".into(),
                category: Cat::Doing,
            });
        },
        expect: &[
            "dor.acceptance_criteria",
            "dor.spec_link_for_ui_or_daemon",
            "dor.size",
            "dor.blocked_by",
        ],
    },
    Row {
        rule: "Ready → Inbox: lead or owner un-refines, with a reason",
        from: Cat::Ready,
        to: Cat::Inbox,
        ok: |c| {
            c.who = bot("lead", LEAD);
            c.reason = Some("needs a spec");
        },
        bad: |c| c.reason = None,
        expect: &["reason.required"],
    },
    Row {
        rule: "Ready → Doing (lead): assignee set, not an epic, task linked",
        from: Cat::Ready,
        to: Cat::Doing,
        ok: |c| {
            c.who = bot("lead", LEAD);
            c.ctx.links.push(link(LinkKind::Task, "lead"));
        },
        bad: |c| {
            c.item.assignee = None;
            c.item.item_type = ItemType::Epic;
            c.ctx.links.clear();
        },
        expect: &["start.assignee", "start.epic", "start.task"],
    },
    Row {
        rule: "Ready → Doing (assignee pulls): per-assignee WIP",
        from: Cat::Ready,
        to: Cat::Doing,
        ok: |c| {
            c.who = bot("dev", &[Role::Dev]);
            c.ctx.links.push(link(LinkKind::Task, "lead"));
            c.wip_limit = Some(1);
        },
        bad: |c| {
            c.ctx.load.insert(
                "doing".into(),
                ColumnLoad {
                    items: 3,
                    assignee_items: 1,
                },
            );
        },
        expect: &["wip.full"],
    },
    Row {
        rule: "Doing → Review: branch link and change note",
        from: Cat::Doing,
        to: Cat::Review,
        ok: |c| {
            c.who = bot("dev", &[Role::Dev]);
            c.ctx.links.push(link(LinkKind::Branch, "dev"));
            c.ctx.links.push(link(LinkKind::Artifact, "dev"));
        },
        bad: |c| c.ctx.links.clear(),
        expect: &["review.branch", "review.change_note"],
    },
    Row {
        rule: "Review → Verify: reviewer ≠ assignee",
        from: Cat::Review,
        to: Cat::Verify,
        ok: |c| {
            c.who = bot("arch", &[Role::ReviewerArch]);
            c.ctx.links.push(link(LinkKind::Branch, "dev"));
        },
        bad: |c| c.item.assignee = Some("arch".into()),
        expect: &["review.independent"],
    },
    Row {
        rule: "Review → Verify: reviewer ≠ branch author",
        from: Cat::Review,
        to: Cat::Verify,
        ok: |c| {
            c.who = bot("arch", &[Role::ReviewerArch]);
            c.ctx.links.push(link(LinkKind::Branch, "dev"));
        },
        bad: |c| c.ctx.links.push(link(LinkKind::Pr, "arch")),
        expect: &["review.author"],
    },
    Row {
        rule: "Review → Doing: reviewer or lead, with a reason",
        from: Cat::Review,
        to: Cat::Doing,
        ok: |c| {
            c.who = bot("arch", &[Role::ReviewerArch]);
            c.reason = Some("tests missing");
        },
        bad: |c| c.reason = Some("  "),
        expect: &["reason.required"],
    },
    Row {
        rule: "Review → Doing over WIP: a return is never refused for WIP (rev 2.1)",
        from: Cat::Review,
        to: Cat::Doing,
        ok: |c| {
            c.who = bot("arch", &[Role::ReviewerArch]);
            c.reason = Some("tests missing");
            c.wip_limit = Some(1);
            c.ctx.load.insert(
                "doing".into(),
                ColumnLoad {
                    items: 4,
                    assignee_items: 2,
                },
            );
        },
        bad: |c| c.reason = None,
        expect: &["reason.required"],
    },
    Row {
        rule: "Verify → Owner testing: machines pass, AC checked, packaged",
        from: Cat::Verify,
        to: Cat::Approval,
        ok: |c| {
            c.who = bot("ops", &[Role::Devops]);
            c.ctx
                .required_machines
                .insert(Platform::Daemon, vec!["mac".into(), "win".into()]);
            c.item.release_id = Some("R-1".into());
            for (machine, result) in [
                ("mac", VerificationResult::Fail),
                ("mac", VerificationResult::Pass),
                ("win", VerificationResult::Pass),
            ] {
                c.item.verifications.push(ItemVerification {
                    machine: machine.into(),
                    result,
                    by: "bot:tester".into(),
                    at: Utc::now(),
                    note: None,
                });
            }
        },
        bad: |c| {
            c.item.acceptance_criteria[0].checked = false;
            c.item.verifications.pop();
            c.item.release_id = None;
        },
        expect: &["verify.ac", "verify.machine", "verify.release"],
    },
    Row {
        rule: "Verify → Doing: tester or lead, naming the failure",
        from: Cat::Verify,
        to: Cat::Doing,
        ok: |c| {
            c.who = bot("tester", &[Role::Tester]);
            c.reason = Some("AC 1 fails on win");
        },
        bad: |c| c.reason = None,
        expect: &["reason.required"],
    },
    Row {
        rule: "Owner testing → Deploying: daemon only",
        from: Cat::Approval,
        to: Cat::Deploying,
        ok: |c| c.who = Who::Daemon,
        bad: |c| c.who = Who::Owner,
        expect: &["move.daemon_only"],
    },
    Row {
        rule: "Owner testing → Doing / Ready: daemon only",
        from: Cat::Approval,
        to: Cat::Ready,
        ok: |c| c.who = Who::Daemon,
        bad: |c| c.who = bot("lead", LEAD),
        expect: &["move.daemon_only"],
    },
    Row {
        rule: "Deploying → Done: daemon only",
        from: Cat::Deploying,
        to: Cat::Done,
        ok: |c| c.who = Who::Daemon,
        bad: |c| c.who = bot("ops", &[Role::Devops]),
        expect: &["move.daemon_only"],
    },
    Row {
        rule: "Owner testing / Deploying → Cancelled: daemon only, owner included",
        from: Cat::Deploying,
        to: Cat::Cancelled,
        ok: |c| c.who = Who::Daemon,
        bad: |c| {
            c.who = Who::Owner;
            c.reason = Some("changed my mind");
        },
        expect: &["move.daemon_only"],
    },
    Row {
        rule: "Doing/Review/Verify → Done: spike or non-code chore, outcome linked",
        from: Cat::Review,
        to: Cat::Done,
        ok: |c| {
            c.who = bot("dev", &[Role::Dev]);
            c.item.item_type = ItemType::Chore;
            c.ctx.links.push(link(LinkKind::Artifact, "dev"));
        },
        bad: |c| c.ctx.links = vec![link(LinkKind::Branch, "dev")],
        expect: &["done.flow", "done.outcome"],
    },
    Row {
        rule: "Verify → Done, no release: the owner, with a reason (ARCH-R22 F1)",
        from: Cat::Verify,
        to: Cat::Done,
        ok: |c| {
            c.who = Who::Owner;
            c.reason = Some("shipped in 0.14.1 by hand");
        },
        bad: |c| c.reason = None,
        expect: &["reason.required"],
    },
    Row {
        rule: "Any → Cancelled: lead or owner, with a reason",
        from: Cat::Verify,
        to: Cat::Cancelled,
        ok: |c| {
            c.who = bot("lead", LEAD);
            c.reason = Some("superseded by H-9");
        },
        bad: |c| c.reason = None,
        expect: &["reason.required"],
    },
    Row {
        rule: "Unlisted (Done → Doing): owner only, with a reason",
        from: Cat::Done,
        to: Cat::Doing,
        ok: |c| {
            c.who = Who::Owner;
            c.reason = Some("reopened");
        },
        bad: |c| c.who = bot("lead", LEAD),
        expect: &["role.not_allowed"],
    },
];

#[test]
fn every_transition_rule_passes_when_met_and_names_what_is_not() {
    for row in ROWS {
        let mut c = case(row.from);
        (row.ok)(&mut c);
        assert_eq!(
            run(&c, row.to),
            Vec::<String>::new(),
            "{}: should pass",
            row.rule
        );

        let mut stranger = case(row.from);
        (row.ok)(&mut stranger);
        if stranger.who != Who::Daemon {
            stranger.who = bot("writer", &[]);
            let codes = run(&stranger, row.to);
            assert!(
                codes
                    .iter()
                    .any(|c| c == "role.not_allowed" || c == "move.daemon_only"),
                "{}: a bot with no role got {codes:?}",
                row.rule
            );
        }

        (row.bad)(&mut c);
        assert_eq!(run(&c, row.to), row.expect, "{}: should refuse", row.rule);
    }
}
