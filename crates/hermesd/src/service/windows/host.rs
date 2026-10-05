//! The Task Scheduler side of an install: the pre-rename `Gravity-…` task
//! and the current `The Hermes-…` one, stopping either with its whole
//! process tree, and registering one. `schtasks.exe` sits behind
//! [`Schtasks`], so tests never create a task.
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::Context;

use super::process_tree;
use super::reap::{reap, stop_daemon};
use super::sequence::{Host, Identity};
use super::stage::remove_if_present;
use super::task::{task_name, task_name_for, task_name_in, write_task_files};
use super::ServicePaths;

/// The Task Scheduler calls an install makes, plus the version and health
/// probes, which tests replace as well.
pub(super) trait Schtasks {
    fn run(&self, args: &[&str]) -> anyhow::Result<()>;
    fn exists(&self, name: &str) -> bool;
    /// Waits for the task to stop running (Ready, or Disabled when a stop
    /// disabled it first) with its process tree gone. `/end` returns before
    /// Task Scheduler has finished ending the launcher, and a `/run` in
    /// that interval reports success while IgnoreNew drops it.
    fn wait_ended(&self, name: &str) -> anyhow::Result<()>;
    fn version_of(&self, binary: &Path) -> anyhow::Result<String> {
        super::stage::version_of(binary)
    }
    fn wait_healthy(&self, home: &Path, port: u16, version: &str) -> anyhow::Result<()> {
        super::sequence::wait_healthy(home, port, version)
    }
}

/// The real `schtasks.exe`.
pub(super) struct System;

impl Schtasks for System {
    fn run(&self, args: &[&str]) -> anyhow::Result<()> {
        super::task::run_task(args)
    }
    fn exists(&self, name: &str) -> bool {
        super::task::task_exists(name)
    }
    fn wait_ended(&self, name: &str) -> anyhow::Result<()> {
        let deadline = Instant::now() + Duration::from_secs(30);
        let mut child = Command::new("powershell.exe")
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                &ended_script(name),
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;
        // The first line names the task's running instances: pin their
        // process trees before they exit and their PIDs can be reused.
        let mut first = String::new();
        if let Some(stdout) = child.stdout.take() {
            BufReader::new(stdout).read_line(&mut first)?;
        }
        let mut tree = Vec::new();
        for pid in engine_pids(&first) {
            tree.extend(process_tree::tree(pid)?);
        }
        let status = child.wait()?;
        anyhow::ensure!(status.success(), "task {name} did not finish stopping");
        wait_exited(&tree, deadline).with_context(|| format!("task {name} left processes running"))
    }
}

/// Task Scheduler's `TASK_STATE_DISABLED` and `TASK_STATE_READY`: a task
/// that is not running. A stop disables the task first, so it ends Disabled.
pub(super) const ENDED_STATES: [u32; 2] = [1, 3];

/// Prints the PIDs of the task's running instances, then waits up to 30 s
/// for it to reach an ended state with no instance left.
pub(super) fn ended_script(name: &str) -> String {
    let ended = ENDED_STATES.map(|state| state.to_string()).join(",");
    format!(
        "$s = New-Object -ComObject Schedule.Service; $s.Connect(); $t = $s.GetFolder('\\').GetTask('{}'); \
         Write-Output ((@($t.GetInstances(0)) | ForEach-Object {{ $_.EnginePID }}) -join ','); \
         $until = [DateTime]::UtcNow.AddSeconds(30); \
         while (-not (@({ended}) -contains $t.State) -or $t.GetInstances(0).Count -gt 0) {{ if ([DateTime]::UtcNow -ge $until) {{ exit 1 }}; Start-Sleep -Milliseconds 100 }}",
        name.replace('\'', "''")
    )
}

/// The engine PIDs on the script's first line.
pub(super) fn engine_pids(line: &str) -> Vec<u32> {
    line.trim()
        .split(',')
        .filter_map(|pid| pid.trim().parse().ok())
        .filter(|&pid| pid != 0)
        .collect()
}

