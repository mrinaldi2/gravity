//! The PR records (H-266): numbered per project, one live PR per card, and
//! a migration that runs again without losing them.

use super::prs::NewPr;
use super::{Db, NewItem};
use crate::actor::Actor;
use crate::board::model::{ItemType, Platform, Priority};
use crate::prs::model::PrState;

fn card(db: &Db, project: &str, title: &str) -> String {
    db.create_item(
        &NewItem {
            project_id: project,
            item_type: ItemType::Feature,
            title,
            description: "",
            platforms: &[Platform::Daemon],
            size: None,
            priority: Priority::P1,
            labels: &[],
            parent_id: None,
            acceptance_criteria: &[],
        },
        &Actor::User,
    )
    .unwrap()
    .id
}

fn new_pr<'a>(project: &'a str, item: &'a str, branch: &'a str) -> NewPr<'a> {
    NewPr {
        project_id: project,
        repo: "mrinaldi2/gravity",
        item_id: item,
        branch,
        base_sha: "b",
        head_sha: "h",
        patch_id: "p",
        author: "bot-dev",
        title: "t",
        change_note: "",
    }
}

#[test]
fn prs_are_numbered_per_project_and_a_card_has_one_live_pr() {
    let db = Db::open_in_memory().unwrap();
    let a = db.create_project("A", "a").unwrap();
    let b = db.create_project("B", "b").unwrap();
    db.ensure_board(&a.id, "d-mac", Some("A")).unwrap();
    db.ensure_board(&b.id, "d-mac", Some("B")).unwrap();
    let (a1, a2, b1) = (
        card(&db, &a.id, "one"),
        card(&db, &a.id, "two"),
        card(&db, &b.id, "x"),
    );
    let first = db
        .board_tx(|t| t.insert_pr(&new_pr(&a.id, &a1, "one")))
        .unwrap();
    let second = db
        .board_tx(|t| t.insert_pr(&new_pr(&a.id, &a2, "two")))
        .unwrap();
    let other = db
        .board_tx(|t| t.insert_pr(&new_pr(&b.id, &b1, "x")))
        .unwrap();
    assert_eq!((first.number, second.number, other.number), (1, 2, 1));
    assert!(
        db.board_tx(|t| t.insert_pr(&new_pr(&a.id, &a1, "again")))
            .is_err(),
        "a card's second live PR is refused by the index"
    );
    db.board_tx(|t| t.close_pr(&first, "no")).unwrap();
    let reopened = db
        .board_tx(|t| t.insert_pr(&new_pr(&a.id, &a1, "again")))
        .unwrap();
    assert_eq!(
        reopened.number, 3,
        "a closed PR frees its card; numbers never repeat"
    );
    let open = db.board_read(|t| t.prs(&a.id, &[PrState::Open])).unwrap();
    assert_eq!(
        open.iter().map(|p| p.number).collect::<Vec<_>>(),
        vec![3, 2]
    );
}

#[test]
fn the_pr_migration_runs_again_without_losing_prs() {
    let db = Db::open_in_memory().unwrap();
    let p = db.create_project("The Hermes", "the-hermes").unwrap();
    db.ensure_board(&p.id, "d-mac", Some("H")).unwrap();
    let item = card(&db, &p.id, "one");
    let pr = db
        .board_tx(|t| t.insert_pr(&new_pr(&p.id, &item, "one")))
        .unwrap();
    let ours: Vec<&str> = bus::schema::MIGRATIONS
        .iter()
        .copied()
        .filter(|m| m.contains("CREATE TABLE IF NOT EXISTS pr ("))
        .collect();
    assert_eq!(ours.len(), 1);
    for sql in ours {
        db.lock().execute_batch(sql).unwrap();
    }
    let again = db.board_read(|t| t.pr(&p.id, 1)).unwrap().unwrap();
    assert_eq!(again, pr);
    let pushes = db.board_read(|t| t.pr_pushes(&pr.id)).unwrap();
    assert_eq!(pushes.len(), 1);
    assert_eq!(pushes[0].pushed_by.as_deref(), Some("bot-dev"));
}
