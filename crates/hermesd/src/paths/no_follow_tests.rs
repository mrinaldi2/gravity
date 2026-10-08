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
        .arg(backslashed(link))
        .arg(backslashed(target))
        .stdout(std::process::Stdio::null())
        .status()
        .unwrap();
    assert!(status.success(), "mklink /J {}", link.display());
    true
}

/// A path as cmd's `mklink` takes it: it reads `/` as a switch.
#[cfg(windows)]
fn backslashed(path: &Path) -> String {
    path.to_string_lossy().replace('/', "\\")
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
        let base = dir.path().join("bot").join("workspace");
        std::fs::create_dir_all(&base).unwrap();
        let rel = Path::new(".claude/settings.json");
        write(&base, rel, b"{}").unwrap();
        assert_eq!(std::fs::read(base.join(rel)).unwrap(), b"{}");

        // The owner's config, faked: a bot links its .claude there.
        let owner = dir.path().join("owner").join(".claude");
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
        let linked = dir.path().join("bot").join("linked");
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

/// H-184 (CE-026 F1): a bot flipping a folder between a plain one and a link
/// to the owner's folder while the daemon writes below it. Every write lands
/// in the plain folder or is refused; none ever reaches the link's target.
/// On Windows the old walk checked by path and then renamed by path, so a
/// junction swapped in between was followed.
#[test]
fn a_folder_swapped_for_a_link_mid_write_is_never_followed() {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::Arc;
    for (kind, link) in folder_links() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path().join("bot");
        let owner = dir.path().join("owner");
        std::fs::create_dir_all(base.join("sub")).unwrap();
        std::fs::create_dir_all(&owner).unwrap();
        if !link(&owner, &base.join("sub.link")) {
            continue;
        }
        let stop = Arc::new(AtomicBool::new(false));
        let swaps = Arc::new(AtomicUsize::new(0));
        let swapper = {
            let (base, stop, swaps) = (base.clone(), stop.clone(), swaps.clone());
            std::thread::spawn(move || {
                let at = |name: &str| base.join(name);
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
                // Each step may fail while the daemon has a part open; a stuck
                // one ends the test rather than hanging it.
                let retry = |step: &dyn Fn() -> std::io::Result<()>| {
                    while step().is_err() {
                        assert!(std::time::Instant::now() < deadline, "the swap is stuck");
                        std::thread::yield_now();
                    }
                };
                while !stop.load(Ordering::SeqCst) {
                    if std::fs::rename(at("sub"), at("sub.real")).is_err() {
                        continue;
                    }
                    // The link in, unless a write just made a fresh `sub`.
                    if std::fs::rename(at("sub.link"), at("sub")).is_ok() {
                        retry(&|| std::fs::rename(at("sub"), at("sub.link")));
                    }
                    // A `sub` a write made meanwhile is dropped, so the real
                    // folder can come back (it isn't the link: that's away).
                    retry(&|| {
                        let _ = std::fs::remove_dir_all(at("sub"));
                        std::fs::rename(at("sub.real"), at("sub"))
                    });
                    swaps.fetch_add(1, Ordering::SeqCst);
                }
            })
        };
        let rel = Path::new("sub/settings.json");
        for _ in 0..400 {
            if let Err(error) = write(&base, rel, b"bot") {
                // Refused at the link, or the folder briefly not there.
                let gone = error.chain().any(|e| {
                    e.downcast_ref::<std::io::Error>()
                        .is_some_and(|io| io.kind() == std::io::ErrorKind::NotFound)
                });
                assert!(
                    gone || error.downcast_ref::<LinkRefused>().is_some(),
                    "{kind}: {error:#}"
                );
            }
        }
        stop.store(true, Ordering::SeqCst);
        swapper.join().unwrap();
        assert!(
            swaps.load(Ordering::SeqCst) > 0,
            "{kind}: the folder was swapped"
        );
        assert_eq!(
            std::fs::read_dir(&owner).unwrap().count(),
            0,
            "{kind}: nothing written through the link"
        );
    }
}

/// H-184 (CE-026 F2): a file in the bot's folder that is a hard link to a
/// file elsewhere is not read back, so its content isn't copied around.
#[test]
fn a_hard_link_to_a_file_elsewhere_is_not_read() {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().join("bot");
    std::fs::create_dir_all(&base).unwrap();
    let owner = dir.path().join("owner.json");
    std::fs::write(&owner, "owner").unwrap();
    std::fs::hard_link(&owner, base.join("mcp.json")).unwrap();
    assert_eq!(read(&base, Path::new("mcp.json")), None);
    // Written by the daemon, the name gets a file of its own again.
    write(&base, Path::new("mcp.json"), b"bot").unwrap();
    assert_eq!(read(&base, Path::new("mcp.json")).as_deref(), Some("bot"));
    assert_eq!(std::fs::read_to_string(&owner).unwrap(), "owner");
}

/// A file held open for a moment, as an antivirus scan holds one just
/// written, doesn't fail the next write over it: it waits the hold out
/// (H-188).
#[cfg(windows)]
#[test]
fn a_write_waits_out_a_brief_hold_on_the_file() {
    use std::os::windows::fs::OpenOptionsExt;
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().join("bot");
    std::fs::create_dir_all(&base).unwrap();
    let rel = Path::new("settings.json");
    write(&base, rel, b"old").unwrap();
    // FILE_SHARE_READ only: while it is open, nothing renames over it.
    let held = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(1)
        .open(base.join(rel))
        .unwrap();
    let release = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(150));
        drop(held);
    });
    write(&base, rel, b"new").unwrap();
    release.join().unwrap();
    assert_eq!(std::fs::read(base.join(rel)).unwrap(), b"new");
}

/// A write that waits out a hold re-vets the path before each retry: a bot
/// that forces the wait can't swap a folder for a junction meanwhile and
/// have the retry rename through it (H-188, Architect M1).
#[cfg(windows)]
#[test]
fn a_junction_swapped_in_during_the_wait_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().join("bot");
    let conf = base.join("conf");
    let outside = dir.path().join("owner");
    std::fs::create_dir_all(&outside).unwrap();
    // A folder at the name: renaming a file over it is "Access is denied",
    // so the first attempt fails as it does over a held file.
    std::fs::create_dir_all(conf.join("settings.json")).unwrap();
    let swap = {
        let (base, conf, outside) = (base.clone(), conf.clone(), outside.clone());
        move || {
            std::fs::rename(&conf, base.join("conf-real")).unwrap();
            junction(&outside, &conf);
        }
    };
    imp::BETWEEN_ATTEMPTS.with(|hook| hook.set(Some(Box::new(swap))));
    let error = write(&base, Path::new("conf/settings.json"), b"x").unwrap_err();
    let ran = imp::BETWEEN_ATTEMPTS.with(|hook| hook.take()).is_none();
    assert!(ran, "the write never retried");
    assert!(error.downcast_ref::<LinkRefused>().is_some(), "{error:#}");
    assert_eq!(std::fs::read_dir(&outside).unwrap().count(), 0);
}
