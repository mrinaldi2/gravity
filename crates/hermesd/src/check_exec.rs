//! Running a check (H-283 ARCH M1, M2): no bot and no AI decide whether a
//! check passed. The daemon of the computer the check was routed to makes
//! a fresh checkout at the exact sha in the job's folder under its `run/`
//! (which bots can't write), then starts the runner command
//! `hermesd check run <job folder>` with a clean environment. The runner
//! runs the job's `run` from the base's `checks.toml` in that checkout, its
//! output going to `check.log`, and exits [`PASS`] when it exited 0,
//! [`FAIL`] when it didn't, and [`COULDNT_RUN`] when it couldn't start. The
//! daemon records the result from that exit status alone, then removes the
//! checkout. The log stays in the job's folder.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::check_tree;
use crate::config::Config;
use crate::prs::check_checkout;
use crate::prs::check_model::CheckResult;

/// The runner's exit status when the check's command exited 0.
pub const PASS: i32 = 0;
/// The runner's exit status when the check's command exited non-zero.
pub const FAIL: i32 = 1;
/// The runner's exit status when the check's command couldn't start.
pub const COULDNT_RUN: i32 = 2;

/// The check's output, in the job's folder.
pub const LOG: &str = "check.log";
/// What the runner runs, written by the daemon before it starts it.
const SPEC: &str = "job.json";

/// The longest a check may run by default (`checks.timeout_secs`) before
/// its tree is stopped and it is an `error`.
pub const DEADLINE: Duration = Duration::from_secs(6 * 3600);

/// The variables a check keeps from the daemon's environment: where its
/// tools are, who and where it runs. No token or secret is passed on.
const KEPT_ENV: &[&str] = &[
    "PATH",
    "HOME",
    "USER",
    "LOGNAME",
    "SHELL",
    "LANG",
    "LC_ALL",
    "TMPDIR",
    "CARGO_HOME",
    "RUSTUP_HOME",
    "PNPM_HOME",
    "COREPACK_HOME",
    // Windows.
    "SystemRoot",
    "SystemDrive",
    "windir",
    "ComSpec",
    "PATHEXT",
    "USERPROFILE",
    "USERNAME",
    "APPDATA",
    "LOCALAPPDATA",
    "ProgramData",
    "ProgramFiles",
    "ProgramFiles(x86)",
    "TEMP",
    "TMP",
];

/// One job: the repository, the commit and the command.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Spec {
    pub url: String,
    pub sha: String,
    pub run: String,
}

/// What the daemon records: the result from the runner's exit status, why,
/// and the log when there is one.
#[derive(Debug, Clone)]
pub struct Outcome {
    pub result: CheckResult,
    pub note: String,
    pub log: Option<PathBuf>,
}

/// A job's folder: `<home>/run/checks/<job>`. `job` is a daemon-made id;
/// anything else is refused, as it names a folder.
pub fn job_dir(home: &Path, job: &str) -> anyhow::Result<PathBuf> {
    anyhow::ensure!(
        !job.is_empty()
            && job.len() <= 64
            && job.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'),
        "{job:?} isn't a check job id"
    );
    Ok(home.join("run").join("checks").join(job))
}

/// Runs `spec` as job `job` on this computer and says how it went.
pub async fn run(cfg: &Config, job: &str, spec: Spec) -> Outcome {
    match run_inner(cfg, job, spec).await {
        Ok(outcome) => outcome,
        Err(error) => Outcome {
            result: CheckResult::Error,
            note: format!("couldn't run: {error:#}"),
            log: None,
        },
    }
}

