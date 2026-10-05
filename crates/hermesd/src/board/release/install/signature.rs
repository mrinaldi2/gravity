//! Before anything is swapped in (ARCH-R43 M2): the build is signed as the
//! owner's, by the identity compiled into this hermesd (H-110). On macOS,
//! `codesign --verify --strict` against the code requirement; on Windows,
//! Authenticode (WinVerifyTrust, through `Get-AuthenticodeSignature`) and
//! the signer's name. A build that can't be checked says so out loud.

use std::path::Path;
use std::process::Command;

use crate::bus_auth::app_identity::{is_team_id, requirement, DEV_BUILD, TEAM_ID};

use super::Kind;

/// The Windows release's signer (`CN=…`), when Windows builds are signed.
const WINDOWS_SIGNER: Option<&str> = option_env!("HERMES_WINDOWS_SIGNER");

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Check {
    /// Not checked, and why: printed, never silent.
    Skip(String),
    /// An app bundle against the app's requirement.
    MacApp(String),
    /// A bare hermesd against the owner's team.
    MacBinary(String),
    /// Authenticode, signed by this subject.
    Windows(String),
}

/// What this build of hermesd can check for a file of `kind`.
pub(crate) fn plan(
    kind: Kind,
    dev_build: bool,
    team: &str,
    windows_signer: Option<&str>,
    macos: bool,
) -> Check {
    if dev_build {
        return Check::Skip("this hermesd is a dev build, so it doesn't check signatures".into());
    }
    let mac = matches!(kind, Kind::AppZip | Kind::AppDmg) || (kind == Kind::Daemon && macos);
    if mac {
        if !is_team_id(team) {
            return Check::Skip(
                "this hermesd has no Team ID compiled in, so it can't tell the owner's signature"
                    .into(),
            );
        }
        return if kind == Kind::Daemon {
            Check::MacBinary(format!(
                "anchor apple generic and certificate leaf[subject.OU] = \"{team}\""
            ))
        } else {
            Check::MacApp(requirement())
        };
    }
    match windows_signer.filter(|s| !s.is_empty()) {
        Some(signer) if kind == Kind::WindowsSetup || kind == Kind::Daemon => {
            Check::Windows(signer.to_string())
        }
        _ if kind == Kind::WindowsSetup || cfg!(windows) => Check::Skip(
            "this hermesd knows no Windows signer: Windows releases aren't signed yet".into(),
        ),
        _ => Check::Skip("builds for this system aren't signed".into()),
    }
}

/// The check for this build of hermesd.
pub(crate) fn plan_here(kind: Kind) -> Check {
    plan(
        kind,
        DEV_BUILD,
        TEAM_ID,
        WINDOWS_SIGNER,
        cfg!(target_os = "macos"),
    )
}

/// Runs `check` on `path`: what was verified, or the reason it wasn't. An
/// error stops the install.
pub(crate) fn run(check: &Check, path: &Path) -> anyhow::Result<String> {
    match check {
        Check::Skip(why) => Ok(format!("signature check skipped: {why}")),
        Check::MacApp(req) | Check::MacBinary(req) => {
            codesign(path, req)?;
            Ok(format!(
                "signature: {} is signed by the owner's team",
                path.display()
            ))
        }
        Check::Windows(signer) => {
            authenticode(path, signer)?;
            Ok(format!(
                "signature: {} is signed by {signer}",
                path.display()
            ))
        }
    }
}

/// `codesign --verify --strict` (every nested binary too) against `req`.
pub(crate) fn codesign(path: &Path, req: &str) -> anyhow::Result<()> {
    let out = Command::new("codesign")
        .args(["--verify", "--strict", "--deep", "-R"])
        .arg(format!("={req}"))
        .arg(path)
        .output()?;
    anyhow::ensure!(
        out.status.success(),
        "{} isn't signed as the owner's build: {}; not installing it",
        path.display(),
        String::from_utf8_lossy(&out.stderr).trim()
    );
    Ok(())
}

/// Authenticode valid, and signed by `signer`.
fn authenticode(path: &Path, signer: &str) -> anyhow::Result<()> {
    let script = "$s = Get-AuthenticodeSignature -LiteralPath $args[0]; \
                  Write-Output ($s.Status.ToString() + '|' + $s.SignerCertificate.Subject)";
    let out = Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", script])
        .arg(path)
        .output()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let (status, subject) = text.trim().split_once('|').unwrap_or((text.trim(), ""));
    anyhow::ensure!(
        out.status.success() && status == "Valid" && subject.contains(signer),
        "{} isn't signed by {signer} (Authenticode: {status}, signer: {subject}); not installing it",
        path.display()
    );
    Ok(())
}
