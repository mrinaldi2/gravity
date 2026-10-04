//! Guard tests: the H-017 §3 table (`table`), then the finer points of WIP,
//! reviewers, DoR sections and the rules outside moves.

use chrono::Utc;

use super::*;
use crate::board::defaults::COLUMNS;
use crate::board::model::{
    AcceptanceCriterion, ItemPerson, ItemType, ItemVerification, Priority, Size, VerificationResult,
};

struct Case {
    item: Item,
    ctx: Context,
    who: Who,
    reason: Option<&'static str>,
    override_reason: Option<&'static str>,
    wip_limit: Option<u32>,
}

fn bot(id: &str, roles: &[Role]) -> Who {
    Who::Bot {
        id: id.to_string(),
        roles: roles.to_vec(),
    }
}

fn link(kind: LinkKind, by: &str) -> ItemLink {
    ItemLink {
        item_id: "H-1".into(),
        kind,
        target: format!("{}-ref", kind.as_str()),
        label: None,
        created_by: format!("bot:{by}"),
        at: Utc::now(),
    }
}

fn column(category: Cat) -> BoardColumn {
    let d = COLUMNS
        .iter()
        .find(|c| c.category == category)
        .expect("default column");
    BoardColumn {
        project_id: "p".into(),
        key: d.key.into(),
        name: d.name.into(),
        ord: 0,
        category,
        wip_limit: None,
        wip_scope: d.wip_scope,
        visible: d.visible,
    }
}

fn item(category: Cat) -> Item {
    let now = Utc::now();
    Item {
        id: "H-1".into(),
        seq: 1,
        item_type: ItemType::Feature,
        title: "A thing".into(),
        description: "## User/problem\n\nIt hurts.\n".into(),
        platforms: vec![Platform::Daemon],
        size: Some(Size::M),
        priority: Priority::P2,
        rank: "a".into(),
        column_key: column(category).key,
        category,
        blocked: None,
        assignee: Some("dev".into()),
        parent_id: None,
        release_id: None,
        labels: Vec::new(),
        acceptance_criteria: vec![AcceptanceCriterion {
            idx: 0,
            text: "works".into(),
            checked: true,
            checked_by: None,
            checked_at: None,
            machine: None,
        }],
        people: Vec::new(),
        verifications: Vec::new(),
        created_by: "bot:lead".into(),
        created_at: now,
        updated_at: now,
        state_entered_at: now,
        done_at: None,
        version: 1,
    }
}

fn run(case: &Case, to: Cat) -> Vec<String> {
    let mut to = column(to);
    to.wip_limit = case.wip_limit;
    let mv = Move {
        to: &to,
        reason: case.reason,
        override_reason: case.override_reason,
    };
    evaluate(&case.item, &mv, &case.who, &case.ctx)
        .into_iter()
        .map(|u| u.code)
        .collect()
}

const LEAD: &[Role] = &[Role::Lead];

fn case(from: Cat) -> Case {
    Case {
        item: item(from),
        ctx: Context::default(),
        who: Who::Owner,
        reason: None,
        override_reason: None,
        wip_limit: None,
    }
}

mod table;

#[test]
fn rules_cover_every_category_pair() {
    for from in Cat::ALL {
        for to in Cat::ALL {
            let _ = rule(*from, *to);
        }
    }
    assert_eq!(rule(Cat::Inbox, Cat::Done), Rule::Release);
    assert_eq!(rule(Cat::Approval, Cat::Cancelled), Rule::Release);
    assert_eq!(rule(Cat::Deploying, Cat::Cancelled), Rule::Release);
    assert_eq!(rule(Cat::Verify, Cat::Cancelled), Rule::Cancel);
    assert_eq!(rule(Cat::Ready, Cat::Inbox), Rule::Unrefine);
    assert_eq!(rule(Cat::Verify, Cat::Review), Rule::Unlisted);
}

#[test]
fn wip_is_overridden_only_by_the_lead_or_owner_with_a_reason() {
    let mut c = case(Cat::Ready);
    c.ctx.links.push(link(LinkKind::Task, "lead"));
    c.ctx.load.insert(
        "review".into(),
        ColumnLoad {
            items: 3,
            assignee_items: 0,
        },
    );
    c.item.column_key = "ready".into();
    c.wip_limit = Some(3);
    // Ready → Review is unlisted, so use the owner and give a reason.
    c.reason = Some("hotfix");
    assert_eq!(run(&c, Cat::Review), ["wip.full"]);
    c.override_reason = Some("hotfix for the owner");
    assert!(run(&c, Cat::Review).is_empty());

    let mut c = case(Cat::Ready);
    c.ctx.links.push(link(LinkKind::Task, "lead"));
    c.ctx.load.insert(
        "doing".into(),
        ColumnLoad {
            items: 1,
            assignee_items: 1,
        },
    );
    c.wip_limit = Some(1);
    c.override_reason = Some("urgent");
    c.who = bot("dev", &[Role::Dev]);
    assert_eq!(
        run(&c, Cat::Doing),
        ["wip.full"],
        "an assignee can't override"
    );
    c.who = bot("lead", LEAD);
    assert!(run(&c, Cat::Doing).is_empty());
    c.item.assignee = Some("other".into());
    c.ctx.load.insert(
        "doing".into(),
        ColumnLoad {
            items: 1,
            assignee_items: 0,
        },
    );
    c.override_reason = None;
    assert!(
        run(&c, Cat::Doing).is_empty(),
        "per-assignee: another bot's item doesn't count"
    );
}

