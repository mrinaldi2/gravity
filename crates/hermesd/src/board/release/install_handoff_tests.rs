use std::path::Path;

use super::super::handoff::{cmd_script, outcome, Job};
#[cfg(unix)]
use super::super::handoff::{launchd_plist, sh_script};

fn job(home: &Path) -> Job {
    Job::new(
        home,
        "r1",
        "/Applications/The Hermes.app/Contents/MacOS/hermesd".into(),
        vec![
            "service".into(),
            "install".into(),
            "--config".into(),
            "/h/it's.toml".into(),
        ],
        &home.join("stage"),
    )
}

// The sh script is for launchd and unix only; its paths use '/'.
#[cfg(unix)]
#[test]
fn the_job_runs_once_records_its_outcome_and_cleans_up() {
    let home = Path::new("/h");
    let job = job(home);
    assert_eq!(job.label, "com.thehermes.release-install.r1");
    let script = sh_script(&job, true);
    assert!(
        script.starts_with(
            "'/Applications/The Hermes.app/Contents/MacOS/hermesd' 'service' 'install'"
        ),
        "{script}"
    );
    assert!(script.contains(r"'/h/it'\''s.toml'"), "quoted: {script}");
    assert!(
        script.contains("> '/h/logs/release-install-r1.log' 2>&1"),
        "{script}"
    );
    assert!(
        script.contains("echo $? > '/h/run/release-install-r1.status'"),
        "{script}"
    );
    assert!(script.contains("rm -rf '/h/stage'"), "{script}");
    assert!(script.ends_with("launchctl bootout gui/$(id -u)/com.thehermes.release-install.r1\n"));
    assert!(!sh_script(&job, false).contains("launchctl"));

    let plist = launchd_plist(&job.label, "a && b > c");
    assert!(plist.contains("<key>RunAtLoad</key><true/>"));
    assert!(plist.contains("<key>KeepAlive</key><false/>"));
    assert!(plist.contains("a &amp;&amp; b &gt; c"), "escaped: {plist}");
}

#[test]
fn the_windows_job_records_its_outcome_and_unschedules_itself() {
    let job = job(Path::new("/h"));
    let cmd = cmd_script(&job);
    assert!(cmd.contains("echo %ERRORLEVEL%>"), "{cmd}");
    assert!(cmd.contains("schtasks /delete /tn \"com.thehermes.release-install.r1\" /f"));
    assert!(cmd.ends_with("del \"%~f0\"\r\n"));
}

#[test]
fn status_reads_the_outcome_the_job_left() {
    let dir = tempfile::tempdir().expect("dir");
    let home = dir.path();
    assert!(outcome(home, "r1").is_err(), "nothing handed off");
    std::fs::create_dir_all(home.join("run")).unwrap();
    std::fs::create_dir_all(home.join("logs")).unwrap();
    std::fs::write(home.join("run/release-install-r1.status"), "running\n").unwrap();
    assert_eq!(outcome(home, "r1").unwrap().0, None);
    std::fs::write(home.join("run/release-install-r1.status"), "0\n").unwrap();
    let log: String = (1..=30).map(|n| format!("line {n}\n")).collect();
    std::fs::write(home.join("logs/release-install-r1.log"), log).unwrap();
    let (code, tail) = outcome(home, "r1").unwrap();
    assert_eq!(code, Some(0));
    assert!(
        tail.starts_with("line 11") && tail.ends_with("line 30"),
        "{tail}"
    );
}
