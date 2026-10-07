//! H-182 (CE-025): a bot linked its workspace's `.claude` to the owner's
//! `~/.claude`; on the bot's next start the daemon would have written its
//! hook settings there, over the owner's own. The daemon writes a bot's
//! files through no link. A fake owner home stands in for the real one.
//!
//! On Windows the folder links are junctions (`mklink /J`), which any user
//! may make. A file link needs Developer Mode or admin there; without it a
//! junction stands in at that name.

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

/// A folder link (dangling or not): a symlink on Unix, a junction on Windows.
fn folder_link(target: &Path, link: &Path) {
    #[cfg(unix)]
    std::os::unix::fs::symlink(target, link).unwrap();
    #[cfg(windows)]
    {
        let status = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(link)
            .arg(target)
            .stdout(std::process::Stdio::null())
            .status()
            .unwrap();
        assert!(status.success(), "mklink /J {}", link.display());
    }
}

/// A link to a file; false when Windows refuses it to this account.
fn file_link(target: &Path, link: &Path) -> bool {
    #[cfg(unix)]
    let made = std::os::unix::fs::symlink(target, link);
    #[cfg(windows)]
    let made = std::os::windows::fs::symlink_file(target, link);
    match made {
        Ok(()) => true,
        // ERROR_PRIVILEGE_NOT_HELD: no Developer Mode and not elevated.
        Err(error) if cfg!(windows) && error.raw_os_error() == Some(1314) => {
            eprintln!("no file link: it needs Developer Mode or admin; a junction stands in");
            false
        }
        Err(error) => panic!("making a link: {error}"),
    }
}

#[tokio::test]
async fn a_bot_start_writes_nothing_through_links_the_bot_planted() {
    let owner = tempfile::tempdir().unwrap();
    let owner_claude = owner.path().join(".claude");
    std::fs::create_dir_all(&owner_claude).unwrap();
    std::fs::write(owner_claude.join("settings.json"), "owner settings").unwrap();
    let owner_rc = owner.path().join("owner.json");
    std::fs::write(&owner_rc, "owner file").unwrap();
    let owner_dir = owner.path().join("docs");
    std::fs::create_dir_all(&owner_dir).unwrap();
    std::fs::write(owner_dir.join("keep"), "owner doc").unwrap();

    let user_home = owner.path().to_path_buf();
    let d = spawn_daemon_with(move |cfg| cfg.user_home = user_home).await;
    let db = &d.app.db;
    let project = db.create_project("p", "p").unwrap();
    let root = d.app.cfg.projects_dir().join("p/bots/dev");
    let workspace = root.join("workspace");
    std::fs::create_dir_all(&workspace).unwrap();
    // What the bot planted: its .claude is the owner's, its mcp.json and
    // the memory files name the owner's files and folders.
    folder_link(&owner_claude, &workspace.join(".claude"));
    if !file_link(&owner_rc, &root.join("mcp.json")) {
        folder_link(&owner_dir, &root.join("mcp.json"));
    }
    folder_link(&owner.path().join("dangling"), &workspace.join("FACTS.md"));
    folder_link(&owner_dir, &workspace.join("CLAUDE.md"));
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
    assert_eq!(listing(&owner_dir), ["keep"], "nothing written in it");
    assert_eq!(
        std::fs::read_to_string(owner_dir.join("keep")).unwrap(),
        "owner doc"
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
