use super::*;

fn daemon_test_home(name: &str) -> PathBuf {
    let home = std::env::temp_dir().join(format!(
        "hermes-desktop-daemon-{name}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(&home).expect("temporary home");
    home
}

#[test]
fn discovers_the_renamed_sidecar_next_to_the_app_binary() {
    let exe = Path::new("/Applications/The Hermes.app/Contents/MacOS/The Hermes");
    assert_eq!(
        sidecar_path_for_exe(exe).expect("sidecar path"),
        PathBuf::from(format!(
            "/Applications/The Hermes.app/Contents/MacOS/hermesd{}",
            std::env::consts::EXE_SUFFIX
        ))
    );
}

#[test]
fn recognizes_only_complete_managed_daemon_installs() {
    let root =
        std::env::temp_dir().join(format!("hermes-desktop-daemon-test-{}", std::process::id()));
    // A previous run that panicked mid-test leaves the plist behind, and
    // pids are reused.
    let _ = std::fs::remove_dir_all(&root);
    let home = root.join("gravity");
    let user_home = root.join("user");
    let bin = home.join(format!("bin/hermesd{}", std::env::consts::EXE_SUFFIX));
    let plist = managed_marker(&home, &user_home);
    std::fs::create_dir_all(bin.parent().expect("binary parent")).expect("binary directory");
    std::fs::write(&bin, b"daemon").expect("daemon binary");
    assert!(!managed_daemon_is_installed(&home, &user_home));

    std::fs::create_dir_all(plist.parent().expect("plist parent")).expect("plist directory");
    std::fs::write(&plist, b"plist").expect("launchd plist");
    assert!(managed_daemon_is_installed(&home, &user_home));

    std::fs::remove_dir_all(root).expect("test cleanup");
}

#[test]
fn recognizes_a_managed_install_from_before_the_rename() {
    let root = daemon_test_home("pre-rename");
    let home = root.join("gravity");
    let user_home = root.join("user");
    let legacy = home.join(format!("bin/gravityd{}", std::env::consts::EXE_SUFFIX));
    let plist = managed_marker(&home, &user_home);
    std::fs::create_dir_all(legacy.parent().expect("binary parent")).expect("binary directory");
    std::fs::create_dir_all(plist.parent().expect("plist parent")).expect("plist directory");
    std::fs::write(&legacy, b"daemon").expect("legacy daemon binary");
    std::fs::write(&plist, b"plist").expect("launchd plist");

    assert!(managed_daemon_is_installed(&home, &user_home));
    std::fs::remove_dir_all(root).expect("test cleanup");
}

#[test]
fn a_service_without_its_binary_needs_repair() {
    let root = daemon_test_home("repair");
    let home = root.join("gravity");
    let user_home = root.join("user");
    let marker = managed_marker(&home, &user_home);
    assert!(!managed_daemon_needs_repair(&home, &user_home));
    std::fs::create_dir_all(marker.parent().expect("marker parent")).expect("marker directory");
    std::fs::write(&marker, b"task").expect("service marker");
    assert!(managed_daemon_needs_repair(&home, &user_home));

    let legacy = home.join(format!("bin/gravityd{}", std::env::consts::EXE_SUFFIX));
    std::fs::create_dir_all(legacy.parent().expect("binary parent")).expect("binary directory");
    std::fs::write(&legacy, b"daemon").expect("legacy daemon binary");
    assert!(!managed_daemon_needs_repair(&home, &user_home));
    std::fs::remove_dir_all(root).expect("test cleanup");
}

#[test]
fn rejects_known_daemon_downgrades() {
    let bundled = Version::parse(env!("CARGO_PKG_VERSION")).expect("bundled version");
    let newer = Version::new(bundled.major + 1, 0, 0).to_string();

    assert!(reject_daemon_downgrade(None).is_ok());
    assert!(reject_daemon_downgrade(Some(&bundled.to_string())).is_ok());
    assert!(reject_daemon_downgrade(Some(&newer)).is_err());
    assert!(reject_daemon_downgrade(Some("dev")).is_err());
}

#[test]
fn runtime_port_takes_precedence_over_the_configured_port() {
    let home = daemon_test_home("runtime-port");
    std::fs::write(home.join("gravityd.toml"), "port = 49777\n").expect("configured port");
    std::fs::write(home.join("gravityd.port"), "50123\n").expect("runtime port");

    assert_eq!(daemon_port_from_home(&home), 50_123);
    std::fs::remove_dir_all(home).expect("test cleanup");
}

/// The dev script runs a workspace-private daemon as a plain child process
/// on its own port, so the restart control must stay hidden there even
/// when a managed install exists for the same home.
#[test]
fn only_the_managed_daemon_on_its_own_port_counts_as_managed() {
    let root = daemon_test_home("is-managed");
    let home = root.join("gravity");
    let user_home = root.join("user");
    let bin = home.join(format!("bin/hermesd{}", std::env::consts::EXE_SUFFIX));
    let plist = managed_marker(&home, &user_home);
    std::fs::create_dir_all(bin.parent().expect("binary parent")).expect("binary directory");
    std::fs::create_dir_all(plist.parent().expect("plist parent")).expect("plist directory");
    std::fs::write(&bin, b"daemon").expect("daemon binary");
    std::fs::write(&plist, b"plist").expect("launchd plist");
    std::fs::write(home.join("gravityd.port"), "49777\n").expect("runtime port");

    assert!(managed_daemon_is_installed(&home, &user_home));
    assert_eq!(daemon_port_from_home(&home), 49_777);
    // The daemon the dev script starts answers elsewhere.
    assert_ne!(daemon_port_from_home(&home), 55_041);

    std::fs::remove_dir_all(root).expect("test cleanup");
}

#[test]
fn invalid_runtime_port_falls_back_to_the_configured_port() {
    let home = daemon_test_home("invalid-runtime-port");
    std::fs::write(home.join("gravityd.toml"), "port = 7777\n").expect("configured port");
    std::fs::write(home.join("gravityd.port"), "stale\n").expect("runtime port");

    assert_eq!(daemon_port_from_home(&home), 7777);
    std::fs::remove_dir_all(home).expect("test cleanup");
}
