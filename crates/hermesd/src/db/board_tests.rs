//! Board storage: seeding, the id sequence, ranks, versioned writes, history
//! and search.

use crate::board::model::*;

use super::board_edit::ItemEdit;
use super::board_items::{NewItem, Write};
use super::MoveTo;
use super::{Actor, Db};

pub(super) const OWNER: Actor<'static> = Actor::User;

pub(super) fn board() -> (Db, String) {
    let db = Db::open_in_memory().unwrap();
    let p = db.create_project("The Hermes", "the-hermes").unwrap();
    let lead = db
        .create_bot(&p.id, "Team Lead", "", "", "", "/tmp/a", "team-lead", None)
        .unwrap();
    db.create_bot(&p.id, "Architect", "", "", "", "/tmp/b", "architect", None)
        .unwrap();
    db.create_bot(&p.id, "Writer", "", "", "", "/tmp/c", "writer", None)
        .unwrap();
    db.set_project_lead(&p.id, Some(&lead.id)).unwrap();
    db.ensure_board(&p.id, "d-mac", Some("H")).unwrap();
    (db, p.id)
}

pub(super) fn new_item<'a>(project_id: &'a str, title: &'a str) -> NewItem<'a> {
    NewItem {
        project_id,
        item_type: ItemType::Feature,
        title,
        description: "",
        platforms: &[Platform::Daemon],
        size: Some(Size::M),
        priority: Priority::P2,
        labels: &[],
        parent_id: None,
        acceptance_criteria: &[],
    }
}

fn done<T: std::fmt::Debug>(write: Write<T>) -> T {
    match write {
        Write::Done(value) => value,
        Write::Conflict(current) => panic!("unexpected conflict: {current:?}"),
    }
}

#[test]
fn a_new_board_gets_columns_templates_and_guessed_roles_once() {
    let (db, p) = board();
    let columns = db.board_columns(&p).unwrap();
    let keys: Vec<&str> = columns.iter().map(|c| c.key.as_str()).collect();
    assert_eq!(
        keys,
        [
            "inbox",
            "ready",
            "doing",
            "review",
            "verify",
            "approval",
            "deploying",
            "done",
            "cancelled"
        ]
    );
    assert_eq!(columns[5].name, "Awaiting owner");
    assert_eq!(columns[2].wip_scope, WipScope::PerAssignee);
    assert_eq!(db.templates(&p, TemplateKind::ItemType).unwrap().len(), 5);
    let roles: Vec<Role> = db
        .project_roles(&p)
        .unwrap()
        .into_iter()
        .map(|r| r.role)
        .collect();
    assert!(roles.contains(&Role::Lead) && roles.contains(&Role::ReviewerArch));
    assert_eq!(roles.len(), 2, "the writer gets no role");

    let again = db.ensure_board(&p, "elsewhere", Some("X")).unwrap();
    assert_eq!(
        (again.key.as_str(), again.home_daemon_id.as_str()),
        ("H", "d-mac")
    );
    assert_eq!(db.board_columns(&p).unwrap().len(), 9);
}

/// A tester verifies on its own machine: a linked tester's is its peer's.
#[test]
fn a_linked_tester_is_seeded_with_its_machine() {
    let db = Db::open_in_memory().unwrap();
    let p = db.create_project("Gravity", "gravity").unwrap();
    let pc = db.create_peer("win-pc", None).unwrap();
    let remote = bus::RemoteBot {
        id: "r1".into(),
        name: "Tester Win".into(),
        description: String::new(),
        avatar: String::new(),
        runtime: bus::BotRuntime::default(),
        project: "Gravity".into(),
        temporary: false,
    };
    db.create_linked_bot(&p.id, &remote, &pc.id).unwrap();
    db.ensure_board(&p.id, "d-mac", None).unwrap();
    let roles = db.project_roles(&p.id).unwrap();
    assert_eq!(roles.len(), 1);
    assert_eq!(
        (roles[0].role, roles[0].machine.as_deref()),
        (Role::Tester, Some("win-pc"))
    );
}

#[test]
fn board_keys_stay_unique_across_projects() {
    let (db, _) = board();
    let other = db.create_project("Hobby", "hobby").unwrap();
    assert_eq!(db.ensure_board(&other.id, "d-mac", None).unwrap().key, "H2");
}

#[test]
fn items_take_sequential_ids_and_rank_last_in_the_inbox() {
    let (db, p) = board();
    let criteria = ["Cards move".to_string()];
    let first = db
        .create_item(
            &NewItem {
                acceptance_criteria: &criteria,
                ..new_item(&p, "Board")
            },
            &OWNER,
        )
        .unwrap();
    let second = db.create_item(&new_item(&p, "Dashboard"), &OWNER).unwrap();
    assert_eq!((first.id.as_str(), second.id.as_str()), ("H-001", "H-002"));
    assert!(first.rank < second.rank);
    assert_eq!(first.column_key, "inbox");
    assert!(first.description.starts_with("## User/problem"));
    assert_eq!(first.acceptance_criteria[0].text, "Cards move");
    assert_eq!(db.board_settings(&p).unwrap().unwrap().next_seq, 3);
    let history = db.item_events(&first.id).unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(
        (history[0].kind, history[0].actor.as_str()),
        (ItemEventKind::Created, "user")
    );
}

