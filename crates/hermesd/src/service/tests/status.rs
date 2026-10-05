use super::*;

/// The incident shape: an install that rolled back left only the pre-rename
/// agent (disabled, so nothing runs it) while a daemon started by hand
/// answers `/health`.
#[test]
fn the_status_report_names_a_lone_legacy_agent_and_an_unmanaged_daemon() {
    let tmp = tempfile::tempdir().unwrap();
    let paths = ServicePaths::new(tmp.path().join("home"), tmp.path().join("user"))
        .with_home_overridden(false);
    let legacy = paths.legacy_plist_path();
    std::fs::create_dir_all(legacy.parent().unwrap()).unwrap();
    std::fs::write(&legacy, "plist").unwrap();
    let home = crate::service_report::HomeState {
        migration_pending: false,
        migrated: true,
    };
    let report = report_with(
        &paths,
        49777,
        Some("0.14.2".into()),
        home,
        &FakeLaunchctl::default(),
    );
    assert_eq!(
        serde_json::to_value(&report).unwrap(),
        serde_json::json!({
            "binary": false,
            "service": false,
            "service_running": false,
            "legacy_service": true,
            "legacy_running": false,
            "migration_pending": false,
            "migrated": true,
            "port": 49777,
            "version": "0.14.2",
            // H-114: how this build knows the owner's app.
            "identity": crate::bus_auth::app_identity::identity_line(),
        })
    );
}
