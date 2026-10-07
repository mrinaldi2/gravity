//! The guard's verdict on one command line, simple command by simple
//! command, carrying `cd`, variables and symlinks from one to the next.

use std::path::{Path, PathBuf};

use super::mentions::{echoed_text, without_redirects};
use super::paths::Scope;
use super::{
    cargo, cd, daemon_cli, full, git, heredoc, powershell, ps_launch, targets, GuardContext,
};
use crate::bot_permissions::shell::{self, Then, Words};

/// Why the line must not run, or `None`.
pub(super) fn line(line: &str, scope: &mut Scope, ctx: &GuardContext) -> Option<String> {
    scope.substitutes |= line.contains("$(") || line.contains('`');
    if let Some(reason) = cargo::computed_target(line, scope.substitutes) {
        return Some(reason);
    }
    let commands = shell::parse(line);
    scope.changes.note(&commands);
    // What an `xargs` later on the line may be fed: every path it names.
    for words in commands.iter().map(|c| &c.words) {
        let program = shell::program_index(words);
        scope.named.extend(
            words
                .iter()
                .enumerate()
                .filter(|(i, w)| Some(*i) != program && looks_like_path(w))
                .map(|(_, w)| w.clone()),
        );
    }
    // `cd X && …` runs what follows in X, and nowhere else, until the chain
    // of `&&` and `|` ends (H-155). Elsewhere a `cd` may not have happened,
    // so every directory the line may be in counts.
    let mut certain: Option<Vec<PathBuf>> = None;
    for cmd in &commands {
        let all = std::mem::take(&mut scope.dirs);
        scope.dirs = certain.clone().unwrap_or_else(|| all.clone());
        let reason = command_as(&cmd.words, cmd.then == Then::Pipe, scope, ctx)
            .or_else(|| heredoc::judge(cmd, scope, ctx));
        let moved = reason
            .is_none()
            .then(|| remember(&cmd.words, scope, ctx))
            .flatten();
        scope.dirs = all;
        if reason.is_some() {
            return reason;
        }
        certain = cd::next(certain, moved, cmd.then, scope);
    }
    None
}

fn looks_like_path(word: &str) -> bool {
    !word.starts_with('-') && (word.contains('/') || word.starts_with('~') || word.starts_with('.'))
}

/// What a command changes for the ones after it: `name=value`, `ln -s`,
/// and where a `cd` went, which the caller tracks.
fn remember(words: &Words, scope: &mut Scope, ctx: &GuardContext) -> Option<cd::Moved> {
    let skip = usize::from(matches!(
        words.first().map(String::as_str),
        Some("export" | "local" | "declare" | "readonly" | "typeset")
    ));
    if words.len() > skip && words[skip..].iter().all(|w| shell::is_assignment(w)) {
        for word in &words[skip..] {
            if let Some((name, value)) = word.split_once('=') {
                let value = ctx.expand(value, scope);
                scope.vars.insert(name.to_string(), value);
            }
        }
        return None;
    }
    if words.first().is_some_and(|w| w == "for") && words.get(2).is_some_and(|w| w == "in") {
        scope.lists.insert(words[1].clone(), words[3..].to_vec());
        return None;
    }
    let at = shell::program_index(words)?;
    let args = positional(&words[at + 1..]);
    match shell::program(&words[at]) {
        "cd" | "pushd" => return Some(cd::moved(&args, scope, ctx)),
        "popd" => return Some(cd::Moved::Somewhere(Vec::new())),
        "ln" if args.len() >= 2 => {
            let (target, link) = (&args[0], &args[args.len() - 1]);
            for dir in ctx.resolve(scope, link) {
                let link = if dir.is_dir() {
                    dir.join(Path::new(target).file_name().unwrap_or_default())
                } else {
                    dir
                };
                let parent = link.parent().map(Path::to_path_buf).unwrap_or_default();
                let mut from = scope.clone();
                from.dirs = vec![parent];
                for target in ctx.resolve(&from, target) {
                    scope.links.push((link.clone(), target));
                }
            }
        }
        _ => {}
    }
    None
}

/// Arguments that are not options; everything after `--` counts.
pub(super) fn positional(args: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    let mut ended = false;
    for arg in args {
        if ended || !arg.starts_with('-') || arg == "-" {
            out.push(arg.clone());
        } else if arg == "--" {
            ended = true;
        }
    }
    out.retain(|w| !is_redirect(w) && !w.starts_with('<'));
    out
}

pub(super) fn command(words: &Words, scope: &Scope, ctx: &GuardContext) -> Option<String> {
    command_as(words, true, scope, ctx)
}

