//! How the guard reads a path the way the shell and the kernel will (CE-003
//! M3, M4): `~`, `~user`, `$HOME` and variables set earlier on the line are
//! expanded; `//`, `/./` and `..` collapse; a relative word is resolved
//! against every directory a `cd` on the line may have moved to; symlinks
//! that exist are followed, and ones made earlier on the line are tracked;
//! a glob is judged by every path it could match.

use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};

use super::path_key::{git_bash_drive, is_null_device, last_separator};
pub use super::path_key::{key, within};
use super::words::pieces;
use super::GuardContext;

/// What one command line has set up so far, for the commands after it.
#[derive(Clone)]
pub(super) struct Scope {
    /// Every directory a command may run in: the call's cwd plus each
    /// `cd`/`pushd` target. A subshell's `cd` may not stick, so none of
    /// them replaces another.
    pub dirs: Vec<PathBuf>,
    /// `name=value` assignments seen so far.
    pub vars: HashMap<String, String>,
    /// `ln -s target link` seen so far: (link, target).
    pub links: Vec<(PathBuf, PathBuf)>,
    /// The line runs a `$( )` or backtick substitution.
    pub substitutes: bool,
    /// Every path-like argument on the line, for `xargs`.
    pub named: Vec<String>,
    /// `for name in a b c`: each value the loop variable takes.
    pub lists: HashMap<String, Vec<String>>,
}

/// More directories than this means the line is playing games.
const MAX_DIRS: usize = 16;

/// Characters that make a word match more than its own spelling. `$` is a
/// variable the guard could not expand.
const WILD: &[char] = &['*', '?', '[', '{', '$'];

impl Scope {
    pub fn new(cwd: &Path) -> Self {
        Self {
            dirs: vec![cwd.to_path_buf()],
            vars: HashMap::new(),
            links: Vec::new(),
            substitutes: false,
            named: Vec::new(),
            lists: HashMap::new(),
        }
    }

    pub fn enter(&mut self, dirs: Vec<PathBuf>) {
        for dir in dirs {
            if !self.dirs.contains(&dir) && self.dirs.len() < MAX_DIRS {
                self.dirs.push(dir);
            }
        }
    }
}

