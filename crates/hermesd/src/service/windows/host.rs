//! What an install asks of Task Scheduler and the daemon it starts.
use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

use anyhow::Context;

use super::task::{run_task, stop, task_exists, task_name};
use super::ServicePaths;

/// The Task Scheduler and daemon operations an install drives, apart from
/// the files it moves, so tests can run the whole sequence without a task.
pub(super) trait Host {
    /// Keeps RestartOnFailure from relaunching a daemon while it is stopped.
    fn disable(&self) -> anyhow::Result<()>;
    /// Ends the task and waits until no managed daemon process is left.
    fn stop(&self) -> anyhow::Result<()>;
    /// (Re)creates the task, enabled, from the definition on disk.
    fn register(&self) -> anyhow::Result<()>;
    fn start(&self) -> anyhow::Result<()>;
    fn delete(&self) -> anyhow::Result<()>;
    /// What `binary --version` reports.
    fn version_of(&self, binary: &Path) -> anyhow::Result<String>;
    /// Waits for `/health` to report `version`.
    fn wait_healthy(&self, version: &str) -> anyhow::Result<()>;
}

pub(super) struct TaskScheduler<'a> {
    pub(super) paths: &'a ServicePaths,
    pub(super) name: String,
    pub(super) port: u16,
}

impl<'a> TaskScheduler<'a> {
    pub(super) fn new(paths: &'a ServicePaths, port: u16) -> anyhow::Result<Self> {
        Ok(Self {
            name: task_name(paths)?,
            paths,
            port,
        })
    }
}

const VERSION_TIMEOUT: Duration = Duration::from_secs(15);
const HEALTH_TIMEOUT: Duration = Duration::from_secs(45);

impl Host for TaskScheduler<'_> {
    fn disable(&self) -> anyhow::Result<()> {
        // A marker without its task (deleted by hand) has nothing to disable.
        if !task_exists(&self.name) {
            return Ok(());
        }
        run_task(&["/change", "/tn", &self.name, "/disable"])
    }

    fn stop(&self) -> anyhow::Result<()> {
        stop(self.paths)
    }

    fn register(&self) -> anyhow::Result<()> {
        let marker = self.paths.plist_path();
        run_task(&[
            "/create",
            "/tn",
            &self.name,
            "/xml",
            &marker.to_string_lossy(),
            "/f",
        ])
    }

    fn start(&self) -> anyhow::Result<()> {
        run_task(&["/run", "/tn", &self.name])
    }

    fn delete(&self) -> anyhow::Result<()> {
        if !task_exists(&self.name) {
            return Ok(());
        }
        run_task(&["/delete", "/tn", &self.name, "/f"])
    }

    fn version_of(&self, binary: &Path) -> anyhow::Result<String> {
        let mut child = Command::new(binary)
            .arg("--version")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn()
            .with_context(|| format!("running {} --version", binary.display()))?;
        let deadline = Instant::now() + VERSION_TIMEOUT;
        while child.try_wait()?.is_none() {
            if Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                anyhow::bail!("{} --version did not exit", binary.display());
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        let out = child.wait_with_output()?;
        anyhow::ensure!(
            out.status.success(),
            "{} --version failed",
            binary.display()
        );
        parse_version(&String::from_utf8_lossy(&out.stdout))
    }

    fn wait_healthy(&self, version: &str) -> anyhow::Result<()> {
        let deadline = Instant::now() + HEALTH_TIMEOUT;
        let mut seen = None;
        loop {
            // The daemon publishes a negotiated port once it is listening.
            let port = crate::home::runtime_port(&self.paths.home).unwrap_or(self.port);
            seen = crate::server::probe_health(port, Duration::from_secs(1)).or(seen);
            if seen.as_deref() == Some(version) {
                return Ok(());
            }
            if Instant::now() >= deadline {
                anyhow::bail!(
                    "the new daemon did not pass its health check within {}s (expected {version}, saw {})",
                    HEALTH_TIMEOUT.as_secs(),
                    seen.as_deref().unwrap_or("nothing")
                );
            }
            std::thread::sleep(Duration::from_millis(250));
        }
    }
}

/// The version in `hermesd --version` output (`hermesd 0.14.3`).
pub(super) fn parse_version(output: &str) -> anyhow::Result<String> {
    let version = output
        .split_whitespace()
        .last()
        .context("the binary reported no version")?;
    anyhow::ensure!(
        version.starts_with(|c: char| c.is_ascii_digit()),
        "unexpected version output: {}",
        output.trim()
    );
    Ok(version.to_string())
}
