//! Where `cargo` builds and deletes (H-029, CE-013). Every bot builds into
//! its own `CARGO_TARGET_DIR`, so a command naming any other target could
//! wipe another bot's build, or plant a binary in DevOps's that ships. For
//! every subcommand that writes a target, each place the line names one
//! must be the bot's own:
//! - `--target-dir` (and `--out-dir`, `--artifact-dir`, `install --root`);
//! - `--config build.target-dir=…` (a `--config` file can't be read here,
//!   so it is refused);
//! - `CARGO_TARGET_DIR` / `CARGO_BUILD_TARGET_DIR` assigned on the line, by
//!   prefix, `env` or an earlier `export`;
//! - a `.cargo/config.toml` above the directory it runs in that sets
//!   `target-dir`.
//!
//! A value the guard can't resolve (an unknown variable, a substitution)
//! is refused. With none named, cargo uses the session's own target.

use std::path::{Path, PathBuf};

use super::paths::Scope;
use super::{cargo_alias, GuardContext};

/// Subcommands that build or change nothing in a target.
const READ_ONLY: &[&str] = &[
    "metadata",
    "tree",
    "version",
    "help",
    "search",
    "locate-project",
    "pkgid",
    "verify-project",
    "read-manifest",
    "info",
    "fmt",
];

/// Global options that take a value.
const VALUED: &[&str] = &["--color", "-Z", "--config", "-C", "--explain"];

/// The variables cargo reads its target from.
const VARS: [&str; 2] = ["CARGO_TARGET_DIR", "CARGO_BUILD_TARGET_DIR"];

/// Options naming a directory cargo writes to.
const DIRS: &[&str] = &["--target-dir", "--out-dir", "--artifact-dir"];

/// Why this `cargo` line must not run, or `None`.
pub(super) fn check(
    words: &[String],
    at: usize,
    scope: &Scope,
    ctx: &GuardContext,
) -> Option<String> {
    check_at(words, at, scope, ctx, 0)
}

/// `cargo-<sub> [<sub>] args…`, a subcommand's binary run directly, judged
/// as `cargo <sub> args…` (CE-013).
pub(super) fn check_binary(
    words: &[String],
    at: usize,
    sub: &str,
    scope: &Scope,
    ctx: &GuardContext,
) -> Option<String> {
    let mut rest = &words[at + 1..];
    if rest.first().is_some_and(|w| w == sub) {
        rest = &rest[1..];
    }
    let mut as_cargo = words[..at].to_vec();
    as_cargo.push("cargo".to_string());
    as_cargo.push(sub.to_string());
    as_cargo.extend_from_slice(rest);
    check_at(&as_cargo, at, scope, ctx, 0)
}

fn check_at(
    words: &[String],
    at: usize,
    scope: &Scope,
    ctx: &GuardContext,
    depth: usize,
) -> Option<String> {
    let rest = &words[at + 1..];
    let (index, sub) = subcommand(rest)?;
    if words[..at]
        .iter()
        .chain(scope.vars.keys())
        .any(|w| w.starts_with("CARGO_ALIAS_"))
    {
        return Some(
            "a CARGO_ALIAS_ variable can make any cargo command build anywhere; run the \
             command itself"
                .to_string(),
        );
    }
    // An alias is judged by what it expands to (G3).
    if !cargo_alias::is_builtin(sub) {
        if depth >= cargo_alias::MAX_DEPTH {
            return Some(format!("the cargo alias `{sub}` nests too deep to check"));
        }
        return match cargo_alias::resolve(sub, scope, ctx) {
            Ok(Some(expansion)) => {
                let mut expanded = words[..at + 1 + index].to_vec();
                expanded.extend(expansion);
                expanded.extend_from_slice(&rest[index + 1..]);
                check_at(&expanded, at, scope, ctx, depth + 1)
            }
            Ok(None) => Some(format!(
                "`cargo {sub}` is neither a cargo command nor an alias the guard can read; run \
                 the cargo command itself"
            )),
            Err(why) => Some(why),
        };
    }
    if READ_ONLY.contains(&sub) {
        return None;
    }
    let named = match named_targets(&words[..at], rest, sub, scope) {
        Ok(named) => named,
        Err(why) => return Some(why),
    };
    for target in &named {
        let expanded = ctx.expand(target, scope);
        // A `$( )` or backtick on the line may compute the target.
        if scope.substitutes || expanded.contains('$') || expanded.contains('`') {
            return Some(format!(
                "can't tell where `cargo {sub}` would build ({target}); name your own target \
                 folder with --target-dir"
            ));
        }
        if let Err(path) = ctx.may_change_word(target, scope) {
            return Some(format!(
                "`cargo {sub}` would build into {} outside your own folders; build into your \
                 own target and leave other bots' alone",
                path.display()
            ));
        }
    }
    for dir in &scope.dirs {
        if let Some(why) = config_files(dir, ctx, scope, sub) {
            return Some(why);
        }
    }
    None
}

