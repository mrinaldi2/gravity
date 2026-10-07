//! `paths::no_follow` against links a bot planted (H-182). Fixtures only.
//!
//! Every kind of link a bot can make is tried: symlinks on Unix; on Windows
//! junctions (`mklink /J`, no privilege needed), plus directory and file
//! symlinks when this account may make them (Developer Mode or admin).

use super::*;

/// Makes a link at `link` to `target`; false when this account may not.
type MakeLink = fn(&Path, &Path) -> bool;

/// The folder links a bot can plant here, by name.
fn folder_links() -> Vec<(&'static str, MakeLink)> {
    #[cfg(unix)]
    return vec![("symlink", file_link)];
    #[cfg(windows)]
    return vec![
        ("junction", junction),
        ("dir symlink", |t, l| {
            privileged(std::os::windows::fs::symlink_dir(t, l))
        }),
    ];
}

/// A link to a file; false when Windows refuses it to this account.
fn file_link(target: &Path, link: &Path) -> bool {
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(target, link).unwrap();
        true
    }
    #[cfg(windows)]
    privileged(std::os::windows::fs::symlink_file(target, link))
}

/// A junction, which any Windows user may make (dangling too).
#[cfg(windows)]
fn junction(target: &Path, link: &Path) -> bool {
    let status = std::process::Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(link)
        .arg(target)
        .stdout(std::process::Stdio::null())
        .status()
        .unwrap();
    assert!(status.success(), "mklink /J {}", link.display());
    true
}

/// A symlink made, or skipped (false) when the account lacks the privilege.
#[cfg(windows)]
fn privileged(made: std::io::Result<()>) -> bool {
    match made {
        Ok(()) => true,
        // ERROR_PRIVILEGE_NOT_HELD: no Developer Mode and not elevated.
        Err(error) if error.raw_os_error() == Some(1314) => {
            eprintln!("skipped: symlinks need Developer Mode or admin on this account");
            false
        }
        Err(error) => panic!("making a symlink: {error}"),
    }
}

fn refused(error: anyhow::Error) {
    assert!(error.downcast_ref::<LinkRefused>().is_some(), "{error:#}");
}

#[test]
fn writes_plain_folders_and_refuses_a_link_at_any_part() {
    for (kind, link) in folder_links() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path().join("bot/workspace");
        std::fs::create_dir_all(&base).unwrap();
        let rel = Path::new(".claude/settings.json");
        write(&base, rel, b"{}").unwrap();
        assert_eq!(std::fs::read(base.join(rel)).unwrap(), b"{}");

        // The owner's config, faked: a bot links its .claude there.
        let owner = dir.path().join("owner/.claude");
        std::fs::create_dir_all(&owner).unwrap();
        std::fs::write(owner.join("settings.json"), "owner").unwrap();
        std::fs::remove_dir_all(base.join(".claude")).unwrap();
        if !link(&owner, &base.join(".claude")) {
            continue;
        }
        refused(write(&base, rel, b"bot").unwrap_err());
        refused(create_new(&base, Path::new(".claude/new.md"), b"bot").unwrap_err());
        assert_eq!(read(&base, rel), None, "{kind}");
        assert_eq!(
            std::fs::read_to_string(owner.join("settings.json")).unwrap(),
            "owner",
            "{kind}"
        );
        assert_eq!(
            std::fs::read_dir(&owner).unwrap().count(),
            1,
            "{kind}: no temp left there"
        );

        // `base` itself a link: refused too.
        let linked = dir.path().join("bot/linked");
        assert!(link(&owner, &linked));
        refused(write(&linked, Path::new("settings.json"), b"bot").unwrap_err());
        assert_eq!(
            std::fs::read_to_string(owner.join("settings.json")).unwrap(),
            "owner",
            "{kind}"
        );
    }
}

#[test]
fn a_file_link_at_the_name_is_replaced_not_written_through() {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().join("bot");
    std::fs::create_dir_all(&base).unwrap();
    let victim = dir.path().join("owner.json");
    std::fs::write(&victim, "owner").unwrap();
    if !file_link(&victim, &base.join("mcp.json")) {
        return;
    }
    write(&base, Path::new("mcp.json"), b"bot").unwrap();
    assert_eq!(std::fs::read_to_string(&victim).unwrap(), "owner");
    let meta = std::fs::symlink_metadata(base.join("mcp.json")).unwrap();
    assert!(meta.is_file(), "the link itself was replaced");
    // And reading never goes through a link.
    assert!(file_link(&victim, &base.join("bot.json")));
    assert_eq!(read(&base, Path::new("bot.json")), None);
    assert_eq!(read(&base, Path::new("mcp.json")).as_deref(), Some("bot"));
}

#[test]
fn a_folder_link_at_the_name_is_replaced_never_followed() {
    for (kind, link) in folder_links() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path().join("bot");
        std::fs::create_dir_all(&base).unwrap();
        let owner = dir.path().join("owner");
        std::fs::create_dir_all(&owner).unwrap();
        std::fs::write(owner.join("keep"), "owner").unwrap();
        if !link(&owner, &base.join("mcp.json")) {
            continue;
        }
        assert_eq!(read(&base, Path::new("mcp.json")), None, "{kind}");
        // Replaced as on Unix, not "Access is denied": the link goes, its
        // target stays as it was.
        write(&base, Path::new("mcp.json"), b"bot").unwrap();
        let meta = std::fs::symlink_metadata(base.join("mcp.json")).unwrap();
        assert!(meta.is_file(), "{kind}: the link itself was replaced");
        assert_eq!(read(&base, Path::new("mcp.json")).as_deref(), Some("bot"));
        assert_eq!(std::fs::read_dir(&owner).unwrap().count(), 1, "{kind}");
        assert_eq!(
            std::fs::read_to_string(owner.join("keep")).unwrap(),
            "owner"
        );
    }
}

#[test]
fn create_new_never_follows_a_dangling_link() {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().join("ws");
    std::fs::create_dir_all(&base).unwrap();
    let target = dir.path().join("owner-rc");
    if file_link(&target, &base.join("FACTS.md")) {
        assert!(!create_new(&base, Path::new("FACTS.md"), b"x").unwrap());
        assert!(!target.exists(), "nothing created through the link");
    }
    for (kind, link) in folder_links() {
        let name = format!("{}.md", kind.replace(' ', "-"));
        if link(&target, &base.join(&name)) {
            assert!(
                !create_new(&base, Path::new(&name), b"x").unwrap(),
                "{kind}"
            );
            assert!(!target.exists(), "{kind}: nothing created through it");
        }
    }
    assert!(create_new(&base, Path::new("CLAUDE.md"), b"x").unwrap());
    assert!(!create_new(&base, Path::new("CLAUDE.md"), b"y").unwrap());
    assert!(write(&base, Path::new("../escape"), b"x").is_err());
}
