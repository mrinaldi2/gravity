//! `hermesd release install <release> [--dry-run]` (B8, H-020 §2.6 a): the
//! supported way a tester installs a release on their computer.
//!
//! Run from the tester's session, it asks the daemon over the local endpoint
//! (which knows the bot by its process, H-044) for `install_release`, so the
//! gate is checked before anything is touched: the owner's settled approval,
//! an open deploy task for this tester, the frozen hash. Then it takes each
//! build for this computer, from the home's disk or its HTTPS url, checks it
//! against the frozen sha256, and runs the platform's installer. A mismatch
//! stops it. The tester smoke-tests and reports with `deploy_confirm`.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::config::Config;

const USAGE: &str = "usage: hermesd release install <release> [--dry-run]";

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

pub async fn run(cfg: &Config, args: &[String]) -> anyhow::Result<()> {
    let (release, dry_run) = parse(args)?;
    let release = release.as_str();
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
    let work =
        std::env::temp_dir().join(format!("hermes-install-{release}-{}", std::process::id()));
    std::fs::create_dir_all(&work)?;
    for build in mine {
        let file = fetch(build, &work)?;
        verify(&file, &build.sha256).inspect_err(|_| {
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
        if dry_run {
            println!("dry run: would install it as {kind:?}");
            continue;
        }
        install(kind, &file, &work)?;
        println!("{} {} installed", build.platform, build.version);
    }
    let _ = std::fs::remove_dir_all(&work);
    if !dry_run {
        println!(
            "Smoke-test it, then report with deploy_confirm (release {release}, machine {}).",
            answer["machine"].as_str().unwrap_or("this computer")
        );
    }
    Ok(())
}

/// The release and `--dry-run`; the daemon's own `--config <path>` is
/// passed through main and skipped here.
pub(crate) fn parse(args: &[String]) -> anyhow::Result<(String, bool)> {
    let mut release = None;
    let mut dry_run = false;
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--dry-run" => dry_run = true,
            "--config" => {
                it.next();
            }
            a if a.starts_with("--") || release.is_some() => anyhow::bail!("{USAGE}"),
            a => release = Some(a.to_string()),
        }
    }
    Ok((release.ok_or_else(|| anyhow::anyhow!("{USAGE}"))?, dry_run))
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

/// The build's file here: the home's own copy when this is the home, else
/// downloaded from its HTTPS url.
fn fetch(build: &Build, work: &Path) -> anyhow::Result<PathBuf> {
    if let Some(local) = build.artifact.as_deref().map(PathBuf::from) {
        if local.is_file() {
            return Ok(local);
        }
    }
    let url = build
        .url
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("{} has no file here and no url", build.platform))?;
    let name = url
        .rsplit('/')
        .next()
        .filter(|n| !n.is_empty())
        .unwrap_or("build");
    let file = work.join(name);
    run_ok(
        Command::new("curl")
            .args(["-fsSL", "--retry", "2", "-o"])
            .arg(&file)
            .arg(url),
    )?;
    Ok(file)
}

/// The file is the build the owner approved: its sha256 is the frozen one.
pub(crate) fn verify(file: &Path, sha256: &str) -> anyhow::Result<()> {
    let mut hasher = Sha256::new();
    std::io::copy(&mut std::fs::File::open(file)?, &mut hasher)?;
    let got = hex::encode(hasher.finalize());
    anyhow::ensure!(
        !sha256.is_empty() && got == sha256,
        "{} doesn't match the release: sha256 {got}, expected {sha256}; not installing it",
        file.display()
    );
    Ok(())
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

fn install(kind: Kind, file: &Path, work: &Path) -> anyhow::Result<()> {
    match kind {
        Kind::AppZip => {
            let unpacked = work.join("unpacked");
            run_ok(
                Command::new("ditto")
                    .args(["-x", "-k"])
                    .arg(file)
                    .arg(&unpacked),
            )?;
            install_app(&find_app(&unpacked)?)
        }
        Kind::AppDmg => {
            let mount = work.join("mount");
            run_ok(
                Command::new("hdiutil")
                    .args(["attach", "-nobrowse", "-readonly", "-mountpoint"])
                    .arg(&mount)
                    .arg(file),
            )?;
            let installed = find_app(&mount).and_then(|app| install_app(&app));
            let _ = Command::new("hdiutil").arg("detach").arg(&mount).status();
            installed
        }
        // The setup's own hook runs `service install` (installer-hooks.nsh).
        Kind::WindowsSetup => run_ok(Command::new(file).arg("/S")),
        Kind::Daemon => {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(file, std::fs::Permissions::from_mode(0o755))?;
            }
            run_ok(Command::new(file).args(["service", "install"]))
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
/// rename on one volume), then installs the service from the new app.
fn install_app(app: &Path) -> anyhow::Result<()> {
    let name = app.file_name().unwrap_or_default();
    let dest = Path::new("/Applications").join(name);
    let staged = dest.with_extension("app.new");
    let old = dest.with_extension("app.old");
    let _ = std::fs::remove_dir_all(&staged);
    run_ok(Command::new("ditto").arg(app).arg(&staged))?;
    let _ = std::fs::remove_dir_all(&old);
    if dest.exists() {
        std::fs::rename(&dest, &old)?;
    }
    std::fs::rename(&staged, &dest)?;
    let _ = std::fs::remove_dir_all(&old);
    run_ok(Command::new(dest.join("Contents/MacOS/hermesd")).args(["service", "install"]))
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
