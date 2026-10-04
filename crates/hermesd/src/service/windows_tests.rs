use super::*;

#[test]
fn stopping_an_absent_or_reused_pid_is_a_no_op() {
    let root = tempfile::tempdir().expect("temporary home");
    let executables = [
        root.path().join("hermesd.exe"),
        root.path().join("gravityd.exe"),
    ];
    stop_daemon(i32::MAX as u32, &executables).expect("absent process");
    stop_daemon(std::process::id(), &executables).expect("unrelated process is preserved");
}

#[test]
fn uninstall_without_an_installed_daemon_is_a_no_op() {
    let root = tempfile::tempdir().expect("temporary home");
    for home in [root.path().to_path_buf(), root.path().join("absent")] {
        let paths = ServicePaths::new(home.clone(), root.path().to_path_buf());
        uninstall(&paths).expect("already uninstalled");
        assert!(!paths.plist_path().exists());
        assert!(!paths.bin_path().exists());
    }
}

#[test]
fn task_is_scoped_to_current_user_and_escapes_paths() {
    let paths = ServicePaths::new(
        PathBuf::from(r"C:\Users\Test & User\.gravity"),
        PathBuf::new(),
    );
    let task = render_task(&paths, "S-1-5-21-123");
    assert!(task.contains("InteractiveToken"));
    assert!(task.contains("LeastPrivilege"));
    assert!(task.contains("Test &amp; User"));
    assert!(task.contains("-WindowStyle Hidden"));
    assert!(task.contains("<ExecutionTimeLimit>PT0S</ExecutionTimeLimit>"));
    assert!(task.contains("<UserId>S-1-5-21-123</UserId>"));
}

/// The old and new tasks never share a name, so deleting the old one can
/// not touch the new one.
#[test]
fn task_names_carry_the_label_user_and_home() {
    let new = task_name_for(SERVICE_LABEL, "S-1-5-21-1", r"c:\users\u\.thehermes");
    let old = task_name_for(
        crate::brand::LEGACY_WINDOWS_TASK,
        "S-1-5-21-1",
        r"c:\users\u\.gravity",
    );
    assert!(new.starts_with("The Hermes-S-1-5-21-1-"));
    assert!(old.starts_with("Gravity-S-1-5-21-1-"));
    assert_eq!(new.len(), "The Hermes-S-1-5-21-1-".len() + 12);
}

#[test]
fn legacy_homes_cover_an_explicit_home_and_the_default() {
    let paths = ServicePaths::new(PathBuf::from(r"C:\h"), PathBuf::from(r"C:\Users\u"));
    assert_eq!(
        paths.legacy_homes(),
        vec![
            PathBuf::from(r"C:\h"),
            PathBuf::from(r"C:\Users\u").join(".gravity")
        ]
    );
    let default = ServicePaths::new(
        PathBuf::from(r"C:\Users\u").join(".gravity"),
        PathBuf::from(r"C:\Users\u"),
    );
    assert_eq!(default.legacy_homes().len(), 1);
}

#[test]
fn launcher_quotes_apostrophes() {
    assert_eq!(
        quote(Path::new("C:/O'Brien/gravity.exe")),
        r"'C:\O''Brien\gravity.exe'"
    );
}