#[test]
fn a_named_reviewer_is_the_only_reviewer() {
    let mut c = case(Cat::Review);
    c.item.people.push(ItemPerson {
        bot_id: "ux".into(),
        role: PersonRole::Reviewer,
    });
    c.who = bot("arch", &[Role::ReviewerArch]);
    assert_eq!(run(&c, Cat::Verify), ["role.not_allowed"]);
    c.who = bot("ux", &[]);
    assert!(run(&c, Cat::Verify).is_empty());
}

#[test]
fn dor_sections_come_from_the_description() {
    let mut c = case(Cat::Inbox);
    c.item.item_type = ItemType::Bug;
    c.ctx.ready = ["steps_to_reproduce", "expected_actual"]
        .map(String::from)
        .to_vec();
    c.item.description =
        "## Steps to reproduce\n\n1. open\n\n## Expected\n\nworks\n\n## Actual\n\n".into();
    assert_eq!(run(&c, Cat::Ready), ["dor.expected_actual"]);
    c.item.description.push_str("crashes\n");
    assert!(run(&c, Cat::Ready).is_empty());
    c.item.description = "## Steps to reproduce\n\n<!-- Numbered steps -->\n\n## Expected\n\nworks\n\n## Actual\n\ncrashes\n".into();
    assert_eq!(
        run(&c, Cat::Ready),
        ["dor.steps_to_reproduce"],
        "a hint doesn't fill a section"
    );
    c.item.description = crate::board::defaults::description_skeleton(&serde_json::json!({
        "sections": ["Steps to reproduce", "Expected", "Actual"]
    }));
    assert_eq!(
        run(&c, Cat::Ready),
        ["dor.steps_to_reproduce", "dor.expected_actual"],
        "a fresh template fills nothing"
    );
    c.item.description =
        "## Steps to reproduce\n\n1. open\n\n## Expected\n\nworks\n\n## Actual\n\ncrashes\n".into();
    c.ctx.ready.push("a_field_from_the_future".into());
    assert!(
        run(&c, Cat::Ready).is_empty(),
        "unknown DoR fields block nothing"
    );
}

#[test]
fn same_column_is_refused_even_for_the_daemon() {
    let mut c = case(Cat::Doing);
    c.who = Who::Daemon;
    assert_eq!(run(&c, Cat::Doing), ["move.same_column"]);
}

#[test]
fn rules_outside_moves() {
    // — → Inbox: type and title.
    assert!(check_new("Fix it").is_empty());
    assert_eq!(check_new("  ")[0].code, "new.title");
    // Set/clear blocked: assignee, lead, owner; a reason when setting.
    let it = item(Cat::Doing);
    assert!(check_block(&it, &bot("dev", &[]), true, Some("waiting on H-2")).is_empty());
    assert_eq!(
        check_block(&it, &bot("dev", &[]), true, None)[0].code,
        "reason.required"
    );
    assert!(check_block(&it, &Who::Owner, false, None).is_empty());
    assert_eq!(
        check_block(&it, &bot("writer", &[]), false, None)[0].code,
        "role.not_allowed"
    );
    // Rank, P0, WIP limits, columns, templates: owner or lead.
    assert!(check_lead(&Who::Owner).is_empty());
    assert!(check_lead(&bot("lead", LEAD)).is_empty());
    assert_eq!(
        check_lead(&bot("coach", &[Role::Coach]))[0].code,
        "role.not_allowed"
    );
}

#[test]
fn templates_name_only_known_dor_fields() {
    for (item_type, body) in crate::board::defaults::item_templates() {
        assert_eq!(check_template(&body), Ok(()), "{item_type:?}");
    }
    let typo = serde_json::json!({"ready": ["acceptance_critera", "size"]});
    let err = check_template(&typo).unwrap_err();
    assert!(
        err.contains("acceptance_critera") && err.contains("acceptance_criteria"),
        "{err}"
    );
}

#[test]
fn acceptance_criteria_are_fixed_from_verify_on() {
    for cat in [Cat::Inbox, Cat::Ready, Cat::Doing, Cat::Review] {
        assert!(check_ac_edit(&item(cat)).is_empty(), "{cat:?}");
    }
    for cat in [
        Cat::Verify,
        Cat::Approval,
        Cat::Deploying,
        Cat::Done,
        Cat::Cancelled,
    ] {
        assert_eq!(check_ac_edit(&item(cat))[0].code, "ac.locked", "{cat:?}");
    }
}
