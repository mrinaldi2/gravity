//! What may be served and what may be published (CE-010 M1, M2, S1; H-100).
//!
//! The served directory is the one folder `tailscale serve` exposes, so it
//! must never be, hold or sit behind the daemon's home: a root that is a
//! symlink, or whose real path is or contains a home, its secrets,
//! `bus.sqlite` or the daemon config, or is any folder inside a home but
//! the dedicated `<home>/releases`, is refused when the config loads and
//! again at every publish; the owner sees why in Needs you. A build is published only from a `<repo>-rel-*`
//! release worktree in the trusted paths or the project's artifacts (unless
//! the owner configures `source_roots`), only as an `.ipa`, `.zip`, `.dmg`,
//! `.exe` or `.msi`, and is read from one open handle that is no hard link.

use std::fs;
use std::path::{Path, PathBuf};

use crate::bot_permissions::{CONFIG_FILES, HOME_NAMES};
use crate::config::Config;
use crate::decisions::{conflict, forbidden, invalid};

use super::serve::served_root;

/// The only file types a build is published as. The daemon writes the iOS
/// manifest itself; nothing else is ever copied in.
pub const BUILD_TYPES: [&str; 5] = ["ipa", "zip", "dmg", "exe", "msi"];

/// The served directory and the folder beside it where copies are staged
/// before they are linked in, so a half-written file is never served.
#[derive(Debug, Clone)]
pub struct ServedDirs {
    pub root: PathBuf,
    pub staging: PathBuf,
}

/// The staging folder for a served root: `<parent>/.<name>-staging`, on the
/// same volume (hard links need that) and outside what is served.
pub fn staging_for(root: &Path) -> PathBuf {
    let name = root
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    root.with_file_name(format!(".{name}-staging"))
}

/// Why the configured served root must not be served, if it mustn't.
pub fn check_root(cfg: &Config) -> anyhow::Result<()> {
    let root = served_root(cfg);
    let refuse = |why: &str| {
        Err(forbidden(format!(
            "the served directory {} {why}; set [releases] dir to a folder of its own, and pass \
             only that folder to `tailscale serve`",
            root.display()
        )))
    };
    if !root.is_absolute() {
        return refuse("is not an absolute path");
    }
    if fs::symlink_metadata(&root).is_ok_and(|m| m.file_type().is_symlink()) {
        return refuse("is a symlink");
    }
    let spellings = [root.clone(), resolved(&root)];
    for home in homes(cfg) {
        // A folder of its own (H-100): inside a home only the dedicated
        // `<home>/releases` is, never e.g. `<home>/projects`.
        let dedicated = home.join("releases");
        if let Some(inside) = spellings
            .iter()
            .find(|r| r.starts_with(&home) && **r != home && **r != dedicated)
        {
            if !inside.starts_with(home.join("secrets")) {
                return refuse(&format!(
                    "is inside the daemon's home {}; only {} is served from there",
                    home.display(),
                    dedicated.display()
                ));
            }
        }
        let secrets = home.join("secrets");
        let mut sensitive = vec![home.clone(), secrets.clone(), home.join("bus.sqlite")];
        sensitive.extend(CONFIG_FILES.iter().map(|name| home.join(name)));
        for root in &spellings {
            if root.starts_with(&secrets) {
                return refuse("is inside the daemon's secrets");
            }
            if let Some(found) = sensitive.iter().find(|p| p.starts_with(root)) {
                return refuse(&format!("is or contains {}", found.display()));
            }
        }
    }
    Ok(())
}

/// The owner's notice, and Needs you row, when nothing can be served.
pub const SERVING_OFF: &str = "Release builds aren't being served";

/// Why release builds aren't being served, for the owner (H-100): refused
/// when the config loaded, or refused now.
pub fn serving_refused(cfg: &Config) -> Option<String> {
    cfg.releases
        .refused
        .clone()
        .or_else(|| check_root(cfg).err().map(|e| format!("{e:#}")))
}

/// Check the served root once the config is read: a root that must not be
/// served is logged, and nothing is published until the config is fixed.
pub fn check_at_load(cfg: &mut Config) {
    if let Err(e) = check_root(cfg) {
        tracing::error!("not serving release builds: {e:#}");
        cfg.releases.refused = Some(format!("{e:#}"));
    }
}

/// The served root and its staging folder, checked and created as real
/// directories, by their real paths.
pub fn prepare(cfg: &Config) -> anyhow::Result<ServedDirs> {
    if let Some(why) = &cfg.releases.refused {
        return Err(forbidden(why.clone()));
    }
    check_root(cfg)?;
    let root = served_root(cfg);
    let staging = staging_for(&root);
    for dir in [&root, &staging] {
        fs::create_dir_all(dir)?;
        if !fs::symlink_metadata(dir).is_ok_and(|m| m.file_type().is_dir()) {
            return Err(forbidden(format!(
                "{} is not a real directory; builds are only served from one",
                dir.display()
            )));
        }
    }
    // Again, now that both exist: nothing may have been swapped in meanwhile.
    check_root(cfg)?;
    Ok(ServedDirs {
        root: fs::canonicalize(&root)?,
        staging: fs::canonicalize(&staging)?,
    })
}

