//! The app swap, run by the system's install job and never by the bot's
//! session (H-117 X1). `hermesd release install` checks a build in its
//! private stage and hands this step off; the job, outside every session:
//! 1. copies the app beside `/Applications/<name>.app` and checks its
//!    signature there again (that copy is what runs);
//! 2. keeps the app it replaces as the backup, `<home>/backups/app/<name>.app`,
//!    and remembers the backup's hash itself, in memory, since that folder
//!    is any bot's to write (ARCH-R52 M2);
//! 3. swaps the new one in and runs its `service install`;
//! 4. waits for the service to answer from the new binary, by its sha256,
//!    not its version, which a same-version reinstall's old daemon also has
//!    (the boot gate, ARCH-R52 M3). If it doesn't, the backup goes back, but
//!    only once its hash and signature check out, and its service install
//!    runs.
//!
//! If putting the backup back fails too, a daemon is still brought up from
//! `<home>/bin` and a marker left for it, so the owner hears of it at boot
//! with a Run card (ARCH-R52 S3). A rollback leaves the old version running,
//! so the daemon's own boot check records the install as rolled back.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use serde_json::json;
use sha2::{Digest, Sha256};

use crate::config::Config;

use super::{run_ok, signature, Kind};

/// How long the new service has to answer from its binary.
const BOOT_GATE: Duration = Duration::from_secs(120);

/// Left in `<home>/run` when the rollback itself failed, for the boot.
pub(crate) const FAILED_MARKER: &str = "rollback-failed.json";

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

/// What a swap did: where the app now is, and the hash of the one it kept.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Swapped {
    pub dest: PathBuf,
    pub backup_sha256: Option<String>,
}

