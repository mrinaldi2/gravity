//! The trust write and Claude Code's config lock.

use super::*;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

#[cfg(windows)]
#[test]
fn windows_keys_match_claudes_forward_slashes() {
    assert_eq!(
        project_key(Path::new(r"C:\bots\one")).unwrap(),
        "C:/bots/one"
    );
    assert_eq!(
        project_key(Path::new(r"\\?\C:\bots\one")).unwrap(),
        "C:/bots/one"
    );
    assert_eq!(
        project_key(Path::new(r"\\?\UNC\server\share\one")).unwrap(),
        "//server/share/one"
    );
}

#[cfg(windows)]
#[test]
fn trust_updates_the_entry_claude_reads_without_forking_project_state() {
    let tmp = tempfile::tempdir().unwrap();
    let workspace = tmp.path().join("workspace");
    fs::create_dir(&workspace).unwrap();
    // Canonical, so an 8.3 temp dir (`RUNNER~1` on CI) expands like the daemon's.
    let canonical = workspace.canonicalize().unwrap();
    let canonical = canonical.to_str().unwrap();
    let node_key = canonical
        .strip_prefix(r"\\?\")
        .unwrap_or(canonical)
        .replace('\\', "/");
    let config_path = tmp.path().join(".claude.json");
    fs::write(
        &config_path,
        serde_json::to_vec(&serde_json::json!({"projects": {
            node_key.clone(): {"hasTrustDialogAccepted": false, "otherSetting": "preserved"}
        }}))
        .unwrap(),
    )
    .unwrap();
    trust_workspace(tmp.path(), &workspace).unwrap();
    let config = read_claude_config(&config_path).unwrap();
    assert!(is_trusted(&config, &node_key));
    assert_eq!(config["projects"][&node_key]["otherSetting"], "preserved");
    assert_eq!(config["projects"].as_object().unwrap().len(), 1);
}

/// A temp home with a workspace and a `.claude.json` holding `config`.
fn trust_fixture(config: &str) -> (tempfile::TempDir, PathBuf, PathBuf) {
    let tmp = tempfile::tempdir().expect("tmp");
    let workspace = tmp.path().join("workspace");
    fs::create_dir(&workspace).expect("workspace");
    let config_path = tmp.path().join(".claude.json");
    fs::write(&config_path, config).expect("config");
    let workspace = workspace.to_path_buf();
    (tmp, workspace, config_path)
}

/// The lock path exactly as `ClaudeConfigLock` derives it.
fn config_lock_path(config_path: &Path) -> PathBuf {
    PathBuf::from(format!(
        "{}.lock",
        config_path
            .canonicalize()
            .expect("canonical config")
            .display()
    ))
}

#[test]
fn trusting_workspace_preserves_existing_claude_config() {
    let tmp = tempfile::tempdir().expect("tmp");
    let workspace = tmp.path().join("workspace");
    fs::create_dir(&workspace).expect("workspace");
    let canonical_workspace = workspace.canonicalize().expect("canonical workspace");
    fs::write(
        tmp.path().join(".claude.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "theme": "dark",
            "projects": {
                project_key(&canonical_workspace).expect("project key"): {
                    "allowedTools": ["Read"],
                    "hasTrustDialogAccepted": false
                },
                "/other/workspace": { "hasTrustDialogAccepted": false }
            }
        }))
        .expect("json"),
    )
    .expect("config");

    trust_workspace(tmp.path(), &workspace).expect("trust");

    let config: serde_json::Value =
        serde_json::from_slice(&fs::read(tmp.path().join(".claude.json")).expect("read config"))
            .expect("parse config");
    assert_eq!(config["theme"], "dark");
    assert_eq!(
        config["projects"][project_key(&canonical_workspace).expect("project key")]["allowedTools"]
            [0],
        "Read"
    );
    assert_eq!(
        config["projects"][project_key(&canonical_workspace).expect("project key")]
            ["hasTrustDialogAccepted"],
        true
    );
    assert_eq!(
        config["projects"]["/other/workspace"]["hasTrustDialogAccepted"],
        false
    );
}

