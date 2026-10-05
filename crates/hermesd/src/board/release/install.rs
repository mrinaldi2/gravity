//! `hermesd release install <release> [--dry-run | --status]` (B8, H-020
//! §2.6 a): the supported way a tester installs a release on their computer.
//!
//! Run from the tester's session, it asks the daemon over the local endpoint
//! (which knows the bot by its process, H-044) for `install_release`, so the
//! gate is checked before anything is touched: the owner's settled approval,
//! an open deploy task for this tester, the frozen hash. For each build for
//! this computer it then (ARCH-R43):
//! 1. copies or downloads it into a fresh private stage and checks its
//!    sha256 there (`stage`);
//! 2. unpacks it and checks its code signature against the identity
//!    compiled into hermesd, saying so when it can't (`signature`);
//! 3. swaps the app into place and hands `service install` to the operating
//!    system (`handoff`), because that restarts this very session.
//!
//! The tester reads the outcome from its next session with `--status`,
//! smoke-tests, and reports with `deploy_confirm`.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use serde_json::{json, Value};

use crate::config::Config;

mod handoff;
mod signature;
mod stage;

#[cfg(test)]
use stage::verify;

const USAGE: &str = "usage: hermesd release install <release> [--dry-run | --status]";

/// How long the gate check may take: forwarded to the home when the board
/// lives on another computer.
const GATE_WAIT: Duration = Duration::from_secs(60);

/// One build of the release, as `install_release` returns it.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Build {
    pub platform: String,
    pub version: String,
    pub artifact: Option<String>,
    pub url: Option<String>,
    pub install_url: Option<String>,
    pub sha256: String,
}

/// What a build file is, and so how it installs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    /// A zipped macOS app bundle: into /Applications, then its service.
    AppZip,
    /// A disk image holding the app bundle.
    AppDmg,
    /// The Windows setup: it installs the app and the service itself.
    WindowsSetup,
    /// A bare `hermesd`: `service install` from it.
    Daemon,
}

/// What the command was asked to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Args {
    pub release: String,
    pub dry_run: bool,
    pub status: bool,
    /// The daemon's own `--config`, passed on to `service install`.
    pub config: Option<String>,
}

pub async fn run(cfg: &Config, args: &[String]) -> anyhow::Result<()> {
    let args = parse(args)?;
    let release = args.release.as_str();
    if args.status {
        return status(cfg, release);
    }
    let answer = gate(cfg, release).await?;
    let builds = builds(&answer);
    for phone in builds.iter().filter(|b| b.platform == "ios") {
        if let Some(url) = &phone.install_url {
            println!("iOS {}: install it on the phone from {url}", phone.version);
        }
    }
    let mine = for_this_computer(&builds, cfg!(target_os = "macos"), cfg!(windows));
    anyhow::ensure!(
        !mine.is_empty(),
        "release {release} has no build for this computer"
    );
    // Every project here pauses first, and what holds the home is reaped;
    // this session is spared until the handoff (H-117 Q4, ARCH-R49 M1).
    if !args.dry_run {
        let version = mine.first().map(|b| b.version.as_str());
        let paused = crate::quiesce::cli::call(cfg, "start", release, version).await?;
        println!("{}", crate::quiesce::cli::describe(&paused));
        anyhow::ensure!(
            paused["proceed"] == true,
            "the install waits until the processes above let go of the home; run it again then"
        );
    }
    for build in mine {
        let stage = stage::Stage::new(release)?;
        let installed = install_one(cfg, &args, build, &stage);
        if args.dry_run || installed.is_err() {
            stage.remove();
        }
        if let Err(e) = installed {
            // Nothing was handed off: every project resumes now.
            if !args.dry_run {
                let _ = crate::quiesce::cli::call(cfg, "resume", release, None).await;
            }
            return Err(e);
        }
    }
    if !args.dry_run {
        println!(
            "The service install runs on its own now and restarts the Hermes service, and this \
             session with it. When you're back: `hermesd release install {release} --status`, \
             smoke-test, then report with deploy_confirm (release {release}, machine {}).",
            answer["machine"].as_str().unwrap_or("this computer")
        );
    }
    Ok(())
}

