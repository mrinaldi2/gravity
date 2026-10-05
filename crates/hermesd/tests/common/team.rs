//! A project with a board and a team of bots, and items placed on it, for
//! the board workflow tests (H-099).

use hermesd::actor::Actor;
use hermesd::board::model::{ItemType, Platform, Priority};
use hermesd::db::{MoveTo, NewItem};
use serde_json::json;

use super::tasks::{project_with_bots, Pair};
use super::McpClient;

pub const OWNER: Actor<'static> = Actor::User;

/// A project with a board and these bots, its first one the lead.
pub async fn team(names: &[&str]) -> (Pair, Vec<McpClient>, String) {
    let (pair, bots) = project_with_bots(names).await;
    let db = &pair.d.app.db;
    let project = db.get_bot(&pair.ids[0]).unwrap().unwrap().project_id;
    db.set_project_lead(&project, Some(&pair.ids[0])).unwrap();
    db.ensure_board(&project, &db.daemon_id().unwrap(), Some("H"))
        .unwrap();
    (pair, bots, project)
}

/// An item in `column`, assigned to `assignee`.
pub fn item(pair: &Pair, project: &str, column: &str, assignee: Option<&str>) -> String {
    let db = &pair.d.app.db;
    let item = db
        .create_item(
            &NewItem {
                project_id: project,
                item_type: ItemType::Feature,
                title: "Workflow",
                description: "",
                platforms: &[Platform::Daemon],
                size: None,
                priority: Priority::P1,
                labels: &[],
                parent_id: None,
                acceptance_criteria: &[],
            },
            &OWNER,
        )
        .unwrap();
    let to = MoveTo {
        column,
        ..MoveTo::default()
    };
    let moved = db.move_item(&item.id, item.version, &to, &OWNER).unwrap();
    let version = match moved {
        hermesd::db::Write::Done(item) => item.version,
        hermesd::db::Write::Conflict(_) => unreachable!(),
    };
    db.assign_item(&item.id, version, assignee, &OWNER).unwrap();
    item.id
}

pub async fn get(bot: &mut McpClient, id: &str) -> serde_json::Value {
    bot.call("item_get", json!({ "id": id })).await["item"].clone()
}

/// The item's version as `expected_version` takes it.
pub async fn version_of(bot: &mut McpClient, id: &str) -> String {
    get(bot, id).await["version"].as_str().unwrap().to_string()
}