async fn run_inner(cfg: &Config, job: &str, spec: Spec) -> anyhow::Result<Outcome> {
    let dir = job_dir(&cfg.home, job)?;
    let made = {
        let (dir, spec) = (dir.clone(), spec.clone());
        tokio::task::spawn_blocking(move || {
            remove_all(&dir)?;
            fs::create_dir_all(&dir)?;
            fs::write(dir.join(SPEC), serde_json::to_vec(&spec)?)?;
            check_checkout::prepare(&dir, &spec.url, &spec.sha)
        })
        .await?
    };
    if let Err(error) = made {
        return Ok(Outcome {
            result: CheckResult::Error,
            note: format!("couldn't check out {}: {error:#}", spec.sha),
            log: None,
        });
    }
    let exe = match &cfg.checks.runner {
        Some(exe) => exe.clone(),
        None => std::env::current_exe()?,
    };
    let mut runner = tokio::process::Command::new(exe);
    runner
        .args(["check", "run"])
        .arg(&dir)
        .current_dir(&dir)
        .env_clear()
        .envs(
            KEPT_ENV
                .iter()
                .filter_map(|k| Some((k, std::env::var_os(k)?))),
        )
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    check_tree::isolate(&mut runner);
    let child = match runner.spawn() {
        Ok(child) => child,
        Err(error) => {
            let _ = check_checkout::remove(&dir);
            anyhow::bail!("the runner didn't start: {error}");
        }
    };
    // H-291: the whole tree ends before the checkout goes, however the
    // check ends, this future being dropped (the runtime stopping) included.
    // It starts only once contained, or not at all.
    let mut tree = match check_tree::start(&cfg.home, &dir, child).await {
        Ok(tree) => tree,
        Err(error) => {
            return Ok(Outcome {
                result: CheckResult::Error,
                note: format!("couldn't contain the check: {error:#}"),
                log: None,
            })
        }
    };
    let limit = Duration::from_secs(cfg.checks.timeout_secs);
    let (over, status) = tree.wait(limit).await;
    let stopped = tree.finish();
    let log = Some(dir.join(LOG)).filter(|l| l.is_file());
    let (result, note) = match status {
        _ if stopped => (CheckResult::Error, "the daemon stopped while it ran".into()),
        _ if over => (
            CheckResult::Error,
            format!("ran over {} and was stopped", span(limit)),
        ),
        Err(error) => (CheckResult::Error, format!("lost the runner: {error}")),
        Ok(status) => match status.code() {
            Some(PASS) => (CheckResult::Pass, "exited 0".into()),
            Some(FAIL) => (CheckResult::Fail, "exited non-zero; see its log".into()),
            Some(COULDNT_RUN) => (CheckResult::Error, "its command couldn't start".into()),
            _ => (CheckResult::Error, format!("the runner ended: {status}")),
        },
    };
    Ok(Outcome { result, note, log })
}

/// `limit` as a person says it: "6 hours", "90 seconds".
fn span(limit: Duration) -> String {
    match limit.as_secs() {
        3600 => "an hour".into(),
        s if s % 3600 == 0 => format!("{} hours", s / 3600),
        s => format!("{s} seconds"),
    }
}

fn remove_all(dir: &Path) -> std::io::Result<()> {
    match fs::remove_dir_all(dir) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

/// `hermesd check run <job folder>`: runs the job's command in its
/// `check/`, all output to its `check.log`, and exits [`PASS`], [`FAIL`] or
/// [`COULDNT_RUN`]. Started by the daemon, with the environment it chose.
pub fn run_cli(args: &[String]) -> i32 {
    let [verb, dir] = args else {
        eprintln!("usage: hermesd check run <job folder>");
        return COULDNT_RUN;
    };
    if verb != "run" {
        eprintln!("usage: hermesd check run <job folder>");
        return COULDNT_RUN;
    }
    let dir = Path::new(dir);
    let Ok(mut log) = fs::File::create(dir.join(LOG)) else {
        return COULDNT_RUN;
    };
    match run_command(dir, &log) {
        Ok(status) => {
            let _ = writeln!(log, "\n[hermesd: the check exited with {status}]");
            if status.success() {
                PASS
            } else {
                FAIL
            }
        }
        Err(error) => {
            let _ = writeln!(log, "[hermesd: the check couldn't start: {error:#}]");
            COULDNT_RUN
        }
    }
}

/// `run` as the computer's shell runs a line: `sh -c` here, `cmd /C` on
/// Windows, given the line as it is (not quoted again).
#[cfg(not(windows))]
fn shell(run: &str) -> std::process::Command {
    let mut c = std::process::Command::new("sh");
    c.arg("-c").arg(run);
    c
}

#[cfg(windows)]
fn shell(run: &str) -> std::process::Command {
    use std::os::windows::process::CommandExt;
    let mut c = std::process::Command::new("cmd");
    c.arg("/C").raw_arg(run);
    c
}

fn run_command(dir: &Path, log: &fs::File) -> anyhow::Result<std::process::ExitStatus> {
    let spec: Spec = serde_json::from_slice(&fs::read(dir.join(SPEC))?)?;
    let checkout = dir.join(check_checkout::DIR);
    anyhow::ensure!(checkout.is_dir(), "there is no checkout to run in");
    let lifeline = check_tree::wait_for_go()?;
    let mut command = shell(&spec.run);
    let mut child = command
        .current_dir(&checkout)
        .env_remove(check_tree::LIFELINE_ENV)
        .stdin(Stdio::null())
        .stdout(log.try_clone()?)
        .stderr(log.try_clone()?)
        .spawn()?;
    if lifeline {
        check_tree::watch_lifeline();
    }
    Ok(child.wait()?)
}