/// Waits until every process in `tree` has exited.
pub(super) fn wait_exited(tree: &[process_tree::Process], deadline: Instant) -> anyhow::Result<()> {
    while tree.iter().any(|process| !process.exited()) {
        anyhow::ensure!(Instant::now() < deadline, "process tree still running");
        std::thread::sleep(Duration::from_millis(100));
    }
    Ok(())
}

/// The tasks an install moves between.
pub(super) struct TaskScheduler<'a, S: Schtasks> {
    pub(super) paths: &'a ServicePaths,
    /// The pre-rename tasks and the homes they run from. Named before the
    /// migration: they hash the old home's resolved path, which the
    /// compatibility junction changes afterwards.
    pub(super) legacy: Vec<(String, PathBuf)>,
    pub(super) port: u16,
    pub(super) schtasks: S,
}

impl<'a, S: Schtasks> TaskScheduler<'a, S> {
    /// Finds the pre-rename tasks: in `old_home` (where the daemon runs
    /// now), in this home when set explicitly, or in the default old home.
    pub(super) fn new(
        paths: &'a ServicePaths,
        old_home: &Path,
        port: u16,
        schtasks: S,
    ) -> anyhow::Result<Self> {
        let mut homes = vec![old_home.to_path_buf()];
        homes.extend(paths.legacy_homes());
        let mut legacy: Vec<(String, PathBuf)> = Vec::new();
        for home in homes {
            let marker = home.join(crate::brand::legacy_daemon_file("-task.xml"));
            if marker.is_file() && !legacy.iter().any(|(_, known)| *known == home) {
                let name = legacy_name(&schtasks, &home)?;
                legacy.push((name, home));
            }
        }
        Ok(Self {
            paths,
            legacy,
            port,
            schtasks,
        })
    }

    /// The tasks of `id`, each with its home and its launcher's PID file.
    fn tasks(&self, id: Identity) -> anyhow::Result<Vec<(String, PathBuf, PathBuf)>> {
        Ok(match id {
            Identity::Legacy => self
                .legacy
                .iter()
                .map(|(name, home)| {
                    let pid = home.join(crate::brand::legacy_daemon_file("-task.pid"));
                    (name.clone(), home.clone(), pid)
                })
                .collect(),
            Identity::Current => vec![(
                task_name(self.paths)?,
                self.paths.home.clone(),
                self.paths.pid_path(),
            )],
        })
    }

    /// Ends the task, its daemon's whole process tree, and any daemon
    /// running from the home's `bin` that no PID file names; then waits
    /// for the task to finish and the home to be released.
    fn stop_task(&self, name: &str, home: &Path, pid_path: &Path) -> anyhow::Result<()> {
        // Ending an idle task returns an error; what follows verifies stop.
        let _ = self.schtasks.run(&["/end", "/tn", name]);
        // Never kill a reused PID belonging to another executable. An upgrade
        // from before the rename stops the task's old `gravityd.exe`.
        let bin = home.join("bin");
        let executables = [bin.join("hermesd.exe"), bin.join("gravityd.exe")];
        if let Ok(pid) = std::fs::read_to_string(pid_path) {
            let pid: u32 = pid.trim().parse().context("invalid managed daemon PID")?;
            stop_daemon(pid, &executables)?;
            remove_if_present(pid_path)?;
        }
        // A launcher that died before writing its PID file, or one whose
        // file was overwritten, leaves a daemon the file does not name.
        reap(&executables)?;
        if self.schtasks.exists(name) {
            self.schtasks.wait_ended(name)?;
        }
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            match crate::home::lock(home) {
                Ok(_lock) => return Ok(()),
                Err(error) if Instant::now() >= deadline => {
                    return Err(error).context("waiting for the managed daemon to stop")
                }
                Err(_) => std::thread::sleep(Duration::from_millis(100)),
            }
        }
    }
}

