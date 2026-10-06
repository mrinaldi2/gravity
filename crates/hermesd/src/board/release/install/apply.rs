//! The app swap, run by the system's install job and never by the bot's
//! session (H-117 X1). `hermesd release install` checks a build in its
//! private stage and hands this step off; the job, outside every session:
//! 1. copies the app beside `/Applications/<name>.app` and checks its
//!    signature there again (that copy is what runs);
//! 2. keeps the app it replaces as the backup, `<home>/backups/app/<name>.app`;
//! 3. swaps the new one in and runs its `service install`;
//! 4. waits for the new service to answer with the new version (the boot
//!    gate) and, if it doesn't, puts the backup back and installs that.
//!
//! A rollback leaves the old version running, so the daemon's own boot
//! check records the install as rolled back (ARCH-R50 S1).

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use crate::config::Config;

use super::{run_ok, signature, Kind};

/// How long the new service has to answer with its version.
const BOOT_GATE: Duration = Duration::from_secs(120);

const USAGE: &str =
    "usage: hermesd release apply-app <release> --app <bundle> --version <v> [--config <path>]";

/// One app swap: the checked bundle in the stage, where apps live, and
/// where the replaced one is kept.
#[derive(Debug, Clone)]
pub(crate) struct AppSwap {
    pub staged: PathBuf,
    pub apps: PathBuf,
    pub backups: PathBuf,
}

impl AppSwap {
    fn name(&self) -> &std::ffi::OsStr {
        self.staged.file_name().unwrap_or_default()
    }

    /// Where the app runs from.
    pub fn dest(&self) -> PathBuf {
        self.apps.join(self.name())
    }

    /// The app it replaced, kept until the next install.
    pub fn backup(&self) -> PathBuf {
        self.backups.join(self.name())
    }

    /// Swaps the staged app in, keeping the one it replaces; returns where
    /// it now is. The copy beside it is checked again: it, not the stage's,
    /// is what runs.
    pub fn swap(&self, check: &signature::Check) -> anyhow::Result<PathBuf> {
        let dest = self.dest();
        let staged = dest.with_extension("app.new");
        let _ = std::fs::remove_dir_all(&staged);
        copy_app(&self.staged, &staged)?;
        if let Err(e) = signature::run(check, &staged) {
            let _ = std::fs::remove_dir_all(&staged);
            return Err(e);
        }
        if dest.exists() {
            std::fs::create_dir_all(&self.backups)?;
            let backup = self.backup();
            let _ = std::fs::remove_dir_all(&backup);
            move_dir(&dest, &backup)?;
        }
        std::fs::rename(&staged, &dest)?;
        Ok(dest)
    }

    /// Puts the backup back in place of the new app; the backup stays.
    pub fn restore(&self) -> anyhow::Result<PathBuf> {
        let dest = self.dest();
        let backup = self.backup();
        anyhow::ensure!(
            backup.exists(),
            "there is no backup of the earlier app at {}",
            backup.display()
        );
        let _ = std::fs::remove_dir_all(&dest);
        copy_app(&backup, &dest)?;
        Ok(dest)
    }
}

/// A bundle copied whole, signatures and attributes kept.
fn copy_app(from: &Path, to: &Path) -> anyhow::Result<()> {
    if cfg!(target_os = "macos") {
        return run_ok(Command::new("ditto").arg(from).arg(to));
    }
    run_ok(Command::new("cp").arg("-R").arg(from).arg(to))
}

/// A rename when both are on one volume, else a copy and a removal.
fn move_dir(from: &Path, to: &Path) -> anyhow::Result<()> {
    if std::fs::rename(from, to).is_ok() {
        return Ok(());
    }
    copy_app(from, to)?;
    std::fs::remove_dir_all(from)?;
    Ok(())
}

/// What the job was asked to do.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Args {
    pub release: String,
    pub app: PathBuf,
    pub version: String,
    pub config: Option<String>,
}

pub(crate) fn parse(args: &[String]) -> anyhow::Result<Args> {
    let (mut release, mut app, mut version, mut config) = (None, None, None, None);
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--app" => app = it.next().map(PathBuf::from),
            "--version" => version = it.next().cloned(),
            "--config" => config = it.next().cloned(),
            a if a.starts_with("--") || release.is_some() => anyhow::bail!("{USAGE}"),
            a => release = Some(a.to_string()),
        }
    }
    let missing = || anyhow::anyhow!("{USAGE}");
    Ok(Args {
        release: release.ok_or_else(missing)?,
        app: app.ok_or_else(missing)?,
        version: version.ok_or_else(missing)?,
        config,
    })
}

/// `service install` from the app at `app`.
fn service_install(app: &Path, config: Option<&str>) -> anyhow::Result<()> {
    let mut command = Command::new(app.join("Contents/MacOS/hermesd"));
    command.args(["service", "install"]);
    if let Some(config) = config {
        command.args(["--config", config]);
    }
    let status = command.status()?;
    anyhow::ensure!(status.success(), "service install exited with {status}");
    Ok(())
}

/// Whether the service answers with `version` within `limit`.
pub(crate) fn answers_with(
    probe: impl Fn() -> Option<String>,
    version: &str,
    limit: Duration,
    every: Duration,
) -> bool {
    let want = version.trim_start_matches('v');
    let started = Instant::now();
    loop {
        if probe().is_some_and(|v| v.trim_start_matches('v') == want) {
            return true;
        }
        if started.elapsed() >= limit {
            return false;
        }
        std::thread::sleep(every);
    }
}

/// The job's step: swap, install, wait for the new service, else roll back.
pub fn run(cfg: &Config, args: &[String]) -> anyhow::Result<()> {
    let args = parse(args)?;
    let swap = AppSwap {
        staged: args.app.clone(),
        apps: PathBuf::from("/Applications"),
        backups: cfg.home.join("backups").join("app"),
    };
    let dest = swap.swap(&signature::plan_here(Kind::AppZip))?;
    println!(
        "{}: swapped in; the earlier app is kept at {}",
        dest.display(),
        swap.backup().display()
    );
    let config = args.config.as_deref();
    let installed = service_install(&dest, config);
    let port = || crate::home::runtime_port(&cfg.home).unwrap_or(cfg.port);
    let probe = || crate::server::probe_health(port(), Duration::from_secs(2));
    if installed.is_ok() && answers_with(probe, &args.version, BOOT_GATE, Duration::from_secs(2)) {
        println!(
            "release {}: the Hermes service runs {}",
            args.release, args.version
        );
        return Ok(());
    }
    let why = match installed {
        Err(e) => format!("its service install failed ({e})"),
        Ok(()) => format!(
            "the new service didn't answer with {} within {} s",
            args.version,
            BOOT_GATE.as_secs()
        ),
    };
    eprintln!(
        "release {}: {why}; putting the earlier app back",
        args.release
    );
    let restored = swap.restore()?;
    service_install(&restored, config)?;
    anyhow::bail!(
        "release {}: rolled back to the earlier app: {why}",
        args.release
    )
}

#[cfg(test)]
#[path = "apply_tests.rs"]
mod tests;
