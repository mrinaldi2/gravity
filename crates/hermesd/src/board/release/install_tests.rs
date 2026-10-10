use serde_json::json;
use sha2::{Digest, Sha256};

use super::signature::{plan, Check};
use super::*;

#[path = "install_handoff_tests.rs"]
mod handoff_tests;

#[test]
fn takes_one_release_and_passes_the_daemons_own_config_on() {
    let args = |a: &[&str]| -> Vec<String> { a.iter().map(|s| (*s).to_string()).collect() };
    let plain = parse(&args(&["r1"])).unwrap();
    assert_eq!(
        (plain.release.as_str(), plain.dry_run, plain.status),
        ("r1", false, false)
    );
    let dry = parse(&args(&["r1", "--dry-run", "--config", "/h/hermesd.toml"])).unwrap();
    assert!(dry.dry_run);
    assert_eq!(dry.config.as_deref(), Some("/h/hermesd.toml"));
    assert!(parse(&args(&["r1", "--status"])).unwrap().status);
    assert!(parse(&args(&["r1", "--status", "--dry-run"])).is_err());
    assert!(parse(&args(&[])).is_err());
    assert!(parse(&args(&["r1", "r2"])).is_err());
    assert!(parse(&args(&["r1", "--force"])).is_err());
}

#[test]
fn the_build_waits_in_a_fresh_private_stage() {
    let dir = tempfile::tempdir().expect("dir");
    let source = dir.path().join("TheHermes.zip");
    std::fs::write(&source, b"the build").expect("write");
    let build = Build {
        platform: "desktop-mac".into(),
        version: "0.16.3".into(),
        artifact: Some(source.display().to_string()),
        url: None,
        install_url: None,
        sha256: hex::encode(Sha256::digest(b"the build")),
    };
    let one = stage::Stage::new("r1/../x").expect("stage");
    let two = stage::Stage::new("r1").expect("stage");
    assert_ne!(one.dir, two.dir, "never reused");
    assert!(one.dir.starts_with(std::env::temp_dir()));
    assert!(!one.dir.to_string_lossy().contains(".."));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&one.dir)
            .expect("meta")
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o700, "only this user");
    }
    let staged = one.fetch(&build).expect("fetch");
    assert!(
        staged.starts_with(&one.dir),
        "checked where it is installed from"
    );
    verify(&staged, &build.sha256).expect("matches");
    one.remove();
    two.remove();
    assert!(!one.dir.exists());
}

#[test]
fn a_build_is_checked_against_the_compiled_identity_or_says_why_not() {
    let team = "ABCDE12345";
    let skipped = |c: Check| matches!(c, Check::Skip(why) if !why.is_empty());
    assert!(
        skipped(plan(Kind::AppZip, true, team, None, true)),
        "dev build"
    );
    assert!(
        skipped(plan(Kind::AppZip, false, "0000000000", None, true)),
        "no Team ID"
    );
    assert!(
        matches!(plan(Kind::AppDmg, false, team, None, true), Check::MacApp(r) if r.contains("identifier"))
    );
    assert!(matches!(
        plan(Kind::Daemon, false, team, None, true),
        Check::MacBinary(r) if r.contains(&format!("subject.OU] = \"{team}\""))
    ));
    assert_eq!(
        plan(Kind::WindowsSetup, false, team, Some("CN=Owner"), false),
        Check::Windows("CN=Owner".into())
    );
    assert!(
        skipped(plan(Kind::WindowsSetup, false, team, None, false)),
        "unsigned Windows"
    );
    assert!(
        skipped(plan(Kind::Daemon, false, team, None, false)),
        "Linux"
    );
}

#[cfg(target_os = "macos")]
#[test]
fn codesign_holds_a_binary_to_its_requirement() {
    let ls = std::path::Path::new("/bin/ls");
    signature::codesign(ls, "anchor apple").expect("Apple signs ls");
    let err = signature::codesign(
        ls,
        "anchor apple generic and certificate leaf[subject.OU] = \"ABCDE12345\"",
    )
    .expect_err("not the owner's team");
    assert!(
        err.to_string()
            .contains("isn't signed as the owner's build"),
        "{err}"
    );
    let skipped = signature::run(&Check::Skip("dev build".into()), ls).expect("skip");
    assert_eq!(skipped, "signature check skipped: dev build");
}

fn answer() -> Value {
    // The platforms `publish` gives them (`serve::platform_for`).
    json!({
        "release_id": "r1", "machine": "mac",
        "builds": [
            {"platform": "desktop-mac", "version": "0.16.2", "artifact": "/h/releases/r1/desktop-mac/TheHermes.zip",
             "url": "https://mac.ts.net/releases/r1/desktop-mac/TheHermes.zip", "sha256": "AB12"},
            {"platform": "desktop-win", "version": "0.16.2", "artifact": "/h/releases/r1/desktop-win/setup.exe",
             "url": "https://mac.ts.net/releases/r1/desktop-win/setup.exe",
             "install_url": "https://mac.ts.net/releases/r1/desktop-win/setup.exe", "sha256": "9f9f"},
            {"platform": "daemon", "version": "0.16.2", "artifact": "", "url": null, "sha256": "cd34"},
            {"platform": "ios", "version": "0.16.2", "install_url": "itms-services://x", "sha256": "ef56"},
        ],
    })
}