/// Whether a file name is one of [`BUILD_TYPES`].
pub fn is_build(file_name: &str) -> bool {
    file_name.rsplit_once('.').is_some_and(|(stem, ext)| {
        !stem.is_empty() && BUILD_TYPES.contains(&ext.to_ascii_lowercase().as_str())
    })
}

/// Where builds may be published from: `source_roots` when configured,
/// else the `<repo>-rel-*` release worktrees in the trusted paths; and
/// always the project's artifacts.
pub fn source_roots(cfg: &Config, artifacts: &Path) -> Vec<PathBuf> {
    let configured = &cfg.releases.source_roots;
    let mut roots: Vec<PathBuf> = if configured.is_empty() {
        cfg.trusted_paths
            .iter()
            .flat_map(|dir| fs::read_dir(expand(cfg, dir)).into_iter().flatten())
            .flatten()
            // A symlink posing as a release worktree is not one.
            .filter(|entry| entry.file_type().is_ok_and(|t| t.is_dir()))
            .filter(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .split_once("-rel-")
                    .is_some_and(|(repo, rest)| !repo.is_empty() && !rest.is_empty())
            })
            .map(|entry| entry.path())
            .collect()
    } else {
        configured.iter().map(|r| expand(cfg, r)).collect()
    };
    roots.sort();
    roots.push(artifacts.to_path_buf());
    roots
}

/// A build file to publish, open once: what is hashed and copied is what was
/// checked.
#[derive(Debug)]
pub struct Source {
    /// Its real path.
    pub path: PathBuf,
    pub file: fs::File,
}

/// The build at `file`, which must be a build type and lie in a source root
/// once every symlink in it is resolved.
pub fn source(cfg: &Config, artifacts: &Path, file: &Path) -> anyhow::Result<Source> {
    if !file.is_absolute() {
        return Err(invalid("'file' must be an absolute path"));
    }
    let real = fs::canonicalize(file)
        .map_err(|e| invalid(format!("can't read {}: {e}", file.display())))?;
    let name = real
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default();
    if !is_build(name) {
        return Err(forbidden(format!(
            "{} is not a build: only .{} files are published",
            real.display(),
            BUILD_TYPES.join(", .")
        )));
    }
    let roots = source_roots(cfg, artifacts);
    let real_roots: Vec<PathBuf> = roots
        .iter()
        .filter_map(|r| fs::canonicalize(r).ok())
        .collect();
    if !real_roots.iter().any(|r| real.starts_with(r)) {
        let names: Vec<String> = roots.iter().map(|r| r.display().to_string()).collect();
        return Err(forbidden(format!(
            "{} is outside the roots builds are published from ({})",
            real.display(),
            names.join(", ")
        )));
    }
    Source::open(&real, &real_roots)
}

impl Source {
    /// Open a real path without following a link at its end, and check the
    /// handle: a regular file, not a hard link, the very file the path named
    /// when it was checked (S1), and still at that path, inside `roots`, by
    /// its own account (H-100): a directory on the way swapped for a link
    /// after the checks shows up here.
    pub fn open(real: &Path, roots: &[PathBuf]) -> anyhow::Result<Self> {
        let cant = |e: std::io::Error| invalid(format!("can't open {}: {e}", real.display()));
        let checked = os::path_id(real).map_err(cant)?;
        let file = os::open_no_follow(real).map_err(cant)?;
        let handle = os::describe(&file).map_err(cant)?;
        if !handle.regular {
            return Err(invalid(format!("{} is not a regular file", real.display())));
        }
        if handle.links != 1 {
            return Err(forbidden(format!(
                "{} is a hard link ({} names); publish a copy of its own",
                real.display(),
                handle.links
            )));
        }
        let at = os::real_path(&file).map_err(cant)?;
        if checked.is_some_and(|id| id != handle.id) || at != real {
            return Err(conflict(format!(
                "{} changed while it was checked; publish it again",
                real.display()
            )));
        }
        if !roots.iter().any(|r| at.starts_with(r)) {
            return Err(forbidden(format!(
                "{} is outside the roots builds are published from",
                at.display()
            )));
        }
        Ok(Self {
            path: real.to_path_buf(),
            file,
        })
    }
}

/// A configured path with a leading `~/` as the user's home.
fn expand(cfg: &Config, path: &str) -> PathBuf {
    match path.strip_prefix("~/") {
        Some(rest) => cfg.user_home.join(rest),
        None => PathBuf::from(path),
    }
}

/// The daemon's home under every name it has had, as spelled and resolved.
fn homes(cfg: &Config) -> Vec<PathBuf> {
    let mut homes = vec![cfg.home.clone()];
    homes.extend(HOME_NAMES.iter().map(|name| cfg.user_home.join(name)));
    let real: Vec<PathBuf> = homes.iter().map(|h| resolved(h)).collect();
    homes.extend(real);
    homes.dedup();
    homes
}

/// The real path of `path`: its longest existing ancestor resolved, plus
/// the rest as written.
fn resolved(path: &Path) -> PathBuf {
    for base in path.ancestors() {
        if let Ok(found) = fs::canonicalize(base) {
            return found.join(path.strip_prefix(base).unwrap_or(Path::new("")));
        }
    }
    path.to_path_buf()
}

#[path = "confine_os.rs"]
mod os;

#[cfg(all(test, unix))]
#[path = "confine_tests.rs"]
mod tests;