/// A line that names a Cargo target and runs a `$( )` or backtick: the
/// substitution splits the words, so the target can't be read; refused.
pub(super) fn computed_target(line: &str, substitutes: bool) -> Option<String> {
    let names = VARS.iter().any(|v| line.contains(v)) || line.contains("target-dir");
    (substitutes && names).then(|| {
        "this line computes a Cargo target with a substitution; name your own target folder \
         with --target-dir"
            .to_string()
    })
}

/// The subcommand after `+toolchain` and global options; `None` for a bare
/// `cargo --version` or `cargo` alone.
fn subcommand(rest: &[String]) -> Option<(usize, &str)> {
    let mut i = 0;
    while let Some(w) = rest.get(i) {
        if w.starts_with('+') {
            i += 1;
        } else if VALUED.contains(&w.as_str()) {
            i += 2;
        } else if w.starts_with('-') {
            i += 1;
        } else {
            return Some((i, w.as_str()));
        }
    }
    None
}

/// Every target the line names, from its variables and options.
fn named_targets(
    before: &[String],
    rest: &[String],
    sub: &str,
    scope: &Scope,
) -> Result<Vec<String>, String> {
    let mut named: Vec<String> = VARS
        .iter()
        .filter_map(|var| scope.vars.get(*var).cloned())
        .collect();
    named.extend(before.iter().filter_map(|w| {
        w.split_once('=')
            .filter(|(name, _)| VARS.contains(name))
            .map(|(_, value)| value.to_string())
    }));
    let mut i = 0;
    while let Some(w) = rest.get(i) {
        i += 1;
        if w == "--" {
            break;
        }
        let writes_to = |flag: &str| DIRS.contains(&flag) || (sub == "install" && flag == "--root");
        if writes_to(w) {
            let value = rest
                .get(i)
                .ok_or_else(|| format!("`{w}` needs a folder; name your own target"))?;
            named.push(value.clone());
            i += 1;
        } else if let Some((flag, value)) = w.split_once('=').filter(|(f, _)| writes_to(f)) {
            let _ = flag;
            named.push(value.to_string());
        } else if w == "--config" {
            let value = rest.get(i).cloned().unwrap_or_default();
            named.extend(config_value(&value)?);
            i += 1;
        } else if let Some(value) = w.strip_prefix("--config=") {
            named.extend(config_value(value)?);
        }
    }
    Ok(named)
}

/// `build.target-dir="x"` as `x`; other keys name no target. A `--config`
/// file can't be read here.
fn config_value(value: &str) -> Result<Option<String>, String> {
    let Some((key, raw)) = value.split_once('=') else {
        return Err(format!(
            "`--config {value}` reads a file the guard can't check; pass --target-dir instead"
        ));
    };
    let key: String = key
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '"')
        .collect();
    if key.starts_with("alias.") {
        return Err(format!(
            "`--config {value}` defines a cargo alias the guard can't follow; run the command itself"
        ));
    }
    if key != "build.target-dir" {
        return Ok(None);
    }
    let raw = raw.trim();
    let unquoted = raw
        .strip_prefix('"')
        .and_then(|r| r.strip_suffix('"'))
        .or_else(|| raw.strip_prefix('\'').and_then(|r| r.strip_suffix('\'')))
        .unwrap_or(raw);
    Ok(Some(unquoted.to_string()))
}

/// A `target-dir` set in a `.cargo/config.toml` (or `config`) above `dir`:
/// relative to the folder holding `.cargo`.
fn config_files(dir: &Path, ctx: &GuardContext, scope: &Scope, sub: &str) -> Option<String> {
    for base in dir.ancestors() {
        for name in ["config.toml", "config"] {
            let file = base.join(".cargo").join(name);
            let Ok(text) = std::fs::read_to_string(&file) else {
                continue;
            };
            let Some(line) = text
                .lines()
                .map(str::trim)
                .find(|l| l.starts_with("target-dir") || l.starts_with("build.target-dir"))
            else {
                continue;
            };
            let value = line.split_once('=').map_or("", |(_, v)| v.trim());
            let value = value.trim_matches(|c| c == '"' || c == '\'');
            let target: PathBuf = if Path::new(value).is_absolute() {
                PathBuf::from(value)
            } else {
                base.join(value)
            };
            let word = target.display().to_string();
            if value.is_empty() || ctx.may_change_word(&word, scope).is_err() {
                return Some(format!(
                    "`cargo {sub}` would build into {} (target-dir in {}), outside your own \
                     folders",
                    target.display(),
                    file.display()
                ));
            }
        }
    }
    None
}
