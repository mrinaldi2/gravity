//! Serving builds over HTTPS on the tailnet (H-020 §6.6, ruling 7097a9ce).
//!
//! `hermesd release publish` stages a build in the served directory, which
//! `tailscale serve --set-path /releases <dir>` exposes and nothing else.
//! Each build lands at `<dir>/<release id>/<platform>/<file>`, and an iOS
//! build also gets the `manifest.plist` its `itms-services` link names. A
//! published file is never replaced by a different one, and no path is
//! followed through a symlink out of the root it must stay in.

use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::config::Config;
use crate::decisions::{conflict, forbidden, invalid};

use super::confine::{ServedDirs, Source};

/// `[releases]` in `hermesd.toml`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ServeConfig {
    /// The served directory; `<home>/releases` when unset.
    pub dir: Option<PathBuf>,
    /// The HTTPS address that serves `dir`, such as
    /// `https://<mac>.<tailnet>.ts.net/releases`. Nothing is published
    /// without it.
    pub base_url: Option<String>,
    /// Where builds may be published from, besides the project's artifacts
    /// (a leading `~/` is the user's home). When empty, the `<repo>-rel-*`
    /// release worktrees in the trusted paths (CE-010 M2).
    pub source_roots: Vec<String>,
    /// The bundle id an iOS manifest names when the publish gives none.
    pub ios_bundle_id: Option<String>,
    /// The app's name in an iOS manifest; "The Hermes" when unset.
    pub ios_title: Option<String>,
    /// Why `dir` was refused when the config loaded (CE-010 M1).
    #[serde(skip)]
    pub refused: Option<String>,
}

/// The served directory, as configured.
pub fn served_root(cfg: &Config) -> PathBuf {
    cfg.releases
        .dir
        .clone()
        .unwrap_or_else(|| cfg.home.join("releases"))
}

/// The configured HTTPS address of the served directory, without a
/// trailing slash.
pub fn base_url(cfg: &Config) -> anyhow::Result<String> {
    let base = cfg.releases.base_url.as_deref().unwrap_or_default().trim();
    if !base.starts_with("https://") {
        return Err(invalid(format!(
            "set [releases] base_url in hermesd.toml to the https:// address that serves {}; \
             pass only that folder to `tailscale serve`, never the home",
            served_root(cfg).display()
        )));
    }
    Ok(base.trim_end_matches('/').to_string())
}

/// The platform a build file is for, from its extension.
pub fn platform_for(file_name: &str) -> Option<&'static str> {
    let (_, ext) = file_name.rsplit_once('.')?;
    match ext.to_ascii_lowercase().as_str() {
        "ipa" => Some("ios"),
        "zip" | "dmg" => Some("desktop-mac"),
        "exe" | "msi" => Some("desktop-win"),
        _ => None,
    }
}

/// A build placed in the served directory.
#[derive(Debug, Clone, PartialEq)]
pub struct Staged {
    pub path: PathBuf,
    pub url: String,
    pub sha256: String,
    /// False when the same file was already published there.
    pub fresh: bool,
}

/// Copy `src` to `<root>/<release>/<platform>/<its name>`, hashing what
/// is read from its one open handle.
pub fn stage(
    dirs: &ServedDirs,
    base_url: &str,
    release_id: &str,
    platform: &str,
    mut src: Source,
) -> anyhow::Result<Staged> {
    let name = src
        .path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| invalid("the build file has no usable name"))?
        .to_string();
    safe_name("the build's file name", &name)?;
    let dir = served_dir(&dirs.root, release_id, platform)?;
    let path = dir.join(&name);
    let (fresh, sha256) = place(&path, &dirs.staging, |out| {
        io::copy(&mut src.file, out).map(|_| ())
    })?;
    Ok(Staged {
        path,
        url: format!("{base_url}/{release_id}/{platform}/{name}"),
        sha256,
        fresh,
    })
}

/// Write the iOS manifest next to the IPA it installs; returns its URL.
pub fn write_manifest(
    dirs: &ServedDirs,
    ipa: &Staged,
    bundle_id: &str,
    version: &str,
    title: &str,
) -> anyhow::Result<String> {
    let body = manifest(&ipa.url, bundle_id, version, title);
    let dir = ipa.path.parent().expect("staged in a directory");
    place(&dir.join("manifest.plist"), &dirs.staging, |out| {
        out.write_all(body.as_bytes())
    })?;
    let base = ipa.url.rsplit_once('/').expect("a url with a path").0;
    Ok(format!("{base}/manifest.plist"))
}

/// Write the install page (H-229) into a served build's folder.
pub fn write_page(dirs: &ServedDirs, dir: &Path, html: &str) -> anyhow::Result<()> {
    place(&dir.join(super::phone::PAGE), &dirs.staging, |out| {
        out.write_all(html.as_bytes())
    })?;
    Ok(())
}