/// Stage, check, swap in, hand off: one build.
fn install_one(
    cfg: &Config,
    args: &Args,
    build: &Build,
    stage: &stage::Stage,
) -> anyhow::Result<()> {
    let file = stage.fetch(build)?;
    stage::verify(&file, &build.sha256).inspect_err(|_| {
        eprintln!("Report it: deploy_confirm with result \"failed\" and this message.");
    })?;
    let name = file
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    let kind = kind_of(&name, cfg!(target_os = "macos"), cfg!(windows))?;
    println!(
        "{} {}: {name} matches its sha256",
        build.platform, build.version
    );
    let unpacked = unpack(kind, &file, &stage.dir)?;
    println!(
        "{}",
        signature::run(&signature::plan_here(kind), &unpacked)?
    );
    if args.dry_run {
        println!("dry run: would install it as {kind:?}");
        return Ok(());
    }
    let (program, mut service_args) = match kind {
        Kind::AppZip | Kind::AppDmg => (
            swap_app(&unpacked, &signature::plan_here(kind))?.join("Contents/MacOS/hermesd"),
            vec!["service".to_string(), "install".to_string()],
        ),
        // The setup's own hook runs `service install` (installer-hooks.nsh).
        Kind::WindowsSetup => (unpacked, vec!["/S".to_string()]),
        Kind::Daemon => (unpacked, vec!["service".to_string(), "install".to_string()]),
    };
    if kind != Kind::WindowsSetup {
        if let Some(config) = &args.config {
            service_args.extend(["--config".to_string(), config.clone()]);
        }
    }
    let job = handoff::Job::new(&cfg.home, &args.release, program, service_args, &stage.dir);
    handoff::hand_off(&job)?;
    println!(
        "{} {}: handed to the system to install (log: {})",
        build.platform,
        build.version,
        job.log.display()
    );
    Ok(())
}

/// `--status`: how the handed-off install ended.
fn status(cfg: &Config, release: &str) -> anyhow::Result<()> {
    let (code, tail) = handoff::outcome(&cfg.home, release)?;
    match code {
        Some(0) => println!("release {release}: installed\n{tail}"),
        Some(code) => anyhow::bail!("release {release}: the install failed ({code})\n{tail}"),
        None => println!("release {release}: still installing\n{tail}"),
    }
    Ok(())
}

/// The release and its flags; the daemon's own `--config <path>` comes
/// through main and is passed on.
pub(crate) fn parse(args: &[String]) -> anyhow::Result<Args> {
    let mut release = None;
    let (mut dry_run, mut status, mut config) = (false, false, None);
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--dry-run" => dry_run = true,
            "--status" => status = true,
            "--config" => config = it.next().cloned(),
            a if a.starts_with("--") || release.is_some() => anyhow::bail!("{USAGE}"),
            a => release = Some(a.to_string()),
        }
    }
    anyhow::ensure!(!(dry_run && status), "{USAGE}");
    Ok(Args {
        release: release.ok_or_else(|| anyhow::anyhow!("{USAGE}"))?,
        dry_run,
        status,
        config,
    })
}

