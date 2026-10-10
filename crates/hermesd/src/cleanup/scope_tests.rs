use std::path::Path;

use super::Roots;

fn roots(dir: &Path) -> Roots {
    let workspace = dir.join("home/projects/p/bots/dev/workspace");
    std::fs::create_dir_all(&workspace).unwrap();
    std::fs::create_dir_all(dir.join("dev")).unwrap();
    Roots::new(
        Some(&workspace),
        &[dir.join("dev")],
        "Desktop Dev",
        &dir.join("home"),
    )
}

#[test]
fn only_the_bots_own_folders_are_in_scope() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = crate::safe_git::canonical(tmp.path()).unwrap();
    std::fs::create_dir_all(dir.join("dev/gravity-wt-desktopdev-a")).unwrap();
    let r = roots(&dir);
    assert!(r.check(&dir.join("dev/gravity-wt-desktopdev-a")).is_ok());
    assert!(r.check(&dir.join("dev/gravity-wt-desktopdev")).is_ok());
    assert!(r
        .check(&dir.join("home/projects/p/bots/dev/workspace/scratch/x"))
        .is_ok());
    for refused in [
        dir.join("dev/gravity-wt-architect-a"),
        dir.join("dev/gravity"),
        dir.join("dev"),
        dir.join("home/projects/p/bots/dev/workspace"),
        dir.join("home/run"),
        dir.join("elsewhere/gravity-wt-desktopdev-a"),
        dir.join("dev/gravity-wt-desktopdev-a/../gravity"),
    ] {
        assert!(r.check(&refused).is_err(), "{}", refused.display());
    }
}

#[cfg(unix)]
#[test]
fn a_link_in_any_part_is_refused() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = crate::safe_git::canonical(tmp.path()).unwrap();
    let r = roots(&dir);
    std::fs::create_dir_all(dir.join("outside/tree")).unwrap();
    // The worktree folder itself is a link.
    std::os::unix::fs::symlink(dir.join("outside"), dir.join("dev/gravity-wt-desktopdev-l"))
        .unwrap();
    let err = r
        .check(&dir.join("dev/gravity-wt-desktopdev-l/tree"))
        .unwrap_err();
    assert!(err.contains("link or junction"), "{err}");
    // A part inside the workspace is a link.
    let ws = dir.join("home/projects/p/bots/dev/workspace");
    std::os::unix::fs::symlink(dir.join("outside"), ws.join("repo")).unwrap();
    assert!(r.check(&ws.join("repo/tree")).is_err());
    assert!(r.check(&ws.join("repo")).is_err());
}
