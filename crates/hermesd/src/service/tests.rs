use super::*;

fn paths(dir: &Path) -> ServicePaths {
    ServicePaths::new(dir.join("state"), dir.join("userhome"))
}

fn fake_binary(dir: &Path) -> PathBuf {
    let src = dir.join("hermesd-src");
    std::fs::write(&src, b"#!/bin/sh\n").unwrap();
    src
}

#[test]
fn plist_bakes_absolute_paths() {
    let rendered = render_plist(
        Path::new("/x/bin/hermesd"),
        Path::new("/x/logs"),
        Path::new("/Users/x"),
    );
    assert!(rendered.contains("<string>/x/bin/hermesd</string>"));
    assert!(rendered.contains("<string>/x/logs/gravityd.out.log</string>"));
    assert!(rendered.contains("<string>--negotiate-port</string>"));
    assert!(rendered.contains(LAUNCHD_LABEL));
    assert!(!rendered.contains('~'));
}

/// A launchd agent inherits no login-shell environment, so a `claude`
/// installed by Claude Code's own installer is only reachable if the plist
/// puts `~/.local/bin` on PATH.
#[test]
fn plist_path_covers_the_claude_code_installer_location() {
    let rendered = render_plist(
        Path::new("/x/bin/hermesd"),
        Path::new("/x/logs"),
        Path::new("/Users/x"),
    );
    assert!(rendered.contains("<string>/Users/x/.local/bin:/usr/local/bin:"));
}

#[test]
fn install_stages_binary_config_and_plist() {
    let tmp = tempfile::tempdir().unwrap();
    let p = paths(tmp.path());
    install(&fake_binary(tmp.path()), &p).unwrap();
    assert!(p.bin_path().exists());
    assert!(p.plist_path().exists());
    assert!(p.config_path().exists());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(p.bin_path())
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o755, 0o755);
    }
}

#[test]
fn install_keeps_an_existing_config() {
    let tmp = tempfile::tempdir().unwrap();
    let p = paths(tmp.path());
    std::fs::create_dir_all(p.config_path().parent().unwrap()).unwrap();
    std::fs::write(p.config_path(), "port = 9999\n").unwrap();
    install(&fake_binary(tmp.path()), &p).unwrap();
    assert_eq!(
        std::fs::read_to_string(p.config_path()).unwrap(),
        "port = 9999\n"
    );
}

#[test]
fn install_replaces_a_previous_binary() {
    let tmp = tempfile::tempdir().unwrap();
    let p = paths(tmp.path());
    install(&fake_binary(tmp.path()), &p).unwrap();
    let src = tmp.path().join("hermesd-src");
    std::fs::write(&src, b"new contents").unwrap();
    install(&src, &p).unwrap();
    assert_eq!(std::fs::read(p.bin_path()).unwrap(), b"new contents");
}

/// An install from before the rename left `bin/gravityd` behind and a
/// plist pointing at it; upgrading must replace both, keeping the label.
#[test]
fn install_replaces_a_pre_rename_binary_and_repoints_the_plist() {
    let tmp = tempfile::tempdir().unwrap();
    let p = paths(tmp.path());
    std::fs::create_dir_all(p.legacy_bin_path().parent().unwrap()).unwrap();
    std::fs::write(p.legacy_bin_path(), b"old").unwrap();
    install(&fake_binary(tmp.path()), &p).unwrap();
    assert!(!p.legacy_bin_path().exists());
    assert!(p.bin_path().exists());
    let plist = std::fs::read_to_string(p.plist_path()).unwrap();
    assert!(plist.contains(&format!("<string>{}</string>", p.bin_path().display())));
    assert!(!plist.contains("bin/gravityd"));
    assert!(plist.contains("<string>in.mikolajczuk.gravityd</string>"));
}

#[test]
fn install_from_the_installed_path_skips_the_copy() {
    let tmp = tempfile::tempdir().unwrap();
    let p = paths(tmp.path());
    install(&fake_binary(tmp.path()), &p).unwrap();
    // Reinstalling from the managed location itself must not fail.
    install(&p.bin_path(), &p).unwrap();
    assert!(p.bin_path().exists());
}
