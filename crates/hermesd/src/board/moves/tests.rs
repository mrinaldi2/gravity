//! The move engine against a real (in-memory) board: guards, versions,
//! history and the WIP override flag.

use super::*;
use crate::board::guards::WIP_OVERRIDE_LABEL;
use crate::board::model::{ItemEventKind, ItemType, LinkKind, Platform, Priority, Size};
use crate::db::{NewItem, Write};

struct Board {
    db: Db,
    project: String,
    lead: String,
    dev: String,
}

fn board() -> Board {
    let db = Db::open_in_memory().unwrap();
    let p = db.create_project("The Hermes", "the-hermes").unwrap();
    let bot = |name: &str, dir: &str| {
        db.create_bot(&p.id, name, "", "", "", "/tmp/x", dir, None)
            .unwrap()
            .id
    };
    let lead = bot("Team Lead", "team-lead");
    let dev = bot("Desktop Dev", "desktop-dev");
    db.ensure_board(&p.id, "d-mac", Some("H")).unwrap();
    Board {
        db,
        project: p.id,
        lead,
        dev,
    }
}

impl Board {
    fn as_bot<'a>(&'a self, id: &'a str) -> Actor<'a> {
        Actor::Bot {
            id,
            project_id: &self.project,
        }
    }

    fn item(&self, title: &str) -> Item {
        let ac = ["it works".to_string()];
        self.db
            .create_item(
                &NewItem {
                    project_id: &self.project,
                    item_type: ItemType::Feature,
                    title,
                    description: "",
                    platforms: &[Platform::Daemon],
                    size: Some(Size::M),
                    priority: Priority::P2,
                    labels: &[],
                    parent_id: None,
                    acceptance_criteria: &ac,
                },
                &Actor::User,
            )
            .unwrap()
    }

    fn link(&self, item: &Item, kind: LinkKind, by: &Actor<'_>) {
        self.db
            .add_item_link(
                &item.id,
                kind,
                &format!("{}:{}", kind.as_str(), item.id),
                None,
                by,
            )
            .unwrap();
    }

    fn moved(&self, item: &Item, to: &str, actor: &Actor<'_>, over: Option<&str>) -> Moved {
        let req = MoveRequest {
            id: &item.id,
            to,
            expected_version: item.version,
            reason: None,
            override_reason: over,
        };
        item_move(&self.db, &req, actor).unwrap()
    }

    /// An item in Ready, assigned to the dev, with its task linked.
    fn ready_for_dev(&self, title: &str) -> Item {
        let item = self.item(title);
        self.link(&item, LinkKind::Artifact, &Actor::User);
        self.link(&item, LinkKind::Task, &Actor::User);
        let item = done(self.moved(&item, "ready", &self.as_bot(&self.lead), None));
        match self
            .db
            .assign_item(&item.id, item.version, Some(&self.dev), &Actor::User)
            .unwrap()
        {
            Write::Done(item) => item,
            Write::Conflict(_) => panic!("conflict"),
        }
    }
}

fn done(moved: Moved) -> Item {
    match moved {
        Moved::Done(item) => *item,
        other => panic!("expected a move, got {other:?}"),
    }
}

fn codes(moved: Moved) -> Vec<String> {
    match moved {
        Moved::Refused(unmet) => unmet.into_iter().map(|u| u.code).collect(),
        other => panic!("expected a refusal, got {other:?}"),
    }
}

#[test]
fn the_lead_refines_once_the_definition_of_ready_is_met() {
    let b = board();
    let lead = b.as_bot(&b.lead);
    let item = b.item("Guards");
    assert_eq!(
        codes(b.moved(&item, "ready", &lead, None)),
        ["dor.spec_link_for_ui_or_daemon"]
    );
    assert_eq!(
        b.db.get_item(&item.id).unwrap().unwrap().column_key,
        "inbox"
    );
    assert_eq!(
        codes(b.moved(&item, "ready", &b.as_bot(&b.dev), None)),
        ["role.not_allowed", "dor.spec_link_for_ui_or_daemon"]
    );

    b.link(&item, LinkKind::Artifact, &lead);
    let ready = done(b.moved(&item, "ready", &lead, None));
    assert_eq!(
        (ready.column_key.as_str(), ready.version),
        ("ready", item.version + 1)
    );
}

#[test]
fn a_stale_version_is_a_conflict_before_any_guard() {
    let b = board();
    let item = b.item("Stale");
    let mut old = item.clone();
    old.version -= 1;
    match b.moved(&old, "cancelled", &Actor::User, None) {
        Moved::Conflict(current) => assert_eq!(current.version, item.version),
        other => panic!("expected a conflict, got {other:?}"),
    }
}

#[test]
fn a_wip_override_is_logged_and_flags_the_card_until_it_leaves() {
    let b = board();
    let lead = b.as_bot(&b.lead);
    let first = b.ready_for_dev("First");
    done(b.moved(&first, "doing", &b.as_bot(&b.dev), None));

    let second = b.ready_for_dev("Second");
    assert_eq!(codes(b.moved(&second, "doing", &lead, None)), ["wip.full"]);
    let over = done(b.moved(&second, "doing", &lead, Some("owner asked for both")));
    assert!(over.labels.contains(&WIP_OVERRIDE_LABEL.to_string()));
    let event =
        b.db.item_events(&second.id)
            .unwrap()
            .into_iter()
            .last()
            .unwrap();
    assert_eq!(event.kind, ItemEventKind::Moved);
    assert_eq!(
        event.note.as_deref(),
        Some("WIP override: owner asked for both")
    );

    b.link(&over, LinkKind::Branch, &b.as_bot(&b.dev));
    let review = done(b.moved(&over, "review", &b.as_bot(&b.dev), None));
    assert!(review.labels.is_empty());
}

#[test]
fn the_check_lists_every_other_column_for_the_asker() {
    let b = board();
    let item = b.item("Check");
    let check = item_move_check(&b.db, &item.id, &b.as_bot(&b.dev)).unwrap();
    assert!(check.iter().all(|(c, _)| c.key != "inbox"));
    let cancel = &check.iter().find(|(c, _)| c.key == "cancelled").unwrap().1;
    let cancel: Vec<&str> = cancel.iter().map(|u| u.code.as_str()).collect();
    assert_eq!(cancel, ["role.not_allowed", "reason.required"]);
    let deploying = &check.iter().find(|(c, _)| c.key == "deploying").unwrap().1;
    assert_eq!(deploying[0].code, "move.daemon_only");
}

#[test]
fn a_bot_of_another_project_is_turned_away() {
    let b = board();
    let item = b.item("Elsewhere");
    let stranger = Actor::Bot {
        id: &b.dev,
        project_id: "another",
    };
    let req = MoveRequest {
        id: &item.id,
        to: "cancelled",
        expected_version: item.version,
        reason: Some("no"),
        override_reason: None,
    };
    assert!(item_move(&b.db, &req, &stranger).is_err());
    assert!(item_move_check(&b.db, &item.id, &stranger).is_err());
}
