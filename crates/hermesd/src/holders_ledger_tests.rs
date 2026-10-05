//! The lineage ledger (H-117 Q1).

use super::*;

fn row(pid: u32, ppid: u32, start: u64) -> Row {
    Row {
        pid,
        ppid,
        start,
        pgid: None,
        sid: None,
    }
}

fn tag(project: &str, bot: &str) -> SessionTag {
    SessionTag {
        project_id: project.into(),
        bot_id: bot.into(),
    }
}

fn pids(entries: &[Entry]) -> Vec<u32> {
    entries.iter().map(|e| e.pid).collect()
}

const NO_ENV: fn(u32) -> Option<String> = |_| None;

#[test]
fn a_detached_grandchild_stays_listed_after_its_session_ends() {
    let mut ledger = Ledger::default();
    ledger.started("s1", tag("phd", "unity"), Some((100, 10)));
    // The session runs a script that starts a server.
    ledger.sweep(&[row(100, 1, 10), row(101, 100, 11), row(102, 101, 12)]);
    // The script detached the server and every ancestor exited: it is now
    // a child of 1, with nothing linking it to the session but the ledger.
    let after = [row(102, 1, 12)];
    ledger.ended("s1", &after);
    let found = ledger.find(&after, None, NO_ENV);
    assert_eq!(pids(&found), vec![102]);
    assert_eq!(found[0].project_id, "phd");
    assert_eq!(found[0].bot_id, "unity");
    // Of its project only.
    assert!(ledger.find(&after, Some("gravity"), NO_ENV).is_empty());
}

#[test]
fn a_recycled_pid_is_never_listed() {
    let mut ledger = Ledger::default();
    ledger.started("s1", tag("p", "b"), Some((100, 10)));
    ledger.sweep(&[row(100, 1, 10), row(101, 100, 11)]);
    // 101 exited and the OS handed its pid to an unrelated process.
    let now = [row(100, 1, 10), row(101, 1, 99)];
    assert_eq!(pids(&ledger.find(&now, None, NO_ENV)), vec![100]);
    ledger.sweep(&now);
    assert_eq!(pids(&ledger.find(&now, None, NO_ENV)), vec![100]);
    // Nor a process born before its supposed parent.
    let mut ledger = Ledger::default();
    ledger.started("s1", tag("p", "b"), Some((100, 10)));
    ledger.sweep(&[row(100, 1, 10), row(200, 100, 5)]);
    assert_eq!(
        pids(&ledger.find(&[row(100, 1, 10), row(200, 100, 5)], None, NO_ENV)),
        vec![100]
    );
}

#[test]
fn children_keep_the_leaders_session_id_after_it_exits() {
    let mut ledger = Ledger::default();
    ledger.started("s1", tag("p", "b"), Some((100, 10)));
    let orphan = Row {
        pid: 300,
        ppid: 1,
        start: 20,
        pgid: Some(250),
        sid: Some(100),
    };
    // The leader is gone; its session id still marks the orphan.
    ledger.sweep(&[orphan]);
    assert_eq!(pids(&ledger.find(&[orphan], None, NO_ENV)), vec![300]);
    // Once the leader's pid names another process, its ids mean nothing.
    let mut ledger = Ledger::default();
    ledger.started("s1", tag("p", "b"), Some((100, 10)));
    let rows = [row(100, 1, 50), orphan];
    ledger.sweep(&rows);
    assert!(ledger.find(&rows, None, NO_ENV).is_empty());
}

#[test]
fn the_environment_tag_finds_what_no_sweep_saw() {
    let mut ledger = Ledger::default();
    ledger.started("s1", tag("phd", "unity"), Some((100, 10)));
    let rows = [row(500, 1, 30), row(501, 1, 31), row(502, 1, 32)];
    let env = |pid: u32| match pid {
        500 => Some("s1".to_string()),
        // A session this daemon never started: not ours.
        501 => Some("elsewhere".to_string()),
        _ => None,
    };
    let found = ledger.find(&rows, None, env);
    assert_eq!(pids(&found), vec![500]);
    assert_eq!(found[0].session, "s1");
}