/// The `itms-services` manifest Safari reads to install an ad-hoc IPA.
pub fn manifest(ipa_url: &str, bundle_id: &str, version: &str, title: &str) -> String {
    let [url, bundle, version, title] = [ipa_url, bundle_id, version, title].map(xml_escape);
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict><key>items</key><array><dict>
<key>assets</key><array><dict><key>kind</key><string>software-package</string><key>url</key><string>{url}</string></dict></array>
<key>metadata</key><dict><key>bundle-identifier</key><string>{bundle}</string><key>bundle-version</key><string>{version}</string><key>kind</key><string>software</string><key>title</key><string>{title}</string></dict>
</dict></array></dict></plist>
"#
    )
}

/// The file at `artifact` when it lies in the served directory, for the
/// install check; `None` for a build that was attached rather than published.
pub fn served_file(cfg: &Config, artifact: &str) -> Option<PathBuf> {
    let root = fs::canonicalize(served_root(cfg)).ok()?;
    let path = Path::new(artifact);
    path.starts_with(&root).then(|| path.to_path_buf())
}

/// Whether a served file still is exactly the build that was frozen. A
/// symlink put in its place never is.
pub fn matches(path: &Path, sha256: &str) -> bool {
    fs::symlink_metadata(path).is_ok_and(|m| m.is_file())
        && sha256_file(path).is_ok_and(|actual| actual == sha256)
}

pub fn sha256_file(path: &Path) -> anyhow::Result<String> {
    let mut file = fs::File::open(path)?;
    let mut hash = Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            return Ok(hex::encode(hash.finalize()));
        }
        hash.update(&buf[..n]);
    }
}

/// `<root>/<release>/<platform>`, created one level at a time so that a
/// symlink anywhere below the root is refused, never followed.
fn served_dir(root: &Path, release_id: &str, platform: &str) -> anyhow::Result<PathBuf> {
    safe_name("the release id", release_id)?;
    safe_name("the platform", platform)?;
    fs::create_dir_all(root)?;
    let mut dir = fs::canonicalize(root)?;
    for part in [release_id, platform] {
        dir.push(part);
        match fs::symlink_metadata(&dir) {
            Ok(m) if m.file_type().is_dir() => {}
            Ok(_) => {
                return Err(forbidden(format!(
                    "{} is a symlink or a file, not a directory; builds are only served from \
                     real directories",
                    dir.display()
                )))
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => fs::create_dir(&dir)?,
            Err(e) => return Err(e.into()),
        }
    }
    Ok(dir)
}

/// Put the file `write` produces at `dest`; returns whether it was written
/// (false when the same file is already there) and its sha256. It is
/// written in `staging`, outside the served tree, and linked in only once
/// complete (S2). A different file, or a symlink, at `dest` is refused: the
/// link goes in without replacing anything, so a concurrent publish can't be
/// overwritten either.
fn place(
    dest: &Path,
    staging: &Path,
    write: impl FnOnce(&mut Hashing) -> io::Result<()>,
) -> anyhow::Result<(bool, String)> {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let name = dest.file_name().and_then(|n| n.to_str()).unwrap_or("build");
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let tmp = staging.join(format!(".{name}.{}.{n}.part", std::process::id()));
    let written = (|| -> anyhow::Result<String> {
        let out = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)?;
        let mut out = Hashing {
            out,
            hash: Sha256::new(),
        };
        write(&mut out)?;
        out.out.sync_all()?;
        let sha256 = hex::encode(out.hash.finalize());
        anyhow::ensure!(
            sha256_file(&tmp)? == sha256,
            "the staged copy changed while it was written; publish it again"
        );
        Ok(sha256)
    })();
    let placed = written.and_then(|sha256| {
        let fresh = if fs::symlink_metadata(dest).is_ok() {
            taken(dest, &sha256)?
        } else {
            match fs::hard_link(&tmp, dest) {
                Ok(()) => true,
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => taken(dest, &sha256)?,
                Err(e) => return Err(e.into()),
            }
        };
        Ok((fresh, sha256))
    });
    let _ = fs::remove_file(&tmp);
    placed
}

/// `dest` is already there: false when it is the same file, else refused.
fn taken(dest: &Path, sha256: &str) -> anyhow::Result<bool> {
    let m = fs::symlink_metadata(dest)?;
    if !m.is_file() {
        return Err(forbidden(format!(
            "{} exists and is not a regular file",
            dest.display()
        )));
    }
    if sha256_file(dest)? == sha256 {
        return Ok(false);
    }
    Err(conflict(format!(
        "{} is already published with different content; publish this build in a new release",
        dest.display()
    )))
}

/// A writer that hashes what it writes.
struct Hashing {
    out: fs::File,
    hash: Sha256,
}

impl Write for Hashing {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let n = self.out.write(buf)?;
        self.hash.update(&buf[..n]);
        Ok(n)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.out.flush()
    }
}

/// A path component that can't climb out, hide, or need URL encoding.
fn safe_name(what: &str, name: &str) -> anyhow::Result<()> {
    let ok = !name.is_empty()
        && name.len() <= 128
        && !name.starts_with('.')
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '+'));
    if ok {
        Ok(())
    } else {
        Err(invalid(format!(
            "{what} '{name}' may use only letters, digits, '.', '_', '-' and '+', and not start \
             with '.'"
        )))
    }
}

pub fn xml_escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

#[cfg(all(test, unix))]
#[path = "serve_tests.rs"]
mod tests;