/// The pre-rename task for `home`, named from its resolved path as 0.14
/// did. Once the home has moved (an install that died after the migration),
/// `home` is a junction that resolves to the new home; the task is then
/// found under the path it had before.
fn legacy_name(schtasks: &impl Schtasks, home: &Path) -> anyhow::Result<String> {
    let label = crate::brand::LEGACY_WINDOWS_TASK;
    let resolved = task_name_in(label, home)?;
    if schtasks.exists(&resolved) {
        return Ok(resolved);
    }
    let unresolved = format!(r"\\?\{}", std::path::absolute(home)?.display());
    let before = task_name_for(
        label,
        &crate::permissions::user_sid()?,
        &unresolved.to_lowercase(),
    );
    Ok(if schtasks.exists(&before) {
        before
    } else {
        resolved
    })
}

impl<S: Schtasks> Host for TaskScheduler<'_, S> {
    fn installed(&self) -> Vec<Identity> {
        let mut ids = Vec::new();
        if !self.legacy.is_empty() {
            ids.push(Identity::Legacy);
        }
        if self.paths.plist_path().is_file() {
            ids.push(Identity::Current);
        }
        ids
    }

    fn disable(&self, id: Identity) -> anyhow::Result<()> {
        for (name, _, _) in self.tasks(id)? {
            // A marker without its task (deleted by hand) has nothing to
            // disable. RestartOnFailure would relaunch a killed launcher.
            if self.schtasks.exists(&name) {
                self.schtasks.run(&["/change", "/tn", &name, "/disable"])?;
            }
        }
        Ok(())
    }

    fn stop(&self, id: Identity) -> anyhow::Result<()> {
        for (name, home, pid) in self.tasks(id)? {
            tracing::info!(task = %name, "stopping the managed task");
            self.stop_task(&name, &home, &pid)?;
        }
        Ok(())
    }

    fn definition_files(&self) -> Vec<PathBuf> {
        vec![self.paths.launcher_path(), self.paths.plist_path()]
    }

    fn write_definition(&self) -> anyhow::Result<()> {
        write_task_files(self.paths)
    }

    fn start(&self, id: Identity) -> anyhow::Result<()> {
        match id {
            // Still registered, only disabled: enable it again. Every task
            // is tried, so one failing never leaves the others down.
            Identity::Legacy => {
                let mut failed = Vec::new();
                for (name, _) in &self.legacy {
                    let started = self
                        .schtasks
                        .run(&["/change", "/tn", name, "/enable"])
                        .and_then(|()| self.schtasks.run(&["/run", "/tn", name]));
                    if let Err(error) = started {
                        failed.push(format!("{error:#}"));
                    }
                }
                anyhow::ensure!(failed.is_empty(), "{}", failed.join("; "));
                Ok(())
            }
            // (Re)created, enabled, from the definition on disk.
            Identity::Current => {
                let name = task_name(self.paths)?;
                let definition = self.paths.plist_path();
                self.schtasks.run(&[
                    "/create",
                    "/tn",
                    &name,
                    "/xml",
                    &definition.to_string_lossy(),
                    "/f",
                ])?;
                self.schtasks.run(&["/run", "/tn", &name])
            }
        }
    }

    fn remove(&self, id: Identity) -> anyhow::Result<()> {
        for (name, home, _) in self.tasks(id)? {
            if self.schtasks.exists(&name) {
                if let Err(error) = self.schtasks.run(&["/delete", "/tn", &name, "/f"]) {
                    // A task left behind is disabled; its files go below.
                    tracing::warn!(%error, task = %name, "could not delete the task");
                }
            }
            if id == Identity::Legacy {
                // Wherever the migration left them.
                for dir in [&home, &self.paths.home] {
                    for suffix in ["-task.xml", "-task.ps1", "-task.pid"] {
                        remove_if_present(&dir.join(crate::brand::legacy_daemon_file(suffix)))?;
                    }
                }
            }
        }
        Ok(())
    }

    fn version_of(&self, binary: &Path) -> anyhow::Result<String> {
        self.schtasks.version_of(binary)
    }

    fn wait_healthy(&self, version: &str) -> anyhow::Result<()> {
        self.schtasks
            .wait_healthy(&self.paths.home, self.port, version)
    }
}