/// The lock is Claude Code's, and a live session holding it is a session
/// mid-write: the daemon must not replace the file underneath it.
#[test]
fn a_held_config_lock_defers_the_trust_write() {
    let (tmp, workspace, config_path) = trust_fixture(r#"{"theme":"dark"}"#);
    let lock = config_lock_path(&config_path);
    fs::create_dir(&lock).expect("hold lock");

    let result = trust_workspace(tmp.path(), &workspace);

    assert!(result.is_err(), "a held lock must defer the write");
    assert_eq!(
        fs::read_to_string(&config_path).expect("read config"),
        r#"{"theme":"dark"}"#,
        "the holder's file must be left alone"
    );
}

/// A held lock directory last touched an hour ago, as a dead holder leaves it.
fn stale_lock(config_path: &Path) -> PathBuf {
    let lock = config_lock_path(config_path);
    fs::create_dir(&lock).expect("hold lock");
    let mut options = fs::OpenOptions::new();
    options.write(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.custom_flags(0x02000000); // FILE_FLAG_BACKUP_SEMANTICS
    }
    #[cfg(unix)]
    let held = fs::File::open(&lock).expect("open lock");
    #[cfg(windows)]
    let held = options.open(&lock).expect("open lock");
    held.set_times(
        fs::FileTimes::new().set_modified(SystemTime::now() - Duration::from_secs(3600)),
    )
    .expect("age the lock");
    lock
}

fn assert_trusted(config_path: &Path, workspace: &Path) {
    let config: serde_json::Value =
        serde_json::from_slice(&fs::read(config_path).expect("read config")).expect("parse");
    assert_eq!(config["theme"], "dark");
    assert!(is_trusted(
        &config,
        &project_key(&workspace.canonicalize().expect("canonical")).expect("project key")
    ));
}

/// A lock left behind by a process that died mid-write would otherwise
/// keep every later start out for good.
#[test]
fn a_stale_config_lock_is_broken() {
    let (tmp, workspace, config_path) = trust_fixture(r#"{"theme":"dark"}"#);
    stale_lock(&config_path);

    trust_workspace(tmp.path(), &workspace).expect("trust");

    assert_trusted(&config_path, &workspace);
}

/// H-183: on a loaded machine, checking and removing a stale lock can take
/// the whole wait. The lock it just broke is still taken, rather than the
/// write giving up as "stayed locked" (what win-pc saw at 0.17.2).
#[test]
fn a_broken_stale_lock_is_taken_even_when_breaking_it_used_up_the_wait() {
    let (_tmp, _workspace, config_path) = trust_fixture(r#"{"theme":"dark"}"#);
    let lock = stale_lock(&config_path);

    let taken = ClaudeConfigLock::acquire(&config_path, Duration::ZERO).expect("taken");

    assert!(lock.is_dir(), "the lock is held again");
    drop(taken);
    assert!(!lock.exists(), "and released");
}

/// H-183: on Windows, a stale lock removed while another process still has
/// it open (an antivirus scan, say) stays delete-pending until that handle
/// closes, and creating it meanwhile is refused as access denied. That is a
/// lock still held, so the write waits for it rather than failing.
#[cfg(windows)]
#[test]
fn a_stale_lock_still_open_elsewhere_is_waited_for() {
    use std::os::windows::fs::OpenOptionsExt;
    let (tmp, workspace, config_path) = trust_fixture(r#"{"theme":"dark"}"#);
    let lock = stale_lock(&config_path);
    // Shares delete like a scanner's handle, so removing the folder succeeds
    // but leaves it in place until this closes.
    let scanner = fs::OpenOptions::new()
        .read(true)
        .custom_flags(0x02000000) // FILE_FLAG_BACKUP_SEMANTICS
        .open(&lock)
        .expect("scanner handle");
    let closer = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(150));
        drop(scanner);
    });

    trust_workspace(tmp.path(), &workspace).expect("trust");

    closer.join().expect("closer");
    assert_trusted(&config_path, &workspace);
}

/// Every bot start re-asserts trust, and almost every one of those finds
/// it already set. Taking the shared lock for that no-op would fail a
/// concurrent session's save for nothing.
#[test]
fn an_already_trusted_workspace_never_takes_the_lock() {
    let tmp = tempfile::tempdir().expect("tmp");
    let workspace = tmp.path().join("workspace");
    fs::create_dir(&workspace).expect("workspace");
    let canonical = workspace.canonicalize().expect("canonical");
    let config_path = tmp.path().join(".claude.json");
    fs::write(
        &config_path,
        serde_json::to_vec_pretty(&serde_json::json!({
            "projects": {
                project_key(&canonical).expect("project key"): { "hasTrustDialogAccepted": true }
            }
        }))
        .expect("json"),
    )
    .expect("config");
    fs::create_dir(config_lock_path(&config_path)).expect("hold lock");

    trust_workspace(tmp.path(), &workspace).expect("trust");
}

#[test]
fn trusting_workspace_creates_missing_claude_config_privately() {
    let tmp = tempfile::tempdir().expect("tmp");
    let workspace = tmp.path().join("workspace");
    fs::create_dir(&workspace).expect("workspace");
    let canonical_workspace = workspace.canonicalize().expect("canonical workspace");

    trust_workspace(tmp.path(), &workspace).expect("trust");

    let config_path = tmp.path().join(".claude.json");
    let config: serde_json::Value =
        serde_json::from_slice(&fs::read(&config_path).expect("read config"))
            .expect("parse config");
    assert_eq!(
        config["projects"][project_key(&canonical_workspace).expect("project key")]
            ["hasTrustDialogAccepted"],
        true
    );
    #[cfg(unix)]
    assert_eq!(
        fs::metadata(config_path)
            .expect("metadata")
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
}