#[test]
fn a_stale_version_is_refused_with_the_current_item() {
    let (db, p) = board();
    let item = db.create_item(&new_item(&p, "Board"), &OWNER).unwrap();
    let edit = ItemEdit {
        title: Some("Board, read-only"),
        priority: Some(Priority::P1),
        ..ItemEdit::default()
    };
    let edited = done(
        db.update_item(&item.id, item.version, &edit, &OWNER)
            .unwrap(),
    );
    assert_eq!(
        (edited.title.as_str(), edited.version),
        ("Board, read-only", item.version + 1)
    );

    let stale = ItemEdit {
        title: Some("Lost update"),
        ..ItemEdit::default()
    };
    match db
        .update_item(&item.id, item.version, &stale, &OWNER)
        .unwrap()
    {
        Write::Conflict(current) => assert_eq!(current.title, "Board, read-only"),
        Write::Done(_) => panic!("a stale version must not write"),
    }
    let fields: Vec<Option<String>> = db
        .item_events(&item.id)
        .unwrap()
        .into_iter()
        .map(|e| e.field)
        .collect();
    assert_eq!(
        fields,
        [None, Some("title".into()), Some("priority".into())]
    );
}

fn to(column: &str) -> MoveTo<'_> {
    MoveTo {
        column,
        ..MoveTo::default()
    }
}

#[test]
fn moves_record_where_from_and_stamp_done() {
    let (db, p) = board();
    let item = db.create_item(&new_item(&p, "Board"), &OWNER).unwrap();
    let bot = Actor::Bot {
        id: "b1",
        project_id: &p,
    };
    let moved = done(
        db.move_item(
            &item.id,
            item.version,
            &MoveTo {
                column: "doing",
                note: Some("picked up"),
                ..MoveTo::default()
            },
            &bot,
        )
        .unwrap(),
    );
    assert_eq!(
        (moved.category, moved.done_at),
        (ColumnCategory::Doing, None)
    );
    let finished = done(
        db.move_item(&item.id, moved.version, &to("done"), &bot)
            .unwrap(),
    );
    assert!(finished.done_at.is_some());
    let last = db.item_events(&item.id).unwrap().pop().unwrap();
    assert_eq!(
        (last.kind, last.from.as_deref(), last.to.as_deref()),
        (ItemEventKind::Moved, Some("doing"), Some("done"))
    );
    assert_eq!(last.actor, "bot:b1");
    assert!(db
        .move_item(&item.id, finished.version, &to("nowhere"), &bot)
        .is_err());
    assert_eq!(
        db.get_item(&item.id).unwrap().unwrap().version,
        finished.version,
        "a failed move changes nothing"
    );
}

#[test]
fn ranking_places_an_item_between_neighbours() {
    let (db, p) = board();
    let a = db.create_item(&new_item(&p, "A"), &OWNER).unwrap();
    let b = db.create_item(&new_item(&p, "B"), &OWNER).unwrap();
    let c = db.create_item(&new_item(&p, "C"), &OWNER).unwrap();
    done(
        db.rank_item(&c.id, c.version, Some(&a.id), Some(&b.id), &OWNER)
            .unwrap(),
    );
    let order: Vec<String> = db
        .board_cards(&p)
        .unwrap()
        .into_iter()
        .map(|card| card.title)
        .collect();
    assert_eq!(order, ["A", "C", "B"]);
    assert!(
        db.rank_item(&a.id, a.version, Some(&b.id), Some(&c.id), &OWNER)
            .is_err(),
        "neighbours out of order"
    );
}

#[test]
fn search_finds_titles_descriptions_and_comments() {
    let (db, p) = board();
    let a = db
        .create_item(&new_item(&p, "Release gate"), &OWNER)
        .unwrap();
    let b = db.create_item(&new_item(&p, "Meetings"), &OWNER).unwrap();
    db.add_item_comment(&b.id, "Needs the standup template", None, &OWNER)
        .unwrap();
    let ids = |q: &str| -> Vec<String> {
        db.search_items(&p, q)
            .unwrap()
            .into_iter()
            .map(|c| c.id)
            .collect()
    };
    assert_eq!(ids("gate"), vec![a.id.clone()]);
    assert_eq!(ids("standup"), vec![b.id.clone()]);
    done(
        db.update_item(
            &a.id,
            a.version,
            &ItemEdit {
                title: Some("Deploy approval"),
                ..ItemEdit::default()
            },
            &OWNER,
        )
        .unwrap(),
    );
    assert!(ids("gate").is_empty());
    assert_eq!(ids("approval"), [a.id]);
}

#[test]
fn cards_flag_blocked_and_count_criteria() {
    let (db, p) = board();
    let criteria = ["one".to_string(), "two".to_string()];
    let item = db
        .create_item(
            &NewItem {
                acceptance_criteria: &criteria,
                ..new_item(&p, "Board")
            },
            &OWNER,
        )
        .unwrap();
    done(
        db.block_item(
            &item.id,
            item.version,
            Some((Some("H-009"), "waits on the contract")),
            &OWNER,
        )
        .unwrap(),
    );
    let card = db.board_cards(&p).unwrap().remove(0);
    assert!(card.blocked && !card.stale);
    assert_eq!((card.ac_checked, card.ac_total), (0, 2));
    let full = db.get_item(&item.id).unwrap().unwrap();
    assert_eq!(full.blocked.unwrap().by.as_deref(), Some("H-009"));
}

#[test]
fn links_are_recorded_once() {
    let (db, p) = board();
    let item = db.create_item(&new_item(&p, "Board"), &OWNER).unwrap();
    db.add_item_link(&item.id, LinkKind::Branch, "B2-board-schema", None, &OWNER)
        .unwrap();
    db.add_item_link(&item.id, LinkKind::Branch, "B2-board-schema", None, &OWNER)
        .unwrap();
    let linked = db
        .item_events(&item.id)
        .unwrap()
        .into_iter()
        .filter(|e| e.kind == ItemEventKind::Linked)
        .count();
    assert_eq!(linked, 1);
}