/// One command's verdict; `feeds_next` when its output is piped on.
fn command_as(
    words: &Words,
    feeds_next: bool,
    scope: &Scope,
    ctx: &GuardContext,
) -> Option<String> {
    // `tar -C ~ …`, `make --directory=…`: later words are relative to it.
    let mut local = scope.clone();
    for (i, w) in words.iter().enumerate() {
        let dir = if matches!(w.as_str(), "-C" | "--directory" | "--chdir" | "--cwd") {
            words.get(i + 1).cloned()
        } else {
            ["--directory=", "--chdir=", "--cwd="]
                .iter()
                .find_map(|flag| w.strip_prefix(flag))
                .map(str::to_string)
        };
        if let Some(dir) = dir {
            let dirs = ctx.resolve(&local, &dir);
            local.enter(dirs);
        }
    }
    let scope = &local;
    let text = echoed_text(words, feeds_next, scope, ctx);
    if let Some(path) = words
        .iter()
        .enumerate()
        .filter(|(i, _)| !text.contains(i))
        .find_map(|(_, w)| ctx.protected_word(w, scope))
    {
        return Some(format!(
            "this command touches {path}, which is protected; don't reword it, ask the owner"
        ));
    }
    if let Some(reason) = redirect_outside(words, scope, ctx) {
        return Some(reason);
    }
    let at = shell::program_index(words)?;
    let name = shell::program(&words[at]);
    let rest = &words[at + 1..];
    let args = positional(rest);
    if ctx.full {
        if let Some(reason) = full::only(name, rest, &args, scope) {
            return Some(reason);
        }
    }
    if let Some(reason) = daemon_cli::redirected(words, at, scope) {
        return Some(reason);
    }
    if starts_vm(name, &args) {
        if let Some(reason) = crate::quiesce::pending::blocking(&ctx.home) {
            return Some(reason);
        }
    }
    if let Some(reason) = reads_tree(name, rest, &args, scope, ctx) {
        return Some(reason);
    }
    if let Some(reason) = super::links::check(name, rest, scope, ctx) {
        return Some(reason);
    }
    let targets: Vec<String> = match name {
        "sh" | "bash" | "zsh" | "dash" | "ksh" => {
            let script = rest
                .iter()
                .position(|w| w.starts_with('-') && !w.starts_with("--") && w.contains('c'))
                .and_then(|i| rest.get(i + 1))?;
            return line(script, &mut scope.clone(), ctx);
        }
        "pkill" | "killall" => {
            return Some(format!(
            "`{name}` could stop another bot's process; keep the PID you started and `kill <pid>`"
        ))
        }
        "git" => {
            let check = |script: &str| line(script, &mut scope.clone(), ctx);
            return git::git(rest, scope, ctx, &check);
        }
        "gh" => return git::gh(rest, ctx),
        // `powershell -c …`, `pwsh -EncodedCommand …`, `cmd /c …` (H-187).
        n if powershell::is_launcher(&powershell::program(n)) => {
            return ps_launch::launch(&powershell::program(n), rest, scope, ctx)
        }
        "xcrun" if rest.first().is_some_and(|w| w == "simctl") => return simctl(&rest[1..]),
        "simctl" => return simctl(rest),
        "find" => return find(rest, scope, ctx),
        "xargs" => return xargs(rest, scope, ctx),
        "cargo" => return cargo::check(words, at, scope, ctx),
        // `cargo-clippy clippy …`: a subcommand's own binary (CE-013).
        bin if bin.len() > 6 && bin.starts_with("cargo-") => {
            return cargo::check_binary(words, at, &bin[6..], scope, ctx)
        }
        _ => {
            let plain = without_redirects(rest);
            targets::of(name, &plain, &positional(&plain))
        }
    };
    let target = targets
        .iter()
        .find_map(|t| ctx.may_change_word(t, scope).err())?;
    Some(format!(
        "`{name}` would change {} outside your own folders; leave it alone and ask its owner",
        target.display()
    ))
}

/// `find ~`, `grep -r … ~`, `tar -c ~`: a recursive read of a folder that
/// holds a protected one reads it too.
fn reads_tree(
    name: &str,
    rest: &[String],
    args: &[String],
    scope: &Scope,
    ctx: &GuardContext,
) -> Option<String> {
    let flag = |letters: &str, long: &[&str]| {
        rest.iter().any(|w| {
            long.contains(&w.as_str())
                || (w.starts_with('-')
                    && !w.starts_with("--")
                    && w.chars().any(|c| letters.contains(c)))
        })
    };
    let recursive = match name {
        "find" | "tar" | "gtar" | "bsdtar" | "rsync" | "ditto" => true,
        "grep" | "egrep" | "fgrep" | "ggrep" => {
            flag("rR", &["--recursive", "--dereference-recursive"])
        }
        "rg" | "ag" => flag("u.", &["--hidden", "--unrestricted"]),
        "cp" | "scp" | "zip" | "7z" => flag("rRa", &["--recursive", "--archive"]),
        _ => false,
    };
    if !recursive {
        return None;
    }
    let mut roots: Vec<String> = args.to_vec();
    // `find` with no start point, or a grep given only its pattern, reads cwd.
    if args.len() < 2 {
        roots.extend(scope.dirs.iter().map(|d| d.display().to_string()));
    }
    let path = roots.iter().find_map(|w| ctx.holds_protected(w, scope))?;
    Some(format!(
        "`{name}` would read {path}, which is protected, along with the rest; narrow it to the folder you need"
    ))
}

