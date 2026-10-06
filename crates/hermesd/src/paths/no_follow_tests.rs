//! `paths::no_follow` against links a bot planted (H-182). Fixtures only.

use super::*;

#[test]
fn writes_plain_folders_and_refuses_a_link_at_any_part() {
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
    std::os::unix::fs::symlink(&owner, base.join(".claude")).unwrap();
    let error = write(&base, rel, b"bot").unwrap_err();
    assert!(error.downcast_ref::<LinkRefused>().is_some(), "{error:#}");
    assert_eq!(
        std::fs::read_to_string(owner.join("settings.json")).unwrap(),
        "owner"
    );
    assert_eq!(
        std::fs::read_dir(&owner).unwrap().count(),
        1,
        "no temp left there"
    );

    // `base` itself a link: refused too.
    let link = dir.path().join("bot/linked");
    std::os::unix::fs::symlink(&owner, &link).unwrap();
    assert!(write(&link, Path::new("settings.json"), b"bot").is_err());
    assert_eq!(
        std::fs::read_to_string(owner.join("settings.json")).unwrap(),
        "owner"
    );
}

#[test]
fn a_link_at_the_name_is_replaced_not_written_through() {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().join("bot");
    std::fs::create_dir_all(&base).unwrap();
    let victim = dir.path().join("owner.json");
    std::fs::write(&victim, "owner").unwrap();
    std::os::unix::fs::symlink(&victim, base.join("mcp.json")).unwrap();
    write(&base, Path::new("mcp.json"), b"bot").unwrap();
    assert_eq!(std::fs::read_to_string(&victim).unwrap(), "owner");
    let meta = std::fs::symlink_metadata(base.join("mcp.json")).unwrap();
    assert!(meta.is_file(), "the link itself was replaced");
    // And reading never goes through a link.
    std::os::unix::fs::symlink(&victim, base.join("bot.json")).unwrap();
    assert_eq!(read(&base, Path::new("bot.json")), None);
    assert_eq!(read(&base, Path::new("mcp.json")).as_deref(), Some("bot"));
}

#[test]
fn create_new_never_follows_a_dangling_link() {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().join("ws");
    std::fs::create_dir_all(&base).unwrap();
    let target = dir.path().join("owner-rc");
    std::os::unix::fs::symlink(&target, base.join("FACTS.md")).unwrap();
    assert!(!create_new(&base, Path::new("FACTS.md"), b"x").unwrap());
    assert!(!target.exists(), "nothing created through the link");
    assert!(create_new(&base, Path::new("CLAUDE.md"), b"x").unwrap());
    assert!(!create_new(&base, Path::new("CLAUDE.md"), b"y").unwrap());
    assert!(write(&base, Path::new("../escape"), b"x").is_err());
}
