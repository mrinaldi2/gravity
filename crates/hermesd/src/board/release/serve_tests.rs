use std::fs;
use std::os::unix::fs::symlink;

use super::*;

const BASE: &str = "https://mac.tail.ts.net/releases";

fn file(dir: &Path, name: &str, body: &str) -> PathBuf {
    let path = dir.join(name);
    fs::write(&path, body).unwrap();
    path
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
    let root = tmp.path().join("served");
    let src = file(tmp.path(), "app.zip", "build one");
    let staged = stage(&root, BASE, "r1", "desktop-mac", &src).unwrap();
    assert!(staged.fresh);
    assert_eq!(staged.url, format!("{BASE}/r1/desktop-mac/app.zip"));
    assert_eq!(staged.sha256, hex::encode(Sha256::digest(b"build one")));
    assert_eq!(fs::read_to_string(&staged.path).unwrap(), "build one");
    assert!(matches(&staged.path, &staged.sha256));

    // The same file again is a no-op; a different one is refused.
    assert!(!stage(&root, BASE, "r1", "desktop-mac", &src).unwrap().fresh);
    fs::write(&src, "build two").unwrap();
    let err = stage(&root, BASE, "r1", "desktop-mac", &src).unwrap_err();
    assert!(err.to_string().contains("different content"), "{err}");
    assert_eq!(fs::read_to_string(&staged.path).unwrap(), "build one");
    assert!(!matches(
        &staged.path,
        &hex::encode(Sha256::digest(b"build two"))
    ));
    // No temporary file is left behind.
    let names: Vec<_> = fs::read_dir(staged.path.parent().unwrap())
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(names, ["app.zip"]);
}

#[test]
fn symlinks_in_the_served_directory_are_refused() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("served");
    let outside = tmp.path().join("outside");
    fs::create_dir_all(&outside).unwrap();
    fs::create_dir_all(&root).unwrap();
    let src = file(tmp.path(), "app.zip", "build");

    symlink(&outside, root.join("r1")).unwrap();
    let err = stage(&root, BASE, "r1", "desktop-mac", &src).unwrap_err();
    assert!(err.to_string().contains("symlink"), "{err}");
    assert!(!outside.join("desktop-mac").exists());

    fs::create_dir_all(root.join("r2/desktop-mac")).unwrap();
    symlink(outside.join("x.zip"), root.join("r2/desktop-mac/app.zip")).unwrap();
    let err = stage(&root, BASE, "r2", "desktop-mac", &src).unwrap_err();
    assert!(err.to_string().contains("not a regular file"), "{err}");
    assert!(!outside.join("x.zip").exists());

    for (id, platform) in [("..", "ios"), ("r3", "../ios"), (".hidden", "ios")] {
        assert!(
            stage(&root, BASE, id, platform, &src).is_err(),
            "{id}/{platform}"
        );
    }
}

#[test]
fn a_source_must_resolve_inside_an_allowed_root() {
    let tmp = tempfile::tempdir().unwrap();
    let allowed = tmp.path().join("rel");
    let artifacts = tmp.path().join("artifacts");
    let elsewhere = tmp.path().join("elsewhere");
    for d in [&allowed, &artifacts, &elsewhere] {
        fs::create_dir_all(d).unwrap();
    }
    let cfg = config(tmp.path(), &[&allowed]);
    let inside = file(&allowed, "app.zip", "ok");
    let shared = file(&artifacts, "setup.exe", "ok");
    let secret = file(&elsewhere, "secret.zip", "no");
    assert!(source(&cfg, &artifacts, &inside).is_ok());
    assert!(source(&cfg, &artifacts, &shared).is_ok());
    let err = source(&cfg, &artifacts, &secret).unwrap_err();
    assert!(err.to_string().contains("outside the roots"), "{err}");
    // A symlink inside a root that points out of it is resolved first.
    symlink(&secret, allowed.join("link.zip")).unwrap();
    assert!(source(&cfg, &artifacts, &allowed.join("link.zip")).is_err());
    assert!(source(&cfg, &artifacts, &allowed.join("../elsewhere/secret.zip")).is_err());
    assert!(source(&cfg, &artifacts, Path::new("rel/app.zip")).is_err());
}

#[test]
fn an_ipa_gets_a_manifest_naming_it() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("served");
    let src = file(tmp.path(), "TheHermes.ipa", "ipa");
    let staged = stage(&root, BASE, "r1", "ios", &src).unwrap();
    let url = write_manifest(&staged, "com.example.hermes", "0.16.0", "A & B").unwrap();
    assert_eq!(url, format!("{BASE}/r1/ios/manifest.plist"));
    let body = fs::read_to_string(root.join("r1/ios/manifest.plist")).unwrap();
    assert!(body.contains(&format!("<string>{BASE}/r1/ios/TheHermes.ipa</string>")));
    assert!(body.contains("<string>com.example.hermes</string>"));
    assert!(body.contains("<string>A &amp; B</string>"));
    // Rewriting the same manifest is fine; a different one is refused.
    assert!(write_manifest(&staged, "com.example.hermes", "0.16.0", "A & B").is_ok());
    assert!(write_manifest(&staged, "com.example.hermes", "0.16.1", "A & B").is_err());
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