/// What `find` deletes or hands to a destructive command: its start points.
fn find(rest: &[String], scope: &Scope, ctx: &GuardContext) -> Option<String> {
    let roots: Vec<&String> = rest
        .iter()
        .take_while(|w| !w.starts_with('-') && *w != "(" && *w != "!")
        .collect();
    let mut destructive = rest.iter().any(|w| w == "-delete");
    for (i, w) in rest.iter().enumerate() {
        if matches!(w.as_str(), "-exec" | "-execdir" | "-ok" | "-okdir") {
            let program = rest.get(i + 1).map_or("", |p| shell::program(p));
            destructive |= DESTRUCTIVE.contains(&program);
        }
    }
    if !destructive {
        return None;
    }
    let start = if roots.is_empty() {
        vec![".".to_string()]
    } else {
        roots.into_iter().cloned().collect()
    };
    let target = start
        .iter()
        .find_map(|root| ctx.may_change_word(root, scope).err())?;
    Some(format!(
        "`find` would delete under {} outside your own folders; leave it alone",
        target.display()
    ))
}

/// Programs that change or delete files, or run anything at all.
const DESTRUCTIVE: &[&str] = &[
    "rm", "rmdir", "unlink", "shred", "srm", "trash", "mv", "cp", "tee", "dd", "truncate", "ln",
    "rsync", "install", "ditto", "chmod", "chown", "sed", "perl", "python", "python3", "ruby",
    "node", "sh", "bash", "zsh", "dash", "xargs", "git",
];

/// `… | xargs rm`: the targets arrive on stdin, so every path the line
/// names, and every directory it runs in, must be the bot's own.
fn xargs(rest: &[String], scope: &Scope, ctx: &GuardContext) -> Option<String> {
    let valued = [
        "-I", "-n", "-P", "-L", "-d", "-E", "-s", "-a", "-J", "-R", "-S",
    ];
    let mut i = 0;
    while let Some(w) = rest.get(i) {
        if valued.contains(&w.as_str()) {
            i += 2;
        } else if w.starts_with('-') {
            i += 1;
        } else {
            break;
        }
    }
    let program = rest.get(i).map_or("", |p| shell::program(p));
    if !DESTRUCTIVE.contains(&program) {
        return None;
    }
    let mut named: Vec<String> = scope.dirs.iter().map(|d| d.display().to_string()).collect();
    named.extend(scope.named.iter().cloned());
    let target = named
        .iter()
        .find_map(|w| ctx.may_change_word(w, scope).err())?;
    Some(format!(
        "`xargs {program}` would act on what the line lists, which includes {} outside your own folders",
        target.display()
    ))
}

/// `colima start`, `limactl start` or the VR run (`scripts/vr-ci.sh`, as is
/// or through a shell): what a pending install must not meet (H-166).
fn starts_vm(name: &str, args: &[String]) -> bool {
    let vr = |word: &str| word.ends_with("vr-ci.sh");
    match name {
        "colima" | "limactl" => args.first().is_some_and(|a| a == "start"),
        "sh" | "bash" | "zsh" => args.first().is_some_and(|a| vr(a)),
        other => vr(other),
    }
}

fn simctl(rest: &[String]) -> Option<String> {
    rest.iter().any(|w| w == "booted" || w == "all").then(|| {
        "address only your own simulator, by its UDID (never `booted` or `all`)".to_string()
    })
}

pub(super) fn is_redirect(word: &str) -> bool {
    word.trim_start_matches(|c: char| c.is_ascii_digit() || c == '&')
        .starts_with('>')
}

/// `> file`, `>> file`, `2>file`: where the output lands must be ours.
fn redirect_outside(words: &Words, scope: &Scope, ctx: &GuardContext) -> Option<String> {
    let mut targets = Vec::new();
    for (i, w) in words.iter().enumerate() {
        if !is_redirect(w) || w.contains(">&") {
            continue; // not a redirect, or fd duplication
        }
        let glued = w.trim_start_matches(|c: char| c.is_ascii_digit() || c == '&' || c == '>');
        if glued.is_empty() {
            if let Some(next) = words.get(i + 1) {
                targets.push(next.as_str());
            }
        } else {
            targets.push(glued);
        }
    }
    let target = targets
        .into_iter()
        .find_map(|t| ctx.may_change_word(t, scope).err())?;
    Some(format!(
        "output would be written to {} outside your own folders",
        target.display()
    ))
}