#[test]
fn a_new_session_ends_the_bots_last_and_finished_sessions_are_forgotten() {
    let mut ledger = Ledger::default();
    ledger.started("s1", tag("p", "b"), Some((100, 10)));
    ledger.started("s2", tag("p", "b"), Some((200, 20)));
    assert!(ledger.sessions["s1"].ended);
    ledger.sweep(&[row(200, 1, 20)]);
    // s1 had nothing left running.
    assert!(ledger.tag("s1").is_none());
    assert!(ledger.tag("s2").is_some());
    ledger.ended_for_bot("b", &[]);
    assert!(ledger.tag("s2").is_none());
}

#[test]
fn procargs_env_is_read_past_the_path_and_arguments() {
    let mut buf = 2i32.to_ne_bytes().to_vec();
    buf.extend_from_slice(b"/bin/sh\0\0\0\0sh\0-c\0HOME=/Users/u\0THEHERMES_SESSION=abc\0\0junk");
    let env = procs::parse_procargs_env(&buf);
    assert_eq!(
        env,
        vec![
            ("HOME".to_string(), "/Users/u".to_string()),
            ("THEHERMES_SESSION".to_string(), "abc".to_string()),
        ]
    );
    assert!(procs::parse_procargs_env(&[1, 0]).is_empty());
}

/// The phd case on a real system: a script that detaches a long runner
/// with `setsid` and exits. Found through the ledger (swept while the
/// session ran) and, independently, through its environment.
#[cfg(unix)]
#[test]
fn a_setsid_runner_outlives_its_session_and_is_still_found() {
    use std::process::Command;
    use std::time::{Duration, Instant};
    let session = format!("test-{}", std::process::id());
    let mut root = Command::new("/bin/sh")
        .args([
            "-c",
            "/usr/bin/perl -MPOSIX -e 'POSIX::setsid(); sleep 60' & sleep 1",
        ])
        .env(procs::SESSION_ENV, &session)
        .spawn()
        .unwrap();
    let root_start = procs::start_of(root.id()).unwrap();
    let mut ledger = Ledger::default();
    ledger.started(&session, tag("phd", "unity"), Some((root.id(), root_start)));
    // Swept while the session runs, as the daemon does every few seconds.
    let deadline = Instant::now() + Duration::from_secs(5);
    while ledger.entries.len() < 2 && Instant::now() < deadline {
        ledger.sweep(&procs::all());
        std::thread::sleep(Duration::from_millis(50));
    }
    root.wait().unwrap();
    let rows = procs::all();
    ledger.ended(&session, &rows);
    let runner: Vec<Entry> = ledger
        .find(&rows, Some("phd"), NO_ENV)
        .into_iter()
        .filter(|e| e.pid != root.id())
        .collect();
    assert_eq!(runner.len(), 1, "{runner:?}");
    let runner = runner[0].clone();
    // Detached: its own session, its parent now 1 (launchd/init).
    let now = rows.iter().find(|r| r.pid == runner.pid).unwrap();
    assert_eq!(now.sid, Some(runner.pid));
    // Without the ledger, the environment alone finds it too where the OS
    // shows another process's environment: Linux. Darwin 27 leaves it out
    // of KERN_PROCARGS2 (checked: only argv comes back), so on macOS the
    // ledger and its sweep stand alone, as the spec's fallback says.
    let mut fresh = Ledger::default();
    fresh.started(&session, tag("phd", "unity"), None);
    let by_env = fresh.find(&rows, None, procs::session_tag);
    if cfg!(target_os = "macos") {
        assert!(procs::session_tag(runner.pid).is_none());
    } else {
        assert!(by_env.iter().any(|e| e.pid == runner.pid), "{by_env:?}");
    }
    // SAFETY: the runner is this test's own child's child.
    unsafe { libc::kill(runner.pid as i32, libc::SIGKILL) };
}
