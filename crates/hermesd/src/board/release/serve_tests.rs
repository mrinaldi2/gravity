use std::fs;
use std::os::unix::fs::symlink;

use super::super::confine::{ServedDirs, Source};
use super::*;

const BASE: &str = "https://mac.tail.ts.net/releases";

fn file(dir: &Path, name: &str, body: &str) -> PathBuf {
    let path = dir.join(name);
    fs::write(&path, body).unwrap();
    path
}

/// The served root and its staging folder under `tmp`.
fn dirs(tmp: &Path) -> ServedDirs {
    let dirs = ServedDirs {
        root: tmp.join("served"),
        staging: tmp.join(".served-staging"),
    };
    fs::create_dir_all(&dirs.root).unwrap();
    fs::create_dir_all(&dirs.staging).unwrap();
    dirs
}

fn open(path: &Path) -> Source {
    let real = fs::canonicalize(path).unwrap();
    let root = real.parent().unwrap().to_path_buf();
    Source::open(&real, &[root]).unwrap()
}

fn config(home: &Path, roots: &[&Path]) -> Config {
    let mut cfg = Config {
        home: home.to_path_buf(),
        user_home: home.join("user"),
        ..Config::default()
    };
    cfg.releases.source_roots = roots.iter().map(|r| r.display().to_string()).collect();
    cfg
}

#[test]
fn a_build_is_copied_hashed_and_never_replaced_by_another() {
    let tmp = tempfile::tempdir().unwrap();
    let dirs = dirs(tmp.path());
    let src = file(tmp.path(), "app.zip", "build one");
    let staged = stage(&dirs, BASE, "r1", "desktop-mac", open(&src)).unwrap();
    assert!(staged.fresh);
    assert_eq!(staged.url, format!("{BASE}/r1/desktop-mac/app.zip"));
    assert_eq!(staged.sha256, hex::encode(Sha256::digest(b"build one")));
    assert_eq!(fs::read_to_string(&staged.path).unwrap(), "build one");
    assert!(matches(&staged.path, &staged.sha256));

    // The same file again is a no-op; a different one is refused.
    assert!(
        !stage(&dirs, BASE, "r1", "desktop-mac", open(&src))
            .unwrap()
            .fresh
    );
    fs::write(&src, "build two").unwrap();
    let err = stage(&dirs, BASE, "r1", "desktop-mac", open(&src)).unwrap_err();
    assert!(err.to_string().contains("different content"), "{err}");
    assert_eq!(fs::read_to_string(&staged.path).unwrap(), "build one");
    assert!(!matches(
        &staged.path,
        &hex::encode(Sha256::digest(b"build two"))
    ));
    // No temporary file is left behind, in the served tree or beside it.
    let names: Vec<_> = fs::read_dir(staged.path.parent().unwrap())
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(names, ["app.zip"]);
    assert_eq!(fs::read_dir(&dirs.staging).unwrap().count(), 0);
}

#[test]
fn symlinks_in_the_served_directory_are_refused() {
    let tmp = tempfile::tempdir().unwrap();
    let dirs = dirs(tmp.path());
    let root = dirs.root.clone();
    let outside = tmp.path().join("outside");
    fs::create_dir_all(&outside).unwrap();
    let src = file(tmp.path(), "app.zip", "build");

    symlink(&outside, root.join("r1")).unwrap();
    let err = stage(&dirs, BASE, "r1", "desktop-mac", open(&src)).unwrap_err();
    assert!(err.to_string().contains("symlink"), "{err}");
    assert!(!outside.join("desktop-mac").exists());

    fs::create_dir_all(root.join("r2/desktop-mac")).unwrap();
    symlink(outside.join("x.zip"), root.join("r2/desktop-mac/app.zip")).unwrap();
    let err = stage(&dirs, BASE, "r2", "desktop-mac", open(&src)).unwrap_err();
    assert!(err.to_string().contains("not a regular file"), "{err}");
    assert!(!outside.join("x.zip").exists());

    for (id, platform) in [("..", "ios"), ("r3", "../ios"), (".hidden", "ios")] {
        assert!(
            stage(&dirs, BASE, id, platform, open(&src)).is_err(),
            "{id}/{platform}"
        );
    }
}

#[test]
fn an_ipa_gets_a_manifest_naming_it() {
    let tmp = tempfile::tempdir().unwrap();
    let dirs = dirs(tmp.path());
    let root = dirs.root.clone();
    let src = file(tmp.path(), "TheHermes.ipa", "ipa");
    let staged = stage(&dirs, BASE, "r1", "ios", open(&src)).unwrap();
    let url = write_manifest(&dirs, &staged, "com.example.hermes", "0.16.0", "A & B").unwrap();
    assert_eq!(url, format!("{BASE}/r1/ios/manifest.plist"));
    let body = fs::read_to_string(root.join("r1/ios/manifest.plist")).unwrap();
    assert!(body.contains(&format!("<string>{BASE}/r1/ios/TheHermes.ipa</string>")));
    assert!(body.contains("<string>com.example.hermes</string>"));
    assert!(body.contains("<string>A &amp; B</string>"));
    // Rewriting the same manifest is fine; a different one is refused.
    assert!(write_manifest(&dirs, &staged, "com.example.hermes", "0.16.0", "A & B").is_ok());
    assert!(write_manifest(&dirs, &staged, "com.example.hermes", "0.16.1", "A & B").is_err());
}

#[test]
fn platforms_and_the_base_url() {
    assert_eq!(platform_for("A.IPA"), Some("ios"));
    assert_eq!(platform_for("a.dmg"), Some("desktop-mac"));
    assert_eq!(platform_for("a-setup.exe"), Some("desktop-win"));
    assert_eq!(platform_for("hermesd"), None);
    let tmp = tempfile::tempdir().unwrap();
    let mut cfg = config(tmp.path(), &[]);
    assert!(base_url(&cfg).is_err());
    cfg.releases.base_url = Some("http://insecure".into());
    assert!(base_url(&cfg).is_err());
    cfg.releases.base_url = Some(format!("{BASE}/"));
    assert_eq!(base_url(&cfg).unwrap(), BASE);
    assert_eq!(served_root(&cfg), tmp.path().join("releases"));
}
