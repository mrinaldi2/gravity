//! The check records (H-270): keyed by commit and name, a pass counting for
//! every commit with the same tree, dispatched to one runner, and a
//! migration that runs again without losing them.

use std::collections::BTreeMap;

use super::prs::NewPr;
use super::{Db, NewItem};
use crate::actor::Actor;
use crate::board::model::{ItemType, Platform, Priority};
use crate::prs::check_model::{CheckResult, NewCheck, Report};

fn project(db: &Db) -> String {
    let p = db.create_project("The Hermes", "the-hermes").unwrap();
    db.ensure_board(&p.id, "d-mac", Some("H")).unwrap();
    p.id
}

fn queued(name: &str) -> NewCheck {
    NewCheck {
        name: name.into(),
        run: format!("node scripts/verify.mjs --only {name}"),
        needs: vec!["cargo".into()],
        machine: None,
        required: true,
        result: CheckResult::Queued,
        note: None,
    }
}

fn queue(db: &Db, project: &str, sha: &str, tree: &str, names: &[&str]) {
    let checks: Vec<NewCheck> = names.iter().map(|n| queued(n)).collect();
    db.board_tx(|t| t.queue_checks(project, "o/r", sha, tree, &checks))
        .unwrap();
}

fn finish(db: &Db, project: &str, sha: &str, name: &str, result: CheckResult) {
    let tools = BTreeMap::from([("cargo".to_string(), "1.99.0".to_string())]);
    db.board_tx(|t| {
        let run = t.check_run(project, sha, name)?.unwrap();
        assert!(t.dispatch_check(&run.id, "bot-runner")?);
        t.report_check(
            &run.id,
            &Report {
                result,
                ran_on: "mac",
                log_artifact: Some("checks/x.log"),
                tool_versions: &tools,
            },
        )
    })
    .unwrap();
}

fn results(db: &Db, project: &str, sha: &str) -> Vec<(String, CheckResult, Option<String>)> {
    db.board_read(|t| t.checks_on(project, sha))
        .unwrap()
        .into_iter()
        .map(|c| (c.name, c.result, c.tree_of))
        .collect()
}

#[test]
fn a_pass_counts_for_every_commit_with_the_same_tree_and_nothing_else() {
    let db = Db::open_in_memory().unwrap();
    let p = project(&db);
    queue(&db, &p, "a1", "tree-a", &["rust", "docs"]);
    queue(&db, &p, "a2", "tree-a", &["rust", "docs"]);
    queue(&db, &p, "b1", "tree-b", &["rust"]);
    finish(&db, &p, "a1", "rust", CheckResult::Pass);
    finish(&db, &p, "a1", "docs", CheckResult::Fail);

    assert_eq!(
        results(&db, &p, "a2"),
        vec![
            ("docs".into(), CheckResult::Queued, None),
            ("rust".into(), CheckResult::Pass, Some("a1".into())),
        ],
        "a pass on the same tree counts; a fail doesn't"
    );
    let counted = db.board_read(|t| t.checks_on(&p, "a2")).unwrap();
    assert_eq!(counted[1].sha, "a2", "shown on the commit asked about");
    assert_eq!(counted[1].ran_on.as_deref(), Some("mac"));
    assert_eq!(
        results(&db, &p, "b1"),
        vec![("rust".into(), CheckResult::Queued, None)],
        "another tree is other files"
    );
}

#[test]
fn queueing_a_head_again_keeps_its_results() {
    let db = Db::open_in_memory().unwrap();
    let p = project(&db);
    queue(&db, &p, "a1", "tree-a", &["rust"]);
    finish(&db, &p, "a1", "rust", CheckResult::Fail);
    queue(&db, &p, "a1", "tree-a", &["rust"]);
    let run = db
        .board_read(|t| t.check_run(&p, "a1", "rust"))
        .unwrap()
        .unwrap();
    assert_eq!(run.result, CheckResult::Fail);
    assert_eq!(run.tool_versions["cargo"], "1.99.0");
    assert!(run.finished_at.is_some());
    assert!(
        !db.board_tx(|t| t.dispatch_check(&run.id, "other")).unwrap(),
        "only a queued check is dispatched"
    );
}

#[test]
fn only_a_heads_author_or_pusher_wrote_it() {
    let db = Db::open_in_memory().unwrap();
    let p = project(&db);
    let item = db
        .create_item(
            &NewItem {
                project_id: &p,
                item_type: ItemType::Feature,
                title: "one",
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
        .id;
    db.board_tx(|t| {
        t.insert_pr(&NewPr {
            project_id: &p,
            repo: "o/r",
            item_id: &item,
            branch: "one",
            base_sha: "b",
            head_sha: "h",
            patch_id: "p",
            author: "bot-dev",
            title: "t",
            change_note: "",
        })
    })
    .unwrap();
    let wrote = |bot: &str| db.board_read(|t| t.wrote_head(&p, "h", bot)).unwrap();
    assert!(wrote("bot-dev"));
    assert!(!wrote("bot-runner"));
}

#[test]
fn machine_tools_replace_what_the_computer_reported_before() {
    let db = Db::open_in_memory().unwrap();
    let first = BTreeMap::from([
        ("cargo".to_string(), "1.98.1".to_string()),
        ("node".to_string(), "24.1.0".to_string()),
    ]);
    let second = BTreeMap::from([("cargo".to_string(), "1.99.0".to_string())]);
    db.board_tx(|t| t.set_machine_tools("imac", &first))
        .unwrap();
    db.board_tx(|t| t.set_machine_tools("imac", &second))
        .unwrap();
    db.board_tx(|t| t.set_machine_tools("mac", &first)).unwrap();
    assert_eq!(db.board_read(|t| t.machine_tools("imac")).unwrap(), second);
    assert_eq!(db.board_read(|t| t.machine_tools("mac")).unwrap(), first);
}

#[test]
fn the_checks_migration_runs_again_without_losing_results() {
    let db = Db::open_in_memory().unwrap();
    let p = project(&db);
    queue(&db, &p, "a1", "tree-a", &["rust"]);
    finish(&db, &p, "a1", "rust", CheckResult::Pass);
    let tools = BTreeMap::from([("cargo".to_string(), "1.99.0".to_string())]);
    db.board_tx(|t| t.set_machine_tools("mac", &tools)).unwrap();
    let before = db.board_read(|t| t.check_run(&p, "a1", "rust")).unwrap();
    let ours: Vec<&str> = bus::schema::MIGRATIONS
        .iter()
        .copied()
        .filter(|m| m.contains("CREATE TABLE IF NOT EXISTS check_run ("))
        .collect();
    assert_eq!(ours.len(), 1);
    for sql in ours {
        db.lock().execute_batch(sql).unwrap();
    }
    assert_eq!(
        db.board_read(|t| t.check_run(&p, "a1", "rust")).unwrap(),
        before
    );
    assert_eq!(db.board_read(|t| t.machine_tools("mac")).unwrap(), tools);
}