fn platforms(all: &[Build], macos: bool, windows: bool) -> Vec<String> {
    for_this_computer(all, macos, windows)
        .into_iter()
        .map(|b| b.platform.clone())
        .collect()
}

#[test]
fn the_desktop_app_installs_where_there_is_one_and_the_daemon_elsewhere() {
    let all = builds(&answer());
    assert_eq!(all[0].sha256, "ab12", "compared in lower case");
    assert_eq!(all[2].artifact, None, "an empty artifact is none");
    assert_eq!(platforms(&all, true, false), ["desktop-mac"]);
    assert_eq!(platforms(&all, false, true), ["desktop-win"]);
    assert_eq!(
        platforms(&all, false, false),
        ["daemon"],
        "Linux has no app"
    );
    let phone_only = builds(&json!({"builds": [{"platform": "ios", "sha256": "x"}]}));
    assert!(for_this_computer(&phone_only, true, false).is_empty());
}

#[test]
fn each_computer_finds_its_own_published_desktop_build() {
    // H-253: only "desktop" matched, so a published release never installed.
    let mac_only = builds(&json!({"builds": [
        {"platform": "desktop-mac", "version": "0.18.0", "url": "https://h/r/desktop-mac/a.zip", "sha256": "1"},
        {"platform": "daemon", "version": "0.18.0", "sha256": "2"},
    ]}));
    assert_eq!(platforms(&mac_only, true, false), ["desktop-mac"]);
    assert_eq!(
        platforms(&mac_only, false, true),
        ["daemon"],
        "never the Mac's app on Windows"
    );
    let win_only = builds(&json!({"builds": [
        {"platform": "desktop-win", "version": "0.18.0", "url": "https://h/r/desktop-win/s.exe", "sha256": "3"},
    ]}));
    assert_eq!(platforms(&win_only, false, true), ["desktop-win"]);
    assert!(
        for_this_computer(&win_only, true, false).is_empty(),
        "never the Windows setup on a Mac"
    );
    let legacy = builds(&json!({"builds": [{"platform": "desktop", "sha256": "4"}]}));
    assert_eq!(platforms(&legacy, true, false), ["desktop"]);
    assert_eq!(platforms(&legacy, false, true), ["desktop"]);
}

#[test]
fn a_build_without_an_install_url_installs_from_its_url() {
    let all = builds(&answer());
    assert_eq!(
        all[0].install_url.as_deref(),
        Some("https://mac.ts.net/releases/r1/desktop-mac/TheHermes.zip"),
        "no install_url: the url"
    );
    assert_eq!(
        all[1].install_url.as_deref(),
        Some("https://mac.ts.net/releases/r1/desktop-win/setup.exe")
    );
    assert_eq!(all[2].install_url, None, "neither");
    assert_eq!(all[3].install_url.as_deref(), Some("itms-services://x"));
}

#[test]
fn only_the_approved_file_installs() {
    let dir = tempfile::tempdir().expect("dir");
    let file = dir.path().join("TheHermes.zip");
    std::fs::write(&file, b"the build").expect("write");
    let sha = hex::encode(Sha256::digest(b"the build"));
    verify(&file, &sha).expect("matches");
    let err = verify(&file, &"0".repeat(64)).expect_err("tampered");
    assert!(
        err.to_string().contains("doesn't match the release"),
        "{err}"
    );
    assert!(verify(&file, "").is_err(), "no frozen hash, no install");
}

#[test]
fn each_file_installs_its_own_way_on_its_own_system() {
    assert_eq!(
        kind_of("TheHermes-0.16.2-macos.zip", true, false).unwrap(),
        Kind::AppZip
    );
    assert_eq!(
        kind_of("The Hermes.dmg", true, false).unwrap(),
        Kind::AppDmg
    );
    assert_eq!(
        kind_of("The Hermes_0.16.2_x64-setup.exe", false, true).unwrap(),
        Kind::WindowsSetup
    );
    assert_eq!(
        kind_of("hermesd-aarch64-apple-darwin", true, false).unwrap(),
        Kind::Daemon
    );
    assert_eq!(kind_of("hermesd.exe", false, true).unwrap(), Kind::Daemon);
    assert!(
        kind_of("TheHermes.zip", false, true).is_err(),
        "a macOS zip on Windows"
    );
    assert!(kind_of("setup.exe", true, false).is_err());
    assert!(kind_of("notes.txt", true, false).is_err());
}
