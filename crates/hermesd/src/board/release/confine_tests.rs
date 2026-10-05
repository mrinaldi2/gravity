use std::fs;
use std::os::unix::fs::symlink;

use super::*;

fn file(dir: &Path, name: &str, body: &str) -> PathBuf {
    fs::create_dir_all(dir).unwrap();
    let path = dir.join(name);
    fs::write(&path, body).unwrap();
    path
}

/// A daemon home at `<tmp>/user/.thehermes`, `~/Developer` as the trusted
/// path, and the served root at `dir` (the default when `None`).
fn config(tmp: &Path, dir: Option<PathBuf>) -> Config {
    let user = tmp.join("user");
    let home = user.join(".thehermes");
    fs::create_dir_all(home.join("secrets")).unwrap();
    fs::write(home.join("bus.sqlite"), "").unwrap();
    let mut cfg = Config {
        home,
        user_home: user,
        trusted_paths: vec!["~/Developer".to_string()],
        ..Config::default()
    };
    cfg.releases.dir = dir;
    cfg
}

#[test]
fn the_default_served_root_is_prepared_beside_its_staging_folder() {
    let tmp = tempfile::tempdir().unwrap();
    let cfg = config(tmp.path(), None);
    assert!(check_root(&cfg).is_ok());
    let dirs = prepare(&cfg).unwrap();
    let home = fs::canonicalize(&cfg.home).unwrap();
    assert_eq!(dirs.root, home.join("releases"));
    assert_eq!(dirs.staging, home.join(".releases-staging"));
    assert!(!dirs.staging.starts_with(&dirs.root));
}

/// M1: a served root that is a symlink, or is or holds the home, its
/// secrets or the bus database, is refused at load and at publish.
#[test]
fn a_served_root_that_would_expose_the_home_is_refused() {
    let tmp = tempfile::tempdir().unwrap();
    let base = config(tmp.path(), None);
    let user = base.user_home.clone();
    let home = base.home.clone();
    let legacy = user.join(".gravity");
    symlink(&home, &legacy).unwrap();
    let elsewhere = tmp.path().join("elsewhere");
    fs::create_dir_all(&elsewhere).unwrap();
    symlink(&home, elsewhere.join("served")).unwrap();
    for (dir, why) in [
        (home.clone(), "contains"),
        (user.clone(), "contains"),
        (tmp.path().to_path_buf(), "contains"),
        (PathBuf::from("/"), "contains"),
        (home.join("secrets"), "secrets"),
        (home.join("secrets/releases"), "secrets"),
        (legacy.clone(), "symlink"),
        (legacy.join("secrets"), "secrets"),
        (elsewhere.join("served"), "symlink"),
        (PathBuf::from("releases"), "absolute"),
    ] {
        let mut cfg = config(tmp.path(), Some(dir.clone()));
        let err = check_root(&cfg).unwrap_err().to_string();
        assert!(err.contains(why), "{}: {err}", dir.display());
        assert!(prepare(&cfg).is_err(), "{}", dir.display());
        check_at_load(&mut cfg);
        assert!(cfg.releases.refused.is_some(), "{}", dir.display());
    }
    // The default root, later swapped for a link to the home.
    let cfg = config(tmp.path(), None);
    symlink(&home, home.join("releases")).unwrap();
    assert!(check_root(&cfg)
        .unwrap_err()
        .to_string()
        .contains("symlink"));
    assert!(prepare(&cfg).is_err());
    // A refusal at load stands even if the disk is fixed meanwhile.
    fs::remove_file(home.join("releases")).unwrap();
    let mut cfg = config(tmp.path(), Some(home.clone()));
    check_at_load(&mut cfg);
    cfg.releases.dir = None;
    assert!(prepare(&cfg).is_err());
    // A folder of its own elsewhere is fine.
    let mut cfg = config(tmp.path(), Some(tmp.path().join("served")));
    check_at_load(&mut cfg);
    assert!(cfg.releases.refused.is_none());
    assert!(prepare(&cfg).is_ok());
}