/// `install_release` as the bot whose session this runs in: the gate.
async fn gate(cfg: &Config, release: &str) -> anyhow::Result<Value> {
    let endpoint = crate::bus_auth::ipc::endpoint(cfg).display().to_string();
    let request = json!({
        "jsonrpc": "2.0", "id": 1, "method": "tools/call",
        "params": { "name": "install_release", "arguments": { "release_id": release } },
    });
    let reply = crate::bus_auth::hook::send(&endpoint, &request, false, GATE_WAIT).await?;
    if let Some(error) = reply.get("error") {
        anyhow::bail!(
            "{} (run this from the tester's own session)",
            error["message"].as_str().unwrap_or("refused")
        );
    }
    let text = reply["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or_default();
    anyhow::ensure!(reply["result"]["isError"] != json!(true), "refused: {text}");
    Ok(serde_json::from_str(text)?)
}

pub(crate) fn builds(answer: &Value) -> Vec<Build> {
    let text = |b: &Value, key: &str| {
        b[key]
            .as_str()
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    };
    answer["builds"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|b| Build {
            platform: text(b, "platform").unwrap_or_default(),
            version: text(b, "version").unwrap_or_default(),
            artifact: text(b, "artifact"),
            url: text(b, "url"),
            install_url: text(b, "install_url"),
            sha256: text(b, "sha256").unwrap_or_default().to_ascii_lowercase(),
        })
        .collect()
}

/// The builds this computer installs: the desktop app where there is one
/// (it carries the daemon), else the daemon alone; never the phone's.
pub(crate) fn for_this_computer(builds: &[Build], macos: bool, windows: bool) -> Vec<&Build> {
    let of = |platform: &str| -> Vec<&Build> {
        builds.iter().filter(|b| b.platform == platform).collect()
    };
    let desktop = of("desktop");
    if (macos || windows) && !desktop.is_empty() {
        desktop
    } else {
        of("daemon")
    }
}

pub(crate) fn kind_of(name: &str, macos: bool, windows: bool) -> anyhow::Result<Kind> {
    let lower = name.to_ascii_lowercase();
    if lower.starts_with("hermesd") {
        return Ok(Kind::Daemon);
    }
    let kind = match () {
        () if macos && lower.ends_with(".zip") => Kind::AppZip,
        () if macos && lower.ends_with(".dmg") => Kind::AppDmg,
        () if windows && lower.ends_with(".exe") => Kind::WindowsSetup,
        () => anyhow::bail!(
            "don't know how to install {name} on this computer; install it by hand and report \
             with deploy_confirm"
        ),
    };
    Ok(kind)
}

/// What gets checked and installed, inside the stage: the app bundle out
/// of a zip or a disk image, or the file itself.
fn unpack(kind: Kind, file: &Path, stage: &Path) -> anyhow::Result<PathBuf> {
    match kind {
        Kind::AppZip => {
            let unpacked = stage.join("unpacked");
            run_ok(
                Command::new("ditto")
                    .args(["-x", "-k"])
                    .arg(file)
                    .arg(&unpacked),
            )?;
            find_app(&unpacked)
        }
        Kind::AppDmg => {
            let mount = stage.join("mount");
            run_ok(
                Command::new("hdiutil")
                    .args(["attach", "-nobrowse", "-readonly", "-mountpoint"])
                    .arg(&mount)
                    .arg(file),
            )?;
            // A copy in the stage, so the image can go before the swap.
            let copied = find_app(&mount).and_then(|app| {
                let to = stage
                    .join("unpacked")
                    .join(app.file_name().unwrap_or_default());
                run_ok(Command::new("ditto").arg(&app).arg(&to)).map(|()| to)
            });
            let _ = Command::new("hdiutil").arg("detach").arg(&mount).status();
            copied
        }
        Kind::WindowsSetup => Ok(file.to_path_buf()),
        Kind::Daemon => {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(file, std::fs::Permissions::from_mode(0o700))?;
            }
            Ok(file.to_path_buf())
        }
    }
}

fn find_app(dir: &Path) -> anyhow::Result<PathBuf> {
    std::fs::read_dir(dir)?
        .flatten()
        .map(|e| e.path())
        .find(|p| p.extension().is_some_and(|e| e == "app"))
        .ok_or_else(|| anyhow::anyhow!("no app bundle in {}", dir.display()))
}

/// Replaces `/Applications/<name>.app` (staged beside it, so the swap is a
/// rename on one volume) and returns where it now is. The copy beside it
/// is checked again: it, not the stage's, is what runs.
fn swap_app(app: &Path, check: &signature::Check) -> anyhow::Result<PathBuf> {
    let name = app.file_name().unwrap_or_default();
    let dest = Path::new("/Applications").join(name);
    let staged = dest.with_extension("app.new");
    let old = dest.with_extension("app.old");
    let _ = std::fs::remove_dir_all(&staged);
    run_ok(Command::new("ditto").arg(app).arg(&staged))?;
    if let Err(e) = signature::run(check, &staged) {
        let _ = std::fs::remove_dir_all(&staged);
        return Err(e);
    }
    let _ = std::fs::remove_dir_all(&old);
    if dest.exists() {
        std::fs::rename(&dest, &old)?;
    }
    std::fs::rename(&staged, &dest)?;
    let _ = std::fs::remove_dir_all(&old);
    Ok(dest)
}

fn run_ok(command: &mut Command) -> anyhow::Result<()> {
    let out = command.output()?;
    anyhow::ensure!(
        out.status.success(),
        "{:?} failed: {}",
        command.get_program(),
        String::from_utf8_lossy(&out.stderr).trim()
    );
    Ok(())
}

#[cfg(test)]
#[path = "install_tests.rs"]
mod tests;