/// The sha256 over a bundle's every file: relative paths, link targets and
/// contents, in a fixed order.
pub(crate) fn bundle_sha256(root: &Path) -> anyhow::Result<String> {
    fn walk(root: &Path, dir: &Path, h: &mut Sha256) -> anyhow::Result<()> {
        let mut entries: Vec<_> = std::fs::read_dir(dir)?
            .flatten()
            .map(|e| e.path())
            .collect();
        entries.sort();
        for path in entries {
            let rel = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .to_string_lossy()
                .into_owned();
            let meta = std::fs::symlink_metadata(&path)?;
            if meta.file_type().is_symlink() {
                h.update(format!(
                    "link\0{rel}\0{}\n",
                    std::fs::read_link(&path)?.display()
                ));
            } else if meta.is_dir() {
                h.update(format!("dir\0{rel}\n"));
                walk(root, &path, h)?;
            } else {
                h.update(format!("file\0{rel}\0"));
                h.update(std::fs::read(&path)?);
            }
        }
        Ok(())
    }
    let mut h = Sha256::new();
    walk(root, root, &mut h)?;
    Ok(hex::encode(h.finalize()))
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

    /// Swaps the staged app in, keeping the one it replaces. The copy beside
    /// it is checked again: it, not the stage's, is what runs.
    pub fn swap(&self, check: &signature::Check) -> anyhow::Result<Swapped> {
        let dest = self.dest();
        let staged = dest.with_extension("app.new");
        let _ = std::fs::remove_dir_all(&staged);
        copy_app(&self.staged, &staged)?;
        if let Err(e) = signature::run(check, &staged) {
            let _ = std::fs::remove_dir_all(&staged);
            return Err(e);
        }
        let mut backup_sha256 = None;
        if dest.exists() {
            backup_sha256 = Some(bundle_sha256(&dest)?);
            std::fs::create_dir_all(&self.backups)?;
            let backup = self.backup();
            let _ = std::fs::remove_dir_all(&backup);
            move_dir(&dest, &backup)?;
        }
        std::fs::rename(&staged, &dest)?;
        Ok(Swapped {
            dest,
            backup_sha256,
        })
    }

    /// Puts the backup back in place of the new app, once it is the app
    /// that was kept (`expected`, its hash then) and still signed as the
    /// owner's; the backup stays. Refused, the new app stays in place.
    pub fn restore(&self, check: &signature::Check, expected: &str) -> anyhow::Result<PathBuf> {
        let dest = self.dest();
        let backup = self.backup();
        anyhow::ensure!(
            backup.exists(),
            "there is no backup of the earlier app at {}",
            backup.display()
        );
        let restoring = dest.with_extension("app.restore");
        let _ = std::fs::remove_dir_all(&restoring);
        copy_app(&backup, &restoring)?;
        let checked = bundle_sha256(&restoring).and_then(|got| {
            anyhow::ensure!(
                got == expected,
                "the kept app changed since it was kept, so it isn't put back"
            );
            signature::run(check, &restoring)
        });
        if let Err(e) = checked {
            let _ = std::fs::remove_dir_all(&restoring);
            return Err(e);
        }
        let _ = std::fs::remove_dir_all(&dest);
        std::fs::rename(&restoring, &dest)?;
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

/// `service install` from the hermesd at `binary`.
fn service_install(binary: &Path, config: Option<&str>) -> anyhow::Result<()> {
    let mut command = Command::new(binary);
    command.args(["service", "install"]);
    if let Some(config) = config {
        command.args(["--config", config]);
    }
    let status = command.status()?;
    anyhow::ensure!(status.success(), "service install exited with {status}");
    Ok(())
}

fn app_binary(app: &Path) -> PathBuf {
    app.join("Contents/MacOS/hermesd")
}

/// Whether the service answers from the binary hashed `want` within
/// `limit`: by its sha256, which tells the new daemon from the old.
pub(crate) fn answers_from(
    probe: impl Fn() -> Option<String>,
    want: &str,
    limit: Duration,
    every: Duration,
) -> bool {
    let started = Instant::now();
    loop {
        if probe().is_some_and(|sha| sha == want) {
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
    let check = signature::plan_here(Kind::AppZip);
    let swapped = swap.swap(&check)?;
    let dest = swapped.dest.clone();
    println!(
        "{}: swapped in; the earlier app is kept at {}",
        dest.display(),
        swap.backup().display()
    );
    let want = crate::quiesce::file_sha256(&app_binary(&dest))?;
    let config = args.config.as_deref();
    let installed = service_install(&app_binary(&dest), config);
    let port = || crate::home::runtime_port(&cfg.home).unwrap_or(cfg.port);
    let probe = || crate::server::probe_binary(port(), Duration::from_secs(2));
    if installed.is_ok() && answers_from(probe, &want, BOOT_GATE, Duration::from_secs(2)) {
        println!(
            "release {}: the Hermes service runs the new {}",
            args.release, args.version
        );
        return Ok(());
    }
    let why = match installed {
        Err(e) => format!("its service install failed ({e})"),
        Ok(()) => format!(
            "the new service didn't answer from its binary within {} s",
            BOOT_GATE.as_secs()
        ),
    };
    eprintln!(
        "release {}: {why}; putting the earlier app back",
        args.release
    );
    let rolled_back = swapped
        .backup_sha256
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("there was no earlier app to put back"))
        .and_then(|kept| swap.restore(&check, kept))
        .and_then(|restored| service_install(&app_binary(&restored), config));
    if let Err(e) = rolled_back {
        return Err(rollback_failed(
            cfg,
            &args,
            &dest,
            &format!("{why}; then {e}"),
        ));
    }
    anyhow::bail!(
        "release {}: rolled back to the earlier app: {why}",
        args.release
    )
}

/// The rollback failed too: a daemon is brought up from `<home>/bin`, the
/// last one installed, else from the app in place, and a marker tells it to
/// tell the owner (ARCH-R52 S3).
fn rollback_failed(cfg: &Config, args: &Args, app: &Path, why: &str) -> anyhow::Error {
    let marker = json!({ "release": args.release, "version": args.version,
                         "app": app.display().to_string(), "why": why });
    let run = cfg.home.join("run");
    let _ = std::fs::create_dir_all(&run);
    let _ = std::fs::write(run.join(FAILED_MARKER), marker.to_string());
    let installed = cfg.home.join("bin").join(crate::brand::daemon_file(""));
    let fallback = if installed.is_file() {
        installed
    } else {
        app_binary(app)
    };
    let started = service_install(&fallback, args.config.as_deref());
    anyhow::anyhow!(
        "release {}: the install failed and so did putting the earlier app back ({why}); \
         started the service from {} ({})",
        args.release,
        fallback.display(),
        match started {
            Ok(()) => "it is starting".to_string(),
            Err(e) => format!("that failed too: {e}"),
        }
    )
}

#[cfg(test)]
#[path = "apply_tests.rs"]
mod tests;
