//! The guard's `git` and `gh` checks: no forced push or remote branch
//! deletion in any spelling, and nothing reaches `main` unless the bot holds
//! the `release_main` extra (CE-003 decision). Aliases are resolved first, so
//! `git p` for `push --force` is judged as the push it is.

use std::path::{Path, PathBuf};
use std::process::Command;

use super::paths::Scope;
use super::GuardContext;
use crate::bot_permissions::shell;

/// The branch only a bot with `release_main` may push or merge to.
const MAIN: &str = "main";

/// Subcommands git runs itself; anything else may be an alias.
const BUILTINS: &[&str] = &[
    "add",
    "am",
    "apply",
    "archive",
    "bisect",
    "blame",
    "branch",
    "cat-file",
    "checkout",
    "cherry-pick",
    "clean",
    "clone",
    "commit",
    "config",
    "describe",
    "diff",
    "fetch",
    "for-each-ref",
    "format-patch",
    "gc",
    "grep",
    "help",
    "init",
    "log",
    "ls-files",
    "ls-remote",
    "merge",
    "merge-base",
    "mv",
    "notes",
    "prune",
    "pull",
    "push",
    "rebase",
    "reflog",
    "remote",
    "reset",
    "restore",
    "rev-list",
    "rev-parse",
    "rm",
    "shortlog",
    "show",
    "show-ref",
    "sparse-checkout",
    "stash",
    "status",
    "submodule",
    "switch",
    "symbolic-ref",
    "tag",
    "update-ref",
    "version",
    "worktree",
];

/// Global options that take the next word as their value.
const VALUED_GLOBALS: &[&str] = &[
    "-C",
    "-c",
    "--git-dir",
    "--work-tree",
    "--namespace",
    "--config-env",
];

/// `git <rest>` run from any of the scope's directories. `shell_check` judges an alias that runs a shell command.
pub(super) fn git(
    rest: &[String],
    scope: &Scope,
    ctx: &GuardContext,
    shell_check: &dyn Fn(&str) -> Option<String>,
) -> Option<String> {
    let mut scope = scope.clone();
    let mut words = rest.to_vec();
    let mut i = 0;
    // An alias can expand to another alias; git stops such loops too.
    for _ in 0..8 {
        while let Some(w) = words.get(i) {
            if VALUED_GLOBALS.contains(&w.as_str()) {
                let value = words.get(i + 1).cloned().unwrap_or_default();
                if w == "-C" {
                    scope.dirs = ctx.resolve(&scope, &value);
                }
                if w == "-c" && is_alias_key(&value) {
                    return Some(ALIAS.to_string());
                }
                i += 2;
            } else if w.starts_with('-') {
                if w.starts_with("--config-env=") {
                    return Some(ALIAS.to_string());
                }
                i += 1;
            } else {
                break;
            }
        }
        let sub = words.get(i).cloned()?;
        let args = &words[i + 1..];
        if BUILTINS.contains(&sub.as_str()) || sub == "send-pack" {
            return match sub.as_str() {
                "push" => push(args, &scope.dirs, ctx),
                "send-pack" => (!ctx.allow_main)
                    .then(|| "use `git push`, which the guard can check".to_string()),
                "config" => config(args),
                "reset" | "clean" | "checkout" | "restore" if discards(&sub, args) => {
                    let dir = scope
                        .dirs
                        .iter()
                        .find(|d| !ctx.may_change(&super::paths::real(d)))?;
                    Some(format!(
                        "`git {sub}` would discard work in {}, which isn't yours; use your own worktree",
                        dir.display()
                    ))
                }
                "worktree" => worktree(args, &scope, ctx),
                _ => None,
            };
        }
        let alias = scope.dirs.iter().find_map(|d| alias_of(d, &sub))?;
        if let Some(script) = alias.strip_prefix('!') {
            return shell_check(script);
        }
        let expanded = shell::commands(&alias)
            .into_iter()
            .next()
            .unwrap_or_default();
        words = expanded.into_iter().chain(args.iter().cloned()).collect();
        i = 0;
    }
    Some("this git alias expands too deeply to check".to_string())
}

/// `git worktree remove|move <path>` deletes or moves a worktree: it must be
/// the bot's own (CE-004 F2), resolved from where git runs.
fn worktree(args: &[String], scope: &Scope, ctx: &GuardContext) -> Option<String> {
    let action = args.first()?;
    if !matches!(action.as_str(), "remove" | "move") {
        return None;
    }
    args[1..]
        .iter()
        .filter(|w| !w.starts_with('-'))
        .find_map(|w| ctx.may_change_word(w, scope).err())
        .map(|path| {
            format!(
                "`git worktree {action}` would act on {}, which isn't yours; use your own worktree",
                path.display()
            )
        })
}

