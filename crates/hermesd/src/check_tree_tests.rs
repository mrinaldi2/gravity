use super::*;

/// What was done to each runner's tree, in order: "kill" (a signal to its
/// group) and "reap".
static EVENTS: Mutex<Vec<(u32, &'static str)>> = Mutex::new(Vec::new());

pub(super) fn record(pid: Option<u32>, event: &'static str) {
    if let Some(pid) = pid {
        EVENTS.lock().unwrap().push((pid, event));
    }
}

#[cfg(unix)]
fn events(pid: u32) -> Vec<&'static str> {
    let events = EVENTS.lock().unwrap();
    events.iter().filter(|e| e.0 == pid).map(|e| e.1).collect()
}

/// A runner as the daemon starts one, running `script` once it is told
/// to go, with its checkout in a job folder of its own.
#[cfg(unix)]
async fn started(home: &Path, script: &str) -> (Guard, u32) {
    let dir = home.join("job");
    std::fs::create_dir_all(dir.join(check_checkout::DIR)).unwrap();
    let mut runner = Command::new("sh");
    runner
        .args(["-c", &format!("head -c1 >/dev/null; {script}")])
        .kill_on_drop(true);
    isolate(&mut runner);
    let guard = start(home, &dir, runner.spawn().unwrap()).await.unwrap();
    let pid = guard.child.id().unwrap();
    (guard, pid)
}

/// M1: once the runner is reaped its pgid may be another group's, so the
/// group is signalled only before: not by the end, nor by a stop or the
/// drop that come after it.
#[cfg(unix)]
#[tokio::test]
async fn a_tree_is_never_signalled_once_its_runner_is_reaped() {
    let home = tempfile::tempdir().unwrap();
    let (mut guard, pid) = started(home.path(), "sleep 30 & sleep 0.2").await;
    let (over, status) = guard.wait(Duration::from_secs(60)).await;
    assert!(!over);
    assert!(status.unwrap().success());
    stop_all(home.path());
    assert!(!guard.finish(), "it ran to its end");
    // The one kill swept the backgrounded sleep while the runner was a
    // zombie; nothing after the reap.
    assert_eq!(events(pid), ["kill", "reap"]);
    assert!(!home.path().join("job").join(check_checkout::DIR).exists());
}

/// Over its time, the tree is killed while the runner is unreaped, then
/// the runner is reaped, and that is the last of it.
#[cfg(unix)]
#[tokio::test]
async fn a_tree_over_its_time_is_killed_before_its_runner_is_reaped() {
    let home = tempfile::tempdir().unwrap();
    let (mut guard, pid) = started(home.path(), "sleep 30").await;
    let (over, _) = guard.wait(Duration::from_millis(300)).await;
    assert!(over);
    drop(guard);
    assert_eq!(events(pid).last(), Some(&"reap"), "{:?}", events(pid));
    assert!(events(pid).starts_with(&["kill"]));
}

/// A daemon stop mid-run is the only signal, and the check says so.
#[cfg(unix)]
#[tokio::test]
async fn a_stopped_tree_says_it_was_stopped() {
    let home = tempfile::tempdir().unwrap();
    let (mut guard, pid) = started(home.path(), "sleep 30").await;
    let stopper = {
        let home = home.path().to_path_buf();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(300));
            stop_all(&home);
        })
    };
    let (over, _) = guard.wait(Duration::from_secs(60)).await;
    stopper.join().unwrap();
    assert!(!over);
    assert!(guard.finish());
    assert_eq!(events(pid).last(), Some(&"reap"), "{:?}", events(pid));
}
