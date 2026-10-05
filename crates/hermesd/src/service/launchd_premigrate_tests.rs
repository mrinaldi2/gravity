//! The migration refused by its own preflight after the pre-rename agent
//! was disabled and booted out (the 0.15.0 install on a Mac): nothing to
//! roll back, and the agent is enabled and loaded again.
use super::*;

#[test]
fn a_migration_refused_after_the_bootout_reloads_the_pre_rename_agent() {
    let moving = Moving::new();
    let (paths, mut launchctl) = legacy_agent(&moving);
    let taken = moving.plan().to.join("taken");
    launchctl.on_bootout = Some(Box::new(move || std::fs::create_dir_all(&taken).unwrap()));
    let Outcome {
        result,
        loaded,
        disabled,
        calls,
    } = install_moving(&moving, &paths, launchctl);
    let error = format!("{:#}", result.unwrap_err());
    assert_eq!(
        error.matches("exists and is not empty").count(),
        1,
        "{error}"
    );
    assert!(error.contains("previous daemon was restored"), "{error}");
    assert!(!error.contains("undoing"), "{error}");
    assert_eq!(
        calls,
        [
            "version".to_string(),
            format!("disable {LEGACY_LABEL}"),
            format!("bootout {LEGACY}"),
            format!("enable {LEGACY_LABEL}"),
            format!("bootstrap {LEGACY}"),
        ]
    );
    assert_eq!(loaded, [LEGACY]);
    assert!(disabled.is_empty());
    assert!(paths.legacy_plist_path().is_file());
    assert!(!paths.plist_path().exists());
    std::fs::remove_dir_all(&moving.plan().to).unwrap();
    moving.assert_rolled_back();
}
