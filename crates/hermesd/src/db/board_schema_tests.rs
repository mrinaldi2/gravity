//! The migration's CHECK lists against the contract enums.

use crate::board::model::*;

use super::board::to_text;
use super::board_items::NewItem;
use super::board_tests::{board, new_item, OWNER};

/// The migration's CHECK lists and the contract enums are written twice; this
/// keeps them in step by storing every variant.
#[test]
fn every_contract_value_passes_the_schema_checks() {
    let (db, p) = board();
    for item_type in [
        ItemType::Epic,
        ItemType::Feature,
        ItemType::Bug,
        ItemType::Spike,
        ItemType::Chore,
    ] {
        for (size, priority) in [
            (Size::S, Priority::P0),
            (Size::M, Priority::P1),
            (Size::L, Priority::P2),
            (Size::L, Priority::P3),
        ] {
            db.create_item(
                &NewItem {
                    item_type,
                    size: Some(size),
                    priority,
                    ..new_item(&p, "x")
                },
                &OWNER,
            )
            .unwrap();
        }
    }
    let item = db.create_item(&new_item(&p, "links"), &OWNER).unwrap();
    for kind in [
        LinkKind::Task,
        LinkKind::Decision,
        LinkKind::Artifact,
        LinkKind::Branch,
        LinkKind::Pr,
        LinkKind::Meeting,
        LinkKind::ItemBlocks,
        LinkKind::ItemRelates,
        LinkKind::ItemDuplicates,
    ] {
        db.add_item_link(&item.id, kind, "ref", None, &OWNER)
            .unwrap();
    }
    let conn = db.lock();
    let exec = |sql: &str, args: &[&dyn rusqlite::ToSql]| {
        conn.execute(sql, args)
            .unwrap_or_else(|e| panic!("{sql}: {e}"));
    };
    for category in [
        ColumnCategory::Inbox,
        ColumnCategory::Ready,
        ColumnCategory::Doing,
        ColumnCategory::Review,
        ColumnCategory::Verify,
        ColumnCategory::Approval,
        ColumnCategory::Deploying,
        ColumnCategory::Done,
        ColumnCategory::Cancelled,
    ] {
        exec(
            "UPDATE board_column SET category = ?2 WHERE project_id = ?1 AND key = 'deploying'",
            &[&p, &to_text(&category)],
        );
    }
    for scope in [WipScope::Column, WipScope::PerAssignee] {
        exec(
            "UPDATE board_column SET wip_scope = ?2 WHERE project_id = ?1 AND key = 'deploying'",
            &[&p, &to_text(&scope)],
        );
    }
    let bot: String = conn
        .query_row("SELECT id FROM bot LIMIT 1", [], |r| r.get(0))
        .unwrap();
    for role in [
        Role::Lead,
        Role::Coach,
        Role::Devops,
        Role::ReviewerArch,
        Role::ReviewerUx,
        Role::Tester,
        Role::Dev,
    ] {
        exec(
            "INSERT OR REPLACE INTO project_role(project_id, role, bot_id) VALUES (?1, ?2, ?3)",
            &[&p, &to_text(&role), &bot],
        );
    }
    for kind in [
        ItemEventKind::Created,
        ItemEventKind::Moved,
        ItemEventKind::Edited,
        ItemEventKind::Commented,
        ItemEventKind::Linked,
        ItemEventKind::Assigned,
        ItemEventKind::Blocked,
        ItemEventKind::Ranked,
    ] {
        exec("INSERT INTO item_event(item_id, project_id, at, actor, kind) VALUES (?1, ?2, 'now', 'user', ?3)",
             &[&item.id, &p, &to_text(&kind)]);
    }
    for role in [PersonRole::Reviewer, PersonRole::Verifier] {
        exec(
            "INSERT INTO item_person(item_id, bot_id, role) VALUES (?1, 'b', ?2)",
            &[&item.id, &to_text(&role)],
        );
    }
    for (machine, result) in [
        ("a", VerificationResult::Pass),
        ("b", VerificationResult::Fail),
        ("c", VerificationResult::Blocked),
    ] {
        exec("INSERT INTO item_verification(item_id, machine, result, by, at) VALUES (?1, ?2, ?3, 'b', 'now')",
             &[&item.id, &machine, &to_text(&result)]);
    }
    for kind in [TemplateKind::ItemType, TemplateKind::MeetingType] {
        exec("INSERT INTO template(project_id, kind, name, version, body, created_at) VALUES (?1, ?2, 'check', 9, '{}', 'now')",
             &[&p, &to_text(&kind)]);
    }
}
