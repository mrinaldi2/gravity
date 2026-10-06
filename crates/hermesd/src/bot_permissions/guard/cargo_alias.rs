//! Cargo aliases (H-029, CE-013 G3). A bot may write an `[alias]` into a
//! `.cargo/config.toml` in its own worktree, say `xb = "build --target-dir
//! <another bot's target>"`, and run `cargo xb`. So a subcommand that isn't
//! one of cargo's own is read as the alias the config chain defines, and the
//! guard judges what it expands to. One it can't resolve is refused.

use std::path::PathBuf;

use super::paths::Scope;
use super::GuardContext;

/// Cargo's own subcommands and their short forms, and the common external
/// ones (`cargo-<name>` binaries) that take cargo's options.
const BUILTIN: &[&str] = &[
    "add",
    "b",
    "bench",
    "build",
    "c",
    "check",
    "clean",
    "config",
    "d",
    "doc",
    "fetch",
    "fix",
    "generate-lockfile",
    "help",
    "info",
    "init",
    "install",
    "locate-project",
    "login",
    "logout",
    "metadata",
    "new",
    "owner",
    "package",
    "pkgid",
    "publish",
    "r",
    "read-manifest",
    "remove",
    "report",
    "rm",
    "run",
    "rustc",
    "rustdoc",
    "search",
    "t",
    "test",
    "tree",
    "uninstall",
    "update",
    "vendor",
    "verify-project",
    "version",
    "yank",
    // Common externals, `cargo-<name>` binaries.
    "clippy",
    "fmt",
    "miri",
    "nextest",
    "llvm-cov",
    "audit",
    "deny",
    "expand",
    "tauri",
    "udeps",
    "outdated",
    "machete",
];

/// How deep an alias may refer to another.
pub(super) const MAX_DEPTH: usize = 4;

pub(super) fn is_builtin(sub: &str) -> bool {
    BUILTIN.contains(&sub)
}

/// The config files cargo reads, nearest first: `.cargo/config.toml` (or
/// `config`) above each directory the command runs in, then `$CARGO_HOME`'s.
fn chain(scope: &Scope, ctx: &GuardContext) -> Vec<PathBuf> {
    let mut bases: Vec<PathBuf> = Vec::new();
    for dir in &scope.dirs {
        bases.extend(dir.ancestors().map(|a| a.join(".cargo")));
    }
    let cargo_home = scope
        .vars
        .get("CARGO_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("CARGO_HOME").map(PathBuf::from))
        .unwrap_or_else(|| ctx.user_home.join(".cargo"));
    bases.push(cargo_home);
    let mut files = Vec::new();
    for base in bases {
        for name in ["config.toml", "config"] {
            let file = base.join(name);
            if !files.contains(&file) {
                files.push(file);
            }
        }
    }
    files
}

/// What `sub` expands to as an alias, `Ok(None)` when no config names it,
/// or why it can't be read.
pub(super) fn resolve(
    sub: &str,
    scope: &Scope,
    ctx: &GuardContext,
) -> Result<Option<Vec<String>>, String> {
    for file in chain(scope, ctx) {
        let Ok(text) = std::fs::read_to_string(&file) else {
            continue;
        };
        let value: toml::Value = match toml::from_str(&text) {
            Ok(value) => value,
            Err(_) if !text.contains("alias") => continue,
            Err(error) => {
                return Err(format!(
                    "can't read the cargo aliases in {} ({}); run the cargo command itself",
                    file.display(),
                    error.message()
                ))
            }
        };
        let Some(entry) = value.get("alias").and_then(|a| a.get(sub)) else {
            continue;
        };
        return words_of(entry)
            .map(Some)
            .ok_or_else(|| format!("the alias `{sub}` in {} isn't words", file.display()));
    }
    Ok(None)
}

/// An alias's words as cargo takes them: a string split on whitespace, with
/// no escapes (a Windows path's `\` stays), or a list as it is.
fn words_of(entry: &toml::Value) -> Option<Vec<String>> {
    match entry {
        toml::Value::String(s) => Some(s.split_whitespace().map(str::to_string).collect()),
        toml::Value::Array(list) => list
            .iter()
            .map(|w| w.as_str().map(str::to_string))
            .collect(),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::words_of;

    #[test]
    fn an_alias_keeps_a_windows_path_whole() {
        let config: toml::Value = toml::from_str(
            r"[alias]
xb = 'build --target-dir \\?\C:\Users\me\bots\dev\cargo-target'
xc = ['clean', '--target-dir', 'C:\Users\me\bots\ops\cargo-target']
",
        )
        .unwrap();
        let alias = |name: &str| words_of(&config["alias"][name]).unwrap();
        assert_eq!(
            alias("xb"),
            [
                "build",
                "--target-dir",
                r"\\?\C:\Users\me\bots\dev\cargo-target"
            ]
        );
        assert_eq!(
            alias("xc"),
            [
                "clean",
                "--target-dir",
                r"C:\Users\me\bots\ops\cargo-target"
            ]
        );
        assert!(words_of(&toml::Value::Integer(3)).is_none());
    }
}
