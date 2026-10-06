//! H-171: a daemon started on a copy of another home's database rewrote the
//! LIVE bots' `mcp.json` and settings (their workspace paths are stored
//! absolute). A daemon now writes only into bot folders inside its own home,
//! and a scratch daemon starts, links and writes nothing. Fixtures only: no
//! real home is ever read here.

mod common;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use bus::BotState;
use common::*;

/// Every file under `root` and its content, to compare before and after.
fn snapshot(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut files = BTreeMap::new();
    let mut dirs = vec![root.to_path_buf()];
    while let Some(dir) = dirs.pop() {
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                dirs.push(path);
            } else {
                files.insert(path.clone(), std::fs::read(&path).unwrap());
            }
        }
    }
    files
}

/// Another home's bot, as a copied database names it, with its files.
fn other_home(dir: &Path) -> (PathBuf, PathBuf) {
    let root = dir.join("live/projects/p/bots/dev");
    let workspace = root.join("workspace");
    std::fs::create_dir_all(workspace.join(".claude")).unwrap();
    std::fs::write(root.join("mcp.json"), "{\"live\": true}").unwrap();
    std::fs::write(root.join("settings.gen.json"), "{\"live\": true}").unwrap();
    std::fs::write(workspace.join(".claude/settings.json"), "{\"live\": true}").unwrap();
    (dir.join("live"), workspace)
}

#[tokio::test]
async fn a_copied_database_naming_another_homes_bots_writes_nothing_there() {
    let dir = tempfile::tempdir().unwrap();
    let (live, workspace) = other_home(dir.path());
    let before = snapshot(&live);

    let d = spawn_daemon().await;
    let db = &d.app.db;
    let project = db.create_project("p", "p").unwrap();
    let bot = db
        .create_bot(
            &project.id,
            "dev",
            "",
            "",
            "",
            workspace.to_str().unwrap(),
            "dev",
            None,
        )
        .unwrap();
    d.app.supervisor.start_bot(&bot.id).unwrap();
    // And the supervision tick, which brings every live bot up.
    tokio::time::sleep(Duration::from_millis(1500)).await;

    assert_eq!(snapshot(&live), before, "the other home is untouched");
    assert_ne!(d.app.supervisor.state(&bot.id).0, BotState::Ready);
    assert_ne!(d.app.supervisor.state(&bot.id).0, BotState::Working);
}

#[tokio::test]
async fn scratch_mode_starts_links_and_writes_nothing() {
    let d = spawn_daemon_with(|cfg| cfg.scratch = true).await;
    let db = &d.app.db;
    let project = db.create_project("p", "p").unwrap();
    // A bot in this home's own folders: still left alone in scratch mode.
    let workspace = d.app.cfg.projects_dir().join("p/bots/dev/workspace");
    let bot = db
        .create_bot(
            &project.id,
            "dev",
            "",
            "",
            "",
            workspace.to_str().unwrap(),
            "dev",
            None,
        )
        .unwrap();
    d.app.supervisor.start_bot(&bot.id).unwrap();
    tokio::time::sleep(Duration::from_millis(1500)).await;
    assert!(!workspace.exists(), "nothing was written for the bot");
    assert!(!d
        .app
        .cfg
        .projects_dir()
        .join("p/bots/dev/mcp.json")
        .exists());
    assert_ne!(d.app.supervisor.state(&bot.id).0, BotState::Ready);

    // No computer links with it.
    let url = format!("http://{}/peer", d.addr);
    let status = reqwest::Client::new()
        .get(&url)
        .header("connection", "upgrade")
        .header("upgrade", "websocket")
        .header("sec-websocket-version", "13")
        .header("sec-websocket-key", "dGhlIHNhbXBsZSBub25jZQ==")
        .send()
        .await
        .unwrap()
        .status();
    assert_eq!(status, reqwest::StatusCode::SERVICE_UNAVAILABLE);

    // The owner's WebSocket still answers: it is for looking.
    let mut owner = WsClient::connect(&d).await;
    let projects = owner
        .request(serde_json::json!({"type": "list_projects"}))
        .await;
    assert_eq!(projects["type"], "projects", "{projects}");
}