/// `a/./b/../c` → `a/c`, without touching the disk.
pub fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for part in path.components() {
        match part {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out
}

/// The path the kernel would reach: the longest prefix that exists, with
/// its symlinks and `..` resolved on disk, plus the rest normalized.
pub fn real(path: &Path) -> PathBuf {
    for base in path.ancestors() {
        if let Ok(found) = base.canonicalize() {
            let rest = path.strip_prefix(base).unwrap_or(Path::new(""));
            return normalize(&found.join(rest));
        }
    }
    normalize(path)
}

impl GuardContext {
    /// The paths no tool call may touch.
    pub(in crate::bot_permissions) fn protected(&self) -> Vec<PathBuf> {
        let mut lexical = vec![
            self.home.join("secrets"),
            self.home.join("bot-settings.json"),
            self.user_home.join(".ssh"),
            self.user_home.join(".claude.json"),
            self.user_home.join(".claude").join("settings.json"),
            self.user_home.join(".claude").join("settings.local.json"),
        ];
        lexical.extend(
            crate::bot_permissions::CONFIG_FILES
                .iter()
                .map(|name| self.home.join(name)),
        );
        // The same paths spelled through a sibling link to the home
        // (`~/.gravity` → `~/.thehermes` after the move).
        let aliases: Vec<PathBuf> = self
            .home_links()
            .iter()
            .flat_map(|link| {
                lexical
                    .iter()
                    .filter_map(|p| p.strip_prefix(&self.home).ok())
                    .map(|rest| link.join(rest))
                    .collect::<Vec<_>>()
            })
            .collect();
        lexical.extend(aliases);
        let mut all: Vec<PathBuf> = lexical.iter().map(|p| real(p)).collect();
        all.extend(lexical);
        all
    }

    /// The home's other names beside it that are symlinks to it.
    fn home_links(&self) -> Vec<PathBuf> {
        let Some(parent) = self.home.parent() else {
            return Vec::new();
        };
        let home = real(&self.home);
        crate::bot_permissions::HOME_NAMES
            .iter()
            .map(|name| parent.join(name))
            .filter(|link| *link != self.home)
            .filter(|link| {
                link.symlink_metadata()
                    .is_ok_and(|m| m.file_type().is_symlink())
            })
            .filter(|link| real(link) == home)
            .collect()
    }

    /// Any of these in a path makes it protected wherever it lives.
    const PROTECTED_NAMES: [&'static str; 3] = [
        ".claude/settings.json",
        ".claude/settings.local.json",
        "settings.gen.json",
    ];

    fn is_protected(&self, path: &Path) -> Option<String> {
        let text = path.display().to_string().replace('\\', "/");
        self.protected()
            .into_iter()
            .find(|p| path.starts_with(p) || within(path, p))
            .map(|p| p.display().to_string())
            .or_else(|| {
                Self::PROTECTED_NAMES
                    .iter()
                    .find(|name| key(Path::new(&text)).contains(&key(Path::new(name))))
                    .map(|name| (*name).to_string())
            })
    }

    /// The paths a word names, resolved from each directory in `scope`:
    /// lexically (with the line's links followed) and on disk. A word with
    /// wildcards keeps them as literal components here.
    pub(super) fn candidates(&self, expanded: &str, scope: &Scope) -> Vec<PathBuf> {
        let drive = git_bash_drive(expanded);
        let word = Path::new(drive.as_deref().unwrap_or(expanded));
        let dirs: Vec<&Path> = if word.is_absolute() {
            vec![Path::new("/")]
        } else {
            scope.dirs.iter().map(PathBuf::as_path).collect()
        };
        let mut out = Vec::new();
        for dir in dirs {
            let raw = dir.join(word);
            let found = [real(&normalize(&raw)), real(&raw)];
            // A link made earlier on the line doesn't exist on disk yet.
            for path in &found {
                for (link, target) in &scope.links {
                    if let Ok(rest) = path.strip_prefix(link) {
                        out.push(real(&target.join(rest)));
                    }
                }
            }
            out.extend(found);
        }
        out
    }

    /// The protected path a word (or any path-like piece of it) names.
    pub(super) fn protected_word(&self, word: &str, scope: &Scope) -> Option<String> {
        // Whole words first: a verbatim drive path is unwrapped, and a device
        // path refused, before `:` splits the word into pieces (WIN-CHK-13).
        let words: Vec<String> = Self::alternatives(word, scope, 4)
            .iter()
            .map(|w| self.expand(w, scope))
            .collect();
        // A device path may name any file, a protected one included.
        if let Some(device) = words.iter().find(|w| super::words::is_device_path(w)) {
            return Some(device.clone());
        }
        let found = words.iter().flat_map(|w| pieces(w)).find_map(|piece| {
            let expanded = self.expand(piece, scope);
            self.candidates(&expanded, scope)
                .iter()
                .find_map(|p| self.is_protected(p))
                .or_else(|| self.glob_reaches_protected(&expanded, scope))
        });
        found
    }

    /// The protected path inside the folder a word names, when there is one.
    pub(super) fn holds_protected(&self, word: &str, scope: &Scope) -> Option<String> {
        let expanded = self.expand(word, scope);
        let protected = self.protected();
        self.candidates(&expanded, scope).iter().find_map(|dir| {
            protected
                .iter()
                .find(|p| (p.starts_with(dir) || within(p, dir)) && key(p) != key(dir))
                .map(|p| p.display().to_string())
        })
    }

    /// `~/.gravity/sec*/x` reaches the secrets: the literal part before the
    /// first wildcard is a prefix of a protected path. The folder part of
    /// that prefix is read both as spelled and resolved like a plain path,
    /// so a glob through a symlink (`~/.gravity` → `~/.thehermes`) is judged
    /// by where it lands (CE-006 G1). A wildcard at the start of a name never
    /// matches a dot file.
    fn glob_reaches_protected(&self, expanded: &str, scope: &Scope) -> Option<String> {
        let at = expanded.find(WILD)?;
        let (prefix, wild) = expanded.split_at(at);
        let (folder, partial) = match last_separator(prefix) {
            Some(i) => (&prefix[..=i], &prefix[i + 1..]),
            None => ("", prefix),
        };
        let protected = self.protected();
        let rooted = Path::new(prefix).has_root() || Path::new(prefix).is_absolute();
        let dirs: Vec<PathBuf> = if rooted {
            vec![PathBuf::from("/")]
        } else {
            scope.dirs.clone()
        };
        let mut folders: Vec<PathBuf> = dirs.iter().map(|d| normalize(&d.join(folder))).collect();
        folders.extend(self.candidates(if folder.is_empty() { "." } else { folder }, scope));
        folders.iter().find_map(|dir| {
            let base = format!("{}/{partial}", key(dir)).replace("//", "/");
            let base = key(Path::new(&base));
            protected.iter().find_map(|p| {
                let rest = key(p).strip_prefix(&base)?.to_string();
                let hidden = base.ends_with('/') && rest.starts_with('.') && !wild.starts_with('.');
                (!hidden).then(|| p.display().to_string())
            })
        })
    }

    /// Whether a destructive command may act on `path` (already real).
    pub(super) fn may_change(&self, path: &Path) -> bool {
        if path == Path::new("/dev/null")
            || path.starts_with("/dev/fd")
            || matches!(path.to_str(), Some("/dev/stdout" | "/dev/stderr"))
        {
            return true;
        }
        if self.in_served(path) || self.in_run(path) {
            return false;
        }
        let inside = |root: &PathBuf| path.starts_with(real(root));
        self.writable.iter().any(inside)
            || self.worktrees.iter().any(|root| {
                path.strip_prefix(real(root)).is_ok_and(|rest| {
                    rest.components().next().is_some_and(|first| {
                        self.owns_worktree(&first.as_os_str().to_string_lossy())
                    })
                })
            })
    }

    /// Whether a folder in the trusted paths is one of this bot's worktrees:
    /// `<repo>-wt-<slug>[-…]` (any `-wt-` when the slug is unknown), or a
    /// `<repo>-rel-*` release worktree for a bot that publishes.
    fn owns_worktree(&self, folder: &str) -> bool {
        let own = folder.split_once("-wt-").is_some_and(|(repo, rest)| {
            !repo.is_empty()
                && self.bot_slug.as_deref().is_none_or(|slug| {
                    rest.strip_prefix(slug)
                        .is_some_and(|tail| tail.is_empty() || tail.starts_with('-'))
                })
        });
        let release = self.releases
            && folder
                .split_once("-rel-")
                .is_some_and(|(repo, rest)| !repo.is_empty() && !rest.is_empty());
        own || release
    }

    /// `Ok` when a destructive command may act on every path `word` names;
    /// otherwise the first path it may not.
    pub(super) fn may_change_word(&self, word: &str, scope: &Scope) -> Result<(), PathBuf> {
        Self::alternatives(word, scope, 4)
            .iter()
            .try_for_each(|word| self.may_change_one(word, scope))
    }

    fn may_change_one(&self, word: &str, scope: &Scope) -> Result<(), PathBuf> {
        let expanded = self.expand(word, scope);
        // Resolved, Windows would read `/dev/null` as `C:\dev\null`.
        if is_null_device(&expanded) {
            return Ok(());
        }
        // A device path is never one of the bot's own folders (CE-015 M2).
        if super::words::is_device_path(&expanded) {
            return Err(PathBuf::from(&expanded));
        }
        if let Some(at) = expanded.find(WILD) {
            let (prefix, wild) = expanded.split_at(at);
            // A computed path that may start anywhere.
            if prefix.is_empty() && wild.starts_with('$') && self.full {
                return Err(PathBuf::from(&expanded));
            }
            // `*/..` climbs out of wherever the wildcard landed.
            if Path::new(wild)
                .components()
                .any(|c| c == Component::ParentDir)
            {
                return Err(PathBuf::from(&expanded));
            }
            let parent = match last_separator(prefix) {
                Some(0) => "/",
                Some(i) => &prefix[..i],
                None => ".",
            };
            let partial = &prefix[last_separator(prefix).map_or(0, |i| i + 1)..];
            for dir in self.candidates(parent, scope) {
                let reach = if partial.is_empty() {
                    dir
                } else {
                    dir.join(partial)
                };
                if !self.may_change(&reach) {
                    return Err(reach);
                }
            }
        }
        match self
            .candidates(&expanded, scope)
            .into_iter()
            .find(|p| !self.may_change(p))
        {
            Some(path) => Err(path),
            None => Ok(()),
        }
    }

    /// Where a Write or Edit may land in Full: the bot's folders, plus
    /// Claude Code's own per-project memory and plans.
    pub(super) fn may_write_file(&self, file: &str, scope: &Scope) -> Result<(), PathBuf> {
        let claude = self.user_home.join(".claude");
        let own = [claude.join("projects"), claude.join("plans")];
        match self.may_change_word(file, scope) {
            Err(path) if !own.iter().any(|dir| path.starts_with(real(dir))) => Err(path),
            _ => Ok(()),
        }
    }

    /// A directory word (`cd`, `git -C`) as the directories it may name.
    pub(super) fn resolve(&self, scope: &Scope, word: &str) -> Vec<PathBuf> {
        if word.is_empty() || word == "-" {
            return vec![self.user_home.clone()];
        }
        let expanded = self.expand(word, scope);
        let literal = match expanded.find(WILD) {
            Some(at) => &expanded[..at],
            None => expanded.as_str(),
        };
        if literal.is_empty() && expanded.starts_with('$') {
            return vec![self.user_home.clone()];
        }
        let mut dirs = self.candidates(literal, scope);
        dirs.dedup();
        dirs
    }
}