/// `reset --hard`, `clean -f`, `checkout -- .`, `restore`: they throw away
/// uncommitted work, like an `rm` of it.
fn discards(sub: &str, args: &[String]) -> bool {
    let has = |flags: &[&str]| args.iter().any(|a| flags.contains(&a.as_str()));
    match sub {
        "reset" => has(&["--hard", "--merge", "--keep"]),
        "clean" => args.iter().any(|a| {
            a == "--force" || (a.starts_with('-') && !a.starts_with("--") && a.contains('f'))
        }),
        "checkout" => has(&["--", ".", "-f", "--force", "-p", "--patch"]),
        _ => !has(&["--staged"]) || has(&["--worktree", "-W"]),
    }
}

const ALIAS: &str =
    "git aliases defined on the command line can hide a push; spell the command out";

fn is_alias_key(setting: &str) -> bool {
    setting.to_ascii_lowercase().starts_with("alias.")
}

/// Writing an alias is how a later call would hide a push.
fn config(args: &[String]) -> Option<String> {
    let reading = args.iter().any(|a| {
        matches!(
            a.as_str(),
            "--get" | "--get-all" | "--get-regexp" | "--list" | "-l" | "get" | "list"
        )
    });
    (!reading && args.iter().any(|a| is_alias_key(a)))
        .then(|| "bots don't define git aliases; spell the command out".to_string())
}

fn alias_of(dir: &Path, name: &str) -> Option<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["config", "--get", &format!("alias.{name}")])
        .output()
        .ok()?;
    let text = String::from_utf8(out.stdout).ok()?;
    let text = text.trim();
    (out.status.success() && !text.is_empty()).then(|| text.to_string())
}

/// Push options that take the next word as their value.
const VALUED_PUSH: &[&str] = &["-o", "--push-option", "--repo", "--receive-pack", "--exec"];

fn push(args: &[String], dirs: &[PathBuf], ctx: &GuardContext) -> Option<String> {
    let mut positional = Vec::new();
    let mut i = 0;
    while let Some(w) = args.get(i) {
        i += 1;
        if VALUED_PUSH.contains(&w.as_str()) {
            i += 1;
        } else if w == "--force-if-includes" || w == "--no-force-if-includes" {
            continue;
        } else if w.starts_with("--force") || w == "--mirror" {
            return Some(FORCED.to_string());
        } else if w == "--delete" {
            return Some(DELETE.to_string());
        } else if w == "--all" || w == "--branches" {
            if !ctx.allow_main {
                return Some(MAIN_DENIED.to_string());
            }
        } else if let Some(cluster) = w
            .strip_prefix('-')
            .filter(|f| !f.is_empty() && !f.starts_with('-'))
        {
            // Letters before an `o` are flags; what follows it is its value.
            let (flags, value) = cluster
                .split_once('o')
                .map_or((cluster, None), |(flags, value)| (flags, Some(value)));
            if flags.contains('f') {
                return Some(FORCED.to_string());
            }
            if flags.contains('d') {
                return Some(DELETE.to_string());
            }
            if value == Some("") {
                i += 1;
            }
        } else if !w.starts_with('-') {
            positional.push(w.as_str());
        }
    }
    let mut refspecs = positional.into_iter().skip(1).peekable();
    if refspecs.peek().is_none() {
        // No refspec: git pushes the current branch.
        return (!ctx.allow_main && dirs.iter().any(|d| on_main(d)))
            .then(|| MAIN_DENIED.to_string());
    }
    let mut skip = false;
    for spec in refspecs {
        // `tag <name>` pushes a tag, never a branch.
        if std::mem::replace(&mut skip, spec == "tag") || spec == "tag" {
            continue;
        }
        if spec.starts_with('+') && spec.len() > 1 {
            return Some(FORCED.to_string());
        }
        let (src, dst) = spec.split_once(':').unwrap_or((spec, spec));
        // `:dst` pushes nothing onto dst: it deletes the remote branch.
        if src.is_empty() {
            return Some(DELETE.to_string());
        }
        if ctx.allow_main {
            continue;
        }
        let current = matches!(dst, "HEAD" | "@");
        let dst = dst.strip_prefix("refs/heads/").unwrap_or(dst);
        if dst == MAIN || dst.contains('*') || (current && dirs.iter().any(|d| on_main(d))) {
            return Some(MAIN_DENIED.to_string());
        }
    }
    None
}

const FORCED: &str = "forced pushes rewrite shared history; push normally or ask the owner";
const DELETE: &str = "deleting a remote branch can't be undone by a bot; ask the owner";
const MAIN_DENIED: &str = "only the bot with the release_main extra (DevOps) pushes or merges to main; push a feature branch and hand the merge over";

fn on_main(dir: &Path) -> bool {
    Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .output()
        .is_ok_and(|out| {
            out.status.success() && String::from_utf8_lossy(&out.stdout).trim() == MAIN
        })
}

/// `gh pr merge` and the API's merge and ref endpoints land on `main`.
pub(super) fn gh(rest: &[String], ctx: &GuardContext) -> Option<String> {
    if ctx.allow_main {
        return None;
    }
    let merge = match rest.first().map(String::as_str) {
        Some("pr") => rest.get(1).is_some_and(|w| w == "merge"),
        Some("api") => rest
            .iter()
            .any(|w| w.contains("/merge") || w.contains("refs/heads/main")),
        _ => false,
    };
    merge.then(|| MAIN_DENIED.to_string())
}
