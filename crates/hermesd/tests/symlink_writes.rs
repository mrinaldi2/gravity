//! H-182 (CE-025): a bot linked its workspace's `.claude` to the owner's
//! `~/.claude`; on the bot's next start the daemon would have written its
//! hook settings there, over the owner's own. The daemon writes a bot's
//! files through no link. A fake owner home stands in for the real one.

#![cfg(unix)]

mod common;

use std::path::Path;

use common::*;

fn listing(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

#[tokio::test]
async fn a_bot_start_writes_nothing_through_links_the_bot_planted() {
    let owner = tempfile::tempdir().unwrap();
    let owner_claude = owner.path().join(".claude");
    std::fs::create_dir_all(&owner_claude).unwrap();
    std::fs::write(owner_claude.join("settings.json"), "owner settings").unwrap();
    let owner_rc = owner.path().join("owner.json");
    std::fs::write(&owner_rc, "owner file").unwrap();

    let user_home = owner.path().to_path_buf();
    let d = spawn_daemon_with(move |cfg| cfg.user_home = user_home).await;
    let db = &d.app.db;
    let project = db.create_project("p", "p").unwrap();
    let root = d.app.cfg.projects_dir().join("p/bots/dev");
    let workspace = root.join("workspace");
    std::fs::create_dir_all(&workspace).unwrap();
    // What the bot planted: its .claude is the owner's, its mcp.json and
    // a memory file name the owner's files.
    std::os::unix::fs::symlink(&owner_claude, workspace.join(".claude")).unwrap();
    std::os::unix::fs::symlink(&owner_rc, root.join("mcp.json")).unwrap();
    std::os::unix::fs::symlink(owner.path().join("dangling"), workspace.join("FACTS.md")).unwrap();
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

    let _ = d.app.supervisor.start_bot(&bot.id);

    assert_eq!(
        std::fs::read_to_string(owner_claude.join("settings.json")).unwrap(),
        "owner settings"
    );
    assert_eq!(
        listing(&owner_claude),
        ["settings.json"],
        "no temp left in the owner's folder"
    );
    assert_eq!(std::fs::read_to_string(&owner_rc).unwrap(), "owner file");
    assert!(
        !owner.path().join("dangling").exists(),
        "nothing created through a link"
    );
    // mcp.json, a link at the name itself, is replaced: the bot's own file now.
    let mcp = std::fs::symlink_metadata(root.join("mcp.json")).unwrap();
    assert!(
        mcp.is_file(),
        "the planted link was replaced, not written through"
    );
    // The .claude link is left as it was, unwritten.
    assert!(std::fs::symlink_metadata(workspace.join(".claude"))
        .unwrap()
        .file_type()
        .is_symlink());
}