/// M2: by default only the `<repo>-rel-*` release worktrees and the
/// project's artifacts, and only build types, even inside them.
#[test]
fn builds_come_only_from_release_worktrees_and_artifacts() {
    let tmp = tempfile::tempdir().unwrap();
    let cfg = config(tmp.path(), None);
    let dev = cfg.user_home.join("Developer");
    let artifacts = cfg.home.join("projects/p/artifacts");
    let rel = dev.join("gravity-rel-0.16.0/bundle/macos");
    let allowed = [
        file(&rel, "TheHermes.zip", "zip"),
        file(&dev.join("gravitiOS-rel-0.16.0/build"), "App.ipa", "ipa"),
        file(&dev.join("gravity-rel-0.16.0"), "Hermes.DMG", "dmg"),
        file(&artifacts.join("peers/win"), "hermes-setup.exe", "exe"),
        file(&artifacts, "hermes.msi", "msi"),
    ];
    for path in &allowed {
        let src = source(&cfg, &artifacts, path).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(src.path, fs::canonicalize(path).unwrap());
    }
    let checkout = file(&dev.join("gravity/target"), "app.zip", "owner's checkout");
    let worktree = file(&dev.join("gravity-wt-dev-x"), "app.zip", "a bot's worktree");
    let other = file(&dev.join("PhD"), "thesis.zip", "personal");
    for path in [&checkout, &worktree, &other] {
        let err = source(&cfg, &artifacts, path).unwrap_err().to_string();
        assert!(err.contains("outside the roots"), "{err}");
    }
    let env = file(&dev.join("gravity-rel-0.16.0"), ".env", "SECRET=1");
    let plist = file(&rel, "manifest.plist", "<plist/>");
    let page = file(&artifacts, "index.html", "<html/>");
    let bare = file(&rel, "zip", "no extension");
    for path in [&env, &plist, &page, &bare] {
        let err = source(&cfg, &artifacts, path).unwrap_err().to_string();
        assert!(err.contains("is not a build"), "{err}");
    }
    // A secret behind a build's name is judged by where it lands.
    let secret = file(&cfg.home.join("secrets"), "token", "t");
    symlink(&secret, rel.join("token.zip")).unwrap();
    assert!(source(&cfg, &artifacts, &rel.join("token.zip")).is_err());
    // A symlink posing as a release worktree is not one.
    symlink(dev.join("PhD"), dev.join("x-rel-1")).unwrap();
    assert!(source(&cfg, &artifacts, &dev.join("x-rel-1/thesis.zip")).is_err());
    assert!(source(&cfg, &artifacts, Path::new("gravity-rel-0.16.0/Hermes.DMG")).is_err());

    // The owner may widen the roots; the type allowlist still holds.
    let mut wide = cfg.clone();
    wide.releases.source_roots = vec!["~/Developer".to_string()];
    assert!(source(&wide, &artifacts, &checkout).is_ok());
    assert!(source(&wide, &artifacts, &env).is_err());
}

/// S1: a hard link to a file elsewhere passes `canonicalize`, so the open
/// handle is checked instead.
#[test]
fn a_hard_linked_source_is_refused() {
    let tmp = tempfile::tempdir().unwrap();
    let cfg = config(tmp.path(), None);
    let artifacts = cfg.home.join("projects/p/artifacts");
    fs::create_dir_all(&artifacts).unwrap();
    let secret = file(&cfg.home.join("secrets"), "client.token", "t");
    fs::hard_link(&secret, artifacts.join("token.zip")).unwrap();
    let err = source(&cfg, &artifacts, &artifacts.join("token.zip"))
        .expect_err("refused")
        .to_string();
    assert!(err.contains("hard link"), "{err}");
    // A directory or a dangling path is no build either.
    fs::create_dir_all(artifacts.join("dir.zip")).unwrap();
    assert!(source(&cfg, &artifacts, &artifacts.join("dir.zip")).is_err());
    assert!(source(&cfg, &artifacts, &artifacts.join("gone.zip")).is_err());
}
