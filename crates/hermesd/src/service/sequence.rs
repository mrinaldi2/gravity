//! The order every `service install` follows, on macOS and Windows alike,
//! with the home migration in the middle:
//!
//! 1. **Stage** the new binary as `.new` beside the live one, in the home
//!    the daemon runs from now (so it moves with the home), and verify it.
//! 2. **Preflight** the migration (destination, disk, ...).
//! 3. **Disable and stop** the running service, keeping its definition.
//! 4. **Migrate** the home.
//! 5. **Swap** the binary by rename, keeping the old one as `.old`.
//! 6. **Register and start** the current identity, then wait for `/health`
//!    to report the staged version.
//! 7. **Only then** remove the pre-rename identity, the `.old` files and the
//!    pre-rename binary.
//!
//! A failure in 1–2 stops nothing. A failure in 3–6 goes through one
//! [`rollback`]: the new identity is stopped (and removed unless it was the
//! one running before), the kept files restored, the migration rolled back,
//! and the services that ran before started again.
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::Context;

use super::stage::{remove_if_present, same_path, stage, with_suffix, Backup};

/// A service registration. The two never share a name, so removing the old
/// one cannot touch the new one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Identity {
    /// What a release before the rename registered: launchd's
    /// `in.mikolajczuk.gravityd`, or the `Gravity-<sid>-<hash>` task, named
    /// for the home before it moved.
    Legacy,
    /// This release's: `com.manuelrinaldi.thehermesd`, or the
    /// `The Hermes-<sid>-<hash>` task, in the home the daemon runs from.
    Current,
}

/// What an install asks of launchd or Task Scheduler, apart from the files it
/// moves, so tests can run the whole sequence without touching either.
pub(crate) trait Host {
    /// The identities registered now: the ones the install stops and a
    /// rollback starts again.
    fn installed(&self) -> Vec<Identity>;
    /// Keeps a restart policy from relaunching the service while it is
    /// stopped. Its definition stays.
    fn disable(&self, id: Identity) -> anyhow::Result<()>;
    /// Stops the service and waits until none of its processes is left and
    /// the home is released.
    fn stop(&self, id: Identity) -> anyhow::Result<()>;
    /// The files that define [`Identity::Current`], kept as `.old` while a
    /// new definition proves itself.
    fn definition_files(&self) -> Vec<PathBuf>;
    /// Writes [`Identity::Current`]'s definition for the new binary.
    fn write_definition(&self) -> anyhow::Result<()>;
    /// Registers `id` from its definition on disk, enabled, and starts it.
    fn start(&self, id: Identity) -> anyhow::Result<()>;
    /// Unregisters `id` and deletes its definition.
    fn remove(&self, id: Identity) -> anyhow::Result<()>;
    /// What `binary --version` reports.
    fn version_of(&self, binary: &Path) -> anyhow::Result<String>;
    /// Waits for `/health` to report `version`.
    fn wait_healthy(&self, version: &str) -> anyhow::Result<()>;
}

/// The home migration, as the install drives it.
pub(crate) trait Migration {
    /// What would stop the move, checked before anything is stopped.
    fn preflight(&self) -> anyhow::Result<()>;
    fn run(&self) -> anyhow::Result<()>;
    fn rollback(&self) -> anyhow::Result<()>;
}

/// [`Migration`] for a real [`crate::migrate_home::Plan`].
pub(crate) struct HomeMigration<'a>(pub(crate) &'a crate::migrate_home::Plan);

impl Migration for HomeMigration<'_> {
    fn preflight(&self) -> anyhow::Result<()> {
        let blockers = crate::migrate_home::blockers_before_stop(self.0)?;
        anyhow::ensure!(
            blockers.is_empty(),
            "cannot migrate {}: {}",
            self.0.from.display(),
            blockers.join("; ")
        );
        Ok(())
    }
    fn run(&self) -> anyhow::Result<()> {
        crate::migrate_home::run(self.0, &mut std::io::stdout()).map(drop)
    }
    fn rollback(&self) -> anyhow::Result<()> {
        crate::migrate_home::rollback(self.0, &mut std::io::stdout())
    }
}

/// Where the install puts things.
pub(crate) struct Layout {
    /// The managed binary in the home the daemon runs from now. The new one
    /// is staged beside it, so a migration carries it along.
    pub(crate) from_bin: PathBuf,
    /// The managed binary once the home has moved (`from_bin` without a
    /// migration).
    pub(crate) bin: PathBuf,
    /// Binaries from before the rename, removed once the new daemon is
    /// healthy.
    pub(crate) leftovers: Vec<PathBuf>,
    /// The published port, stale once the old daemon stopped.
    pub(crate) port_file: PathBuf,
    /// Directories the new home needs (`bin`, `logs`).
    pub(crate) dirs: Vec<PathBuf>,
    /// The config, written from `default_config` when absent.
    pub(crate) config: PathBuf,
    pub(crate) default_config: &'static str,
}

/// What [`rollback`] has to undo.
#[derive(Default)]
struct Progress {
    migrating: bool,
    backup: Backup,
    registered: bool,
}

