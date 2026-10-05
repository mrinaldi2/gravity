use serde_json::json;

use super::*;

#[test]
fn takes_one_release_and_skips_the_daemons_own_config() {
    let args = |a: &[&str]| -> Vec<String> { a.iter().map(|s| (*s).to_string()).collect() };
    assert_eq!(parse(&args(&["r1"])).unwrap(), ("r1".to_string(), false));
    assert_eq!(
        parse(&args(&["r1", "--dry-run", "--config", "/h/hermesd.toml"])).unwrap(),
        ("r1".to_string(), true)
    );
    assert!(parse(&args(&[])).is_err());
    assert!(parse(&args(&["r1", "r2"])).is_err());
    assert!(parse(&args(&["r1", "--force"])).is_err());
}

fn answer() -> Value {
    json!({
        "release_id": "r1", "machine": "mac",
        "builds": [
            {"platform": "desktop", "version": "0.16.2", "artifact": "/h/releases/r1/desktop/TheHermes.zip",
             "url": "https://mac.ts.net/releases/r1/desktop/TheHermes.zip", "sha256": "AB12"},
            {"platform": "daemon", "version": "0.16.2", "artifact": "", "url": null, "sha256": "cd34"},
            {"platform": "ios", "version": "0.16.2", "install_url": "itms-services://x", "sha256": "ef56"},
        ],
    })
}

#[test]
fn the_desktop_app_installs_where_there_is_one_and_the_daemon_elsewhere() {
    let all = builds(&answer());
    assert_eq!(all[0].sha256, "ab12", "compared in lower case");
    assert_eq!(all[1].artifact, None, "an empty artifact is none");
    let platforms = |macos, windows| -> Vec<String> {
        for_this_computer(&all, macos, windows)
            .into_iter()
            .map(|b| b.platform.clone())
            .collect()
    };
    assert_eq!(platforms(true, false), ["desktop"]);
    assert_eq!(platforms(false, true), ["desktop"]);
    assert_eq!(platforms(false, false), ["daemon"], "Linux has no app");
    let phone_only = builds(&json!({"builds": [{"platform": "ios", "sha256": "x"}]}));
    assert!(for_this_computer(&phone_only, true, false).is_empty());
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
