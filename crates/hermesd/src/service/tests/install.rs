use super::*;

#[test]
fn a_pre_rename_agent_is_migrated_and_removed_once_the_new_one_is_healthy() {
    let moving = Moving::new();
    let (paths, launchctl) = legacy_agent(&moving);
    let Outcome {
        result,
        loaded,
        disabled,
        calls,
    } = install_moving(&moving, &paths, launchctl);
    result.unwrap();
    assert_eq!(
        calls,
        [
            "version".to_string(),
            format!("disable {LEGACY_LABEL}"),
            format!("bootout {LEGACY}"),
            format!("bootstrap {CURRENT}"),
            "health".into(),
            format!("bootout {LEGACY}"),
        ]
    );
    assert_eq!(loaded, [CURRENT]);
    // Disabled for good: had the install died before removing its plist,
    // the next login would not have loaded it.
    assert_eq!(disabled, [LEGACY_LABEL]);
    assert!(!paths.legacy_plist_path().exists());
    let plist = std::fs::read_to_string(paths.plist_path()).unwrap();
    assert!(plist.contains(&format!("<string>{}</string>", paths.bin_path().display())));
    assert!(paths.config_path().is_file());
    assert!(!with_suffix(&paths.plist_path(), ".old").exists());
    moving.assert_moved();
}

/// Each step of the install failing, through launchd's host: the old agent
/// is enabled and loaded again from its plist, the new one is gone, and the
/// home and binary are back as they were.
#[test]
fn a_failure_at_each_step_reloads_the_pre_rename_agent() {
    let steps: [(&str, Option<String>, &str); 8] = [
        ("stage", None, "copying daemon binary"),
        ("verify", Some("version".into()), "nothing was stopped"),
        (
            "disable",
            Some(format!("disable {LEGACY_LABEL}")),
            "it was left running",
        ),
        (
            "stop",
            Some(format!("bootout {LEGACY}")),
            "it was left running",
        ),
        ("migrate", None, "injected crash at Database"),
        ("swap", None, "bin/hermesd.old"),
        (
            "register",
            Some(format!("bootstrap {CURRENT}")),
            "at bootstrap",
        ),
        (
            "health",
            Some("health".into()),
            "injected failure at health",
        ),
    ];
    for (step, fail, expected) in steps {
        let mut moving = Moving::new();
        let (paths, mut launchctl) = legacy_agent(&moving);
        launchctl.fail = fail;
        match step {
            "stage" => moving.source = moving.plan().user_home.join("absent"),
            "migrate" => moving.crash_migration_at("Database"),
            // A directory where the old binary is kept blocks the swap.
            "swap" => {
                std::fs::create_dir_all(moving.plan().from.join("bin/hermesd.old/stuck")).unwrap()
            }
            _ => {}
        }
        let Outcome {
            result,
            loaded,
            disabled,
            calls,
        } = install_moving(&moving, &paths, launchctl);
        let error = format!("{:#}", result.expect_err(step));
        assert!(error.contains(expected), "{step}: {error}");
        if !matches!(step, "stage" | "verify" | "disable" | "stop") {
            assert!(
                error.contains("previous daemon was restored"),
                "{step}: {error}"
            );
        }
        if step == "swap" {
            std::fs::remove_dir_all(moving.plan().from.join("bin/hermesd.old")).unwrap();
        }
        moving.assert_rolled_back();
        assert_eq!(loaded, [LEGACY], "{step}: {error}\n{calls:?}");
        assert!(disabled.is_empty(), "{step}: still disabled {calls:?}");
        assert!(paths.legacy_plist_path().is_file(), "{step}");
        assert!(!paths.plist_path().exists(), "{step}: new plist left");
        let stopped = calls.contains(&format!("bootout {LEGACY}"));
        assert_eq!(
            stopped,
            !matches!(step, "stage" | "verify" | "disable"),
            "{step}: {calls:?}"
        );
    }
}

/// 0.15.0 → 0.15.1: the same label before and after, no migration.
#[test]
fn a_failed_same_label_upgrade_reloads_the_previous_plist_and_binary() {
    let tmp = tempfile::tempdir().unwrap();
    let paths = ServicePaths::new(tmp.path().join("home"), tmp.path().join("user"));
    std::fs::create_dir_all(paths.home.join("bin")).unwrap();
    std::fs::write(paths.bin_path(), "old").unwrap();
    std::fs::create_dir_all(&paths.launch_agents).unwrap();
    std::fs::write(paths.plist_path(), "previous plist").unwrap();
    let source = tmp.path().join("bundled");
    std::fs::write(&source, "new").unwrap();
    let launchctl = FakeLaunchctl {
        fail: Some("health".into()),
        ..Default::default()
    };
    launchctl.loaded.borrow_mut().insert(CURRENT.into());
    let host = launchd_with(&paths, paths.home.clone(), launchctl);
    install_with(&source, &paths, None, &host).unwrap_err();
    assert_eq!(std::fs::read_to_string(paths.bin_path()).unwrap(), "old");
    assert_eq!(
        std::fs::read_to_string(paths.plist_path()).unwrap(),
        "previous plist"
    );
    assert_eq!(host.launchctl.loaded(), [CURRENT]);
    assert!(!with_suffix(&paths.bin_path(), ".old").exists());
}

#[test]
fn restart_reinstalls_a_missing_binary_without_migrating() {
    let tmp = tempfile::tempdir().unwrap();
    let paths = ServicePaths::new(tmp.path().join("home"), tmp.path().join("user"));
    std::fs::create_dir_all(&paths.launch_agents).unwrap();
    std::fs::write(paths.plist_path(), "plist").unwrap();
    let bundled = tmp.path().join("bundled");
    std::fs::write(&bundled, "new").unwrap();
    let host = launchd_with(&paths, paths.home.clone(), FakeLaunchctl::default());
    sequence::restart(&bundled, &layout(&paths, &paths.home), &host).unwrap();
    assert_eq!(std::fs::read_to_string(paths.bin_path()).unwrap(), "new");
    assert_eq!(host.launchctl.loaded(), [CURRENT]);
}

/// With `THEHERMES_HOME` set, the pre-rename agent runs the user's main
/// default home, which nothing migrates: the install leaves it running and
/// keeps its plist.
#[test]
fn an_overridden_home_never_takes_over_the_pre_rename_agent() {
    let tmp = tempfile::tempdir().unwrap();
    let paths = ServicePaths::new(tmp.path().join("chosen"), tmp.path().join("user"))
        .with_home_overridden(true);
    std::fs::create_dir_all(paths.user_home.join(crate::brand::LEGACY_HOME_DIR_NAME)).unwrap();
    std::fs::create_dir_all(&paths.launch_agents).unwrap();
    std::fs::write(paths.legacy_plist_path(), "legacy agent").unwrap();
    let source = tmp.path().join("bundled");
    std::fs::write(&source, "new").unwrap();
    let launchctl = FakeLaunchctl::default();
    launchctl.loaded.borrow_mut().insert(LEGACY.into());
    let host = launchd_with(&paths, paths.home.clone(), launchctl);
    assert!(host.installed().is_empty());
    install_with(&source, &paths, None, &host).unwrap();
    let calls = host.launchctl.calls.borrow().clone();
    assert!(!calls.iter().any(|c| c.contains(LEGACY_LABEL)), "{calls:?}");
    assert_eq!(host.launchctl.loaded(), [CURRENT, LEGACY]);
    assert!(paths.legacy_plist_path().is_file());
}