/// Installs `source` and starts it, migrating the home in between when
/// `migration` is given, or leaves what ran before running. See the module
/// docs for the order.
pub(crate) fn upgrade(
    source: &Path,
    layout: &Layout,
    host: &impl Host,
    migration: Option<&dyn Migration>,
) -> anyhow::Result<()> {
    // 1. Stage. Reinstalling the installed binary itself has nothing to copy.
    let staged = migration.is_some() || !same_path(source, &layout.bin);
    let version = if staged {
        stage(source, &layout.from_bin, |binary| host.version_of(binary))?.1
    } else {
        host.version_of(source)?
    };
    let discard_staged = || {
        for bin in [&layout.from_bin, &layout.bin] {
            let _ = remove_if_present(&with_suffix(bin, ".new"));
        }
    };
    // 2. Preflight.
    if let Some(migration) = migration {
        if let Err(error) = migration.preflight() {
            discard_staged();
            return Err(error.context("nothing was stopped"));
        }
    }
    // 3. Disable and stop what runs now.
    let old = host.installed();
    for id in &old {
        if let Err(error) = host.disable(*id).and_then(|()| host.stop(*id)) {
            discard_staged();
            // It may be half-stopped; put it back as it was.
            return Err(match start_all(host, &old) {
                Ok(()) => error.context("could not stop the running daemon; it was left running"),
                Err(again) => error.context(format!(
                    "could not stop the running daemon, nor restart it: {again:#}"
                )),
            });
        }
    }
    // 4–6. Migrate, swap, register and start, health.
    let mut progress = Progress::default();
    let installed = install(layout, host, migration, staged, &version, &mut progress);
    if let Err(error) = installed {
        discard_staged();
        return Err(match rollback(host, &old, &progress, migration) {
            Ok(()) if old.is_empty() => error.context("install failed and was undone"),
            Ok(()) => error.context("install failed; the previous daemon was restored"),
            Err(again) => {
                error.context(format!("install failed, and so did undoing it: {again:#}"))
            }
        });
    }
    // 7. The new daemon is healthy: the old identity and binaries can go.
    if old.contains(&Identity::Legacy) {
        host.remove(Identity::Legacy)
            .context("the new daemon runs, but the pre-rename service could not be removed")?;
    }
    progress.backup.discard()?;
    for leftover in &layout.leftovers {
        remove_if_present(leftover)?;
    }
    Ok(())
}

fn install(
    layout: &Layout,
    host: &impl Host,
    migration: Option<&dyn Migration>,
    staged: bool,
    version: &str,
    progress: &mut Progress,
) -> anyhow::Result<()> {
    if let Some(migration) = migration {
        // Marked first: a run that fails partway is rolled back too.
        progress.migrating = true;
        migration.run().context("migrating the home")?;
    }
    for dir in &layout.dirs {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    if !layout.config.exists() {
        std::fs::write(&layout.config, layout.default_config)
            .with_context(|| format!("writing {}", layout.config.display()))?;
    }
    // The health check must read the new daemon's port, not the old one's.
    remove_if_present(&layout.port_file)?;
    if staged {
        progress.backup.keep(layout.bin.clone())?;
        std::fs::rename(with_suffix(&layout.bin, ".new"), &layout.bin)
            .context("swapping in the new daemon binary")?;
    }
    for file in host.definition_files() {
        progress.backup.keep(file)?;
    }
    progress.registered = true;
    host.write_definition()?;
    host.start(Identity::Current)?;
    host.wait_healthy(version)
}

/// The one way back from a failed install: stop the new identity, restore
/// the kept files, roll the migration back, start what ran before.
fn rollback(
    host: &impl Host,
    old: &[Identity],
    progress: &Progress,
    migration: Option<&dyn Migration>,
) -> anyhow::Result<()> {
    if progress.registered {
        // The new daemon must be gone before its binary or home can move.
        let _ = host.disable(Identity::Current);
        host.stop(Identity::Current)?;
        if !old.contains(&Identity::Current) {
            host.remove(Identity::Current)?;
        }
    }
    progress.backup.restore()?;
    if let (true, Some(migration)) = (progress.migrating, migration) {
        migration
            .rollback()
            .context("rolling the home migration back")?;
    }
    start_all(host, old)
}

fn start_all(host: &impl Host, ids: &[Identity]) -> anyhow::Result<()> {
    for id in ids {
        host.start(*id)?;
    }
    Ok(())
}

/// `service restart`: restarts the current identity, first reinstalling its
/// binary from `bundled` when the service outlived it (it would have nothing
/// to start). A repair, never a migration.
pub(crate) fn restart(bundled: &Path, layout: &Layout, host: &impl Host) -> anyhow::Result<()> {
    anyhow::ensure!(
        host.installed().contains(&Identity::Current),
        "no managed daemon is installed"
    );
    if !layout.bin.is_file() {
        tracing::warn!(
            binary = %layout.bin.display(),
            source = %bundled.display(),
            "managed daemon binary is missing; reinstalling it"
        );
        return upgrade(bundled, layout, host, None);
    }
    host.stop(Identity::Current)?;
    host.start(Identity::Current)
}

/// Waits up to 45 s for `/health` on the port the daemon in `home`
/// publishes (or `configured_port`) to report `version`.
pub(crate) fn wait_healthy(home: &Path, configured_port: u16, version: &str) -> anyhow::Result<()> {
    const TIMEOUT: Duration = Duration::from_secs(45);
    let deadline = Instant::now() + TIMEOUT;
    let mut seen = None;
    loop {
        // The daemon publishes a negotiated port once it is listening.
        let port = crate::home::runtime_port(home).unwrap_or(configured_port);
        seen = crate::server::probe_health(port, Duration::from_secs(1)).or(seen);
        if seen.as_deref() == Some(version) {
            return Ok(());
        }
        if Instant::now() >= deadline {
            anyhow::bail!(
                "the new daemon did not pass its health check within {}s (expected {version}, saw {})",
                TIMEOUT.as_secs(),
                seen.as_deref().unwrap_or("nothing")
            );
        }
        std::thread::sleep(Duration::from_millis(250));
    }
}

#[cfg(test)]
#[path = "sequence_tests.rs"]
pub(super) mod tests;
