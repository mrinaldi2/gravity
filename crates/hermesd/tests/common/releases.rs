//! A project ready for releases: a board homed here, a lead, DevOps and a
//! tester on machine "mac", and items already in Verify.

use hermesd::actor::Actor;
use hermesd::board::model::{ItemType, Platform, Priority, ProjectRole, Role};
use hermesd::db::{MoveTo, NewItem};
use serde_json::{json, Value};

use super::tasks::{project_with_bots, Pair};
use super::McpClient;

pub struct Releases {
    pub pair: Pair,
    pub project: String,
    /// Team Lead, DevOps, Tester.
    pub bots: Vec<McpClient>,
    pub items: Vec<String>,
}

pub async fn releases(items: usize) -> Releases {
    let (pair, bots) = project_with_bots(&["Team Lead", "DevOps", "Tester"]).await;
    let db = &pair.d.app.db;
    let project = db.get_bot(&pair.ids[0]).unwrap().unwrap().project_id;
    db.ensure_board(&project, &db.daemon_id().unwrap(), Some("H"))
        .unwrap();
    db.set_project_role(&ProjectRole {
        project_id: project.clone(),
        role: Role::Tester,
        bot_id: pair.ids[2].clone(),
        machine: Some("mac".into()),
    })
    .unwrap();
    let items = (0..items)
        .map(|n| {
            let title = format!("Item {n}");
            let item = db
                .create_item(
                    &NewItem {
                        project_id: &project,
                        item_type: ItemType::Feature,
                        title: &title,
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
                .unwrap();
            let to = MoveTo {
                column: "verify",
                ..MoveTo::default()
            };
            db.move_item(&item.id, item.version, &to, &Actor::User)
                .unwrap();
            item.id
        })
        .collect();
    Releases {
        pair,
        project,
        bots,
        items,
    }
}

impl Releases {
    /// DevOps creates, builds and submits a package of every item; returns
    /// the submitted release's JSON.
    pub async fn submitted(&mut self, name: &str) -> Value {
        let items = self.items.clone();
        self.package(name, json!({"name": name, "items": items}))
            .await
    }

    /// DevOps creates a package from `create`'s arguments, attaches its
    /// build, the tester passes it on "mac", and DevOps submits it.
    pub async fn package(&mut self, name: &str, create: Value) -> Value {
        let created = self.bots[1].call("release_create", create).await;
        let id = created["release"]["id"].as_str().unwrap().to_string();
        self.bots[1]
            .call(
                "release_attach_build",
                json!({"release_id": id, "platform": "daemon", "version": name,
                       "artifact": format!("/builds/{name}"), "sha256": "a".repeat(64)}),
            )
            .await;
        self.passed(&id).await;
        self.bots[1]
            .call("release_submit", json!({"release_id": id}))
            .await["release"]
            .clone()
    }

    /// The tester's pass on "mac" against the package's build.
    pub async fn passed(&mut self, id: &str) {
        self.bots[2]
            .call(
                "release_test",
                json!({"release_id": id, "machine": "mac", "build_sha256": "a".repeat(64),
                       "result": "pass"}),
            )
            .await;
    }

    pub fn column(&self, item: &str) -> String {
        self.pair
            .d
            .app
            .db
            .get_item(item)
            .unwrap()
            .unwrap()
            .column_key
    }
}

/// The owner's ruling over the WebSocket.
pub async fn rule(owner: &mut super::WsClient, release: &Value, verdicts: Value) -> Value {
    owner
        .request(json!({"type": "release_rule", "release_id": release["id"],
                        "verdicts": verdicts, "expected_version": release["version"]}))
        .await
}
