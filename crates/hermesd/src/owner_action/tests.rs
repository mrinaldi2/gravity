//! Owner actions: the hash, the stored row and running one (H-117 R1).

use super::model::{OwnerAction, Pinned, Proposal, Shell, State};
use super::run;
use crate::db::Db;
use chrono::{Duration, Utc};

fn proposal(content: &str) -> Proposal {
    Proposal {
        project_id: "p".into(),
        proposed_by: "bot:b".into(),
        item_id: None,
        decision_id: None,
        target_machine: "d".into(),
        shell: Shell::here(),
        cwd: std::env::temp_dir().display().to_string(),
        content: content.into(),
        pinned_files: Vec::new(),
        reason: "because".into(),
        timeout_s: 5,
    }
}

#[test]
fn the_hash_covers_every_field() {
    let base = proposal("echo hi");
    let same = proposal("echo hi");
    assert_eq!(base.sha256(), same.sha256());
    let changed: Vec<Proposal> = vec![
        Proposal {
            content: "echo hi ".into(),
            ..base.clone()
        },
        Proposal {
            cwd: "/".into(),
            ..base.clone()
        },
        Proposal {
            target_machine: "other".into(),
            ..base.clone()
        },
        Proposal {
            timeout_s: 6,
            ..base.clone()
        },
        Proposal {
            shell: Shell::Bash,
            ..base.clone()
        },
        Proposal {
            reason: "why not".into(),
            ..base.clone()
        },
        Proposal {
            item_id: Some("H-1".into()),
            ..base.clone()
        },
        Proposal {
            pinned_files: vec![Pinned {
                path: "/x".into(),
                sha256: "0".into(),
            }],
            ..base.clone()
        },
    ];
    for other in changed {
        assert_ne!(other.sha256(), base.sha256(), "{other:?}");
    }
}

fn stored(db: &Db, content: &str) -> OwnerAction {
    let p = proposal(content);
    let now = Utc::now();
    let a = OwnerAction {
        id: bus::new_id(),
        sha256: p.sha256(),
        proposal: p,
        flags: Vec::new(),
        origin: "bot".into(),
        state: State::Proposed,
        created_at: now,
        expires_at: now + Duration::hours(24),
        run_by: None,
        run_at: None,
        finished_at: None,
        exit_code: None,
        output_path: None,
        output_tail: None,
        reject_reason: None,
        local_project_id: None,
    };
    db.insert_owner_action(&a).unwrap();
    a
}

#[test]
fn a_proposal_cant_change_and_runs_once() {
    let db = Db::open_in_memory().unwrap();
    let a = stored(&db, "echo hi");
    // The trigger refuses any change to what was proposed.
    let changed = db.rerun_sql(&format!(
        "UPDATE owner_action SET content = 'rm -rf ~' WHERE id = '{}'",
        a.id
    ));
    assert!(format!("{:#}", changed.unwrap_err()).contains("immutable"));
    // The audit is append-only.
    db.audit_owner_action(&a.id, "bot:b", "proposed", &serde_json::json!({}))
        .unwrap();
    assert!(db.rerun_sql("DELETE FROM owner_action_audit").is_err());
    assert!(db
        .rerun_sql("UPDATE owner_action_audit SET event = 'x'")
        .is_err());
    // A wrong hash claims nothing; two claims, one run.
    let now = Utc::now();
    assert!(!db
        .claim_owner_action(&a.id, "0".repeat(64).as_str(), "user", now)
        .unwrap());
    assert!(db
        .claim_owner_action(&a.id, &a.sha256, "user", now)
        .unwrap());
    assert!(!db
        .claim_owner_action(&a.id, &a.sha256, "user", now)
        .unwrap());
    // An expired one can't be claimed.
    let late = stored(&db, "echo late");
    assert!(!db
        .claim_owner_action(&late.id, &late.sha256, "user", now + Duration::hours(25))
        .unwrap());
    assert_eq!(
        db.expire_owner_actions(now + Duration::hours(25)).unwrap(),
        vec![late.id]
    );
}

#[test]
fn powershell_gets_the_content_utf16_encoded() {
    // "ls" as UTF-16LE is 6c 00 73 00.
    assert_eq!(run::encoded_command("ls"), "bABzAA==");
    let (_, args) = run::argv(Shell::Zsh, "echo 'a b'");
    assert_eq!(args, ["-f", "-c", "echo 'a b'"]);
}

#[cfg(unix)]
#[tokio::test]
async fn a_run_reports_its_exit_and_output() {
    let log = tempfile::tempdir().unwrap();
    let path = log.path().join("a.log");
    let ran = run::execute(&proposal("echo out; echo err >&2; exit 3"), &path, |_| {}).await;
    assert_eq!(ran.state, State::Failed);
    assert_eq!(ran.exit_code, Some(3));
    assert!(
        ran.output.contains("out") && ran.output.contains("err"),
        "{}",
        ran.output
    );
    let logged = std::fs::read_to_string(&path).unwrap();
    assert!(logged.contains("out"));
    use std::os::unix::fs::PermissionsExt;
    assert_eq!(
        std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    let ok = run::execute(&proposal("echo fine"), &path, |_| {}).await;
    assert_eq!((ok.state, ok.exit_code), (State::Succeeded, Some(0)));
}

#[cfg(unix)]
#[tokio::test]
async fn a_timeout_stops_the_run_and_everything_it_started() {
    let log = tempfile::tempdir().unwrap();
    let mut p = proposal("sleep 60 & echo child=$!; sleep 60");
    p.timeout_s = 1;
    let ran = run::execute(&p, &log.path().join("t.log"), |_| {}).await;
    assert_eq!(ran.state, State::TimedOut);
    let child: u32 = ran
        .output
        .lines()
        .find_map(|l| l.strip_prefix("child="))
        .and_then(|pid| pid.trim().parse().ok())
        .expect("the grandchild's pid");
    std::thread::sleep(std::time::Duration::from_millis(200));
    assert_eq!(
        crate::holders::procs::start_of(child),
        None,
        "the grandchild was stopped"
    );
}

/// A local connection is traced to the process that made it, which is how
/// a run from a bot's own session is told apart (ws/owner_actions.rs).
#[cfg(unix)]
#[test]
fn a_local_connection_is_traced_to_its_process() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let mut client = std::process::Command::new("/usr/bin/nc")
        .args(["127.0.0.1", &port.to_string()])
        .stdin(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let (_socket, peer) = listener.accept().unwrap();
    assert_eq!(crate::holders::procs::tcp_client(peer), Some(client.id()));
    let _ = client.kill();
    let _ = client.wait();
}

/// The PowerShell variant: the content arrives through -EncodedCommand.
#[cfg(windows)]
#[tokio::test]
async fn powershell_runs_the_encoded_content() {
    let log = tempfile::tempdir().unwrap();
    let mut p = proposal("Write-Output \"quotes 'and' $([char]0x263A)\"; exit 2");
    p.shell = Shell::Powershell;
    let ran = run::execute(&p, &log.path().join("p.log"), |_| {}).await;
    assert_eq!(
        (ran.state, ran.exit_code),
        (State::Failed, Some(2)),
        "{}",
        ran.output
    );
    assert!(ran.output.contains("quotes 'and'"), "{}", ran.output);
}
