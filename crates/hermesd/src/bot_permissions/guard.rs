//! `hermesd guard`: the PreToolUse hook every bot runs (H-031 §2, hardened
//! per CE-003 M3–M5).
//!
//! Permission rules match command text, so `git -C . push -f`, `sh -c '…'`,
//! an absolute `/bin/rm` or a `python3 -c "open(…)"` walk past them. The
//! guard reads the tool call on stdin, parses it, and denies:
//! - a Read, Grep or Glob of a protected path;
//! - any mention of the daemon's secrets, `~/.ssh`, `~/.claude.json`,
//!   Claude settings files, the daemon config or a bot's generated settings,
//!   however the path is spelled (`//`, `/./`, `..`, `"$HOME"`, `~user`, a
//!   variable set earlier on the line, a glob, a `cd` before it, a symlink);
//! - `rm`, `mv`, `cp`, `tee`, `dd`, `truncate`, `ln`, `rsync`, `sed -i`,
//!   `find -delete`/`-exec`, `xargs <destructive>` or an output redirect
//!   aimed outside the bot's own directory, the project's artifacts, the
//!   temp dirs and the git worktrees (`<repo>-wt-*`) in the trusted paths;
//! - a forced `git push` or a remote branch deletion, in any spelling, and
//!   any push or merge to `main` unless the bot holds `release_main`;
//! - `pkill`/`killall`, and `simctl` against `all` or `booted` (CE-001);
//! - in Full, where nothing else reviews a call: inline interpreter code
//!   (`python3 -c`, `perl -e`, `node -e`, …), commands read from stdin,
//!   `eval`, decoded text fed into a substitution, and writes outside the
//!   bot's own folders.
//!
//! Hooks run in every permission mode, so this is the boundary that still
//! holds in Full. It is a parser, not a sandbox: a script file can still
//! compute a path at runtime.

use std::path::PathBuf;

use serde_json::{json, Value};

mod commands;
mod full;
mod git;
mod paths;
mod words;

use paths::Scope;

/// What the guard knows about the bot it guards, passed on its command line.
pub struct GuardContext {
    /// The daemon's home (`~/.gravity`).
    pub home: PathBuf,
    pub user_home: PathBuf,
    /// Where destructive commands may act: the bot's directory, the
    /// project's artifacts and the temp dirs.
    pub writable: Vec<PathBuf>,
    /// Folders (the trusted paths) whose `<repo>-wt-*` children are git
    /// worktrees bots may also change. The folders themselves, and the
    /// owner's checkouts in them, are not.
    pub worktrees: Vec<PathBuf>,
    /// The bot holds `release_main`: it may push and merge to `main`.
    pub allow_main: bool,
    /// The project runs in Full, where the guard is the only check.
    pub full: bool,
}

/// Why the tool call must not run, or `None` to let it through.
pub fn decide(input: &Value, ctx: &GuardContext) -> Option<String> {
    let tool = input["tool_name"].as_str().unwrap_or_default();
    let args = &input["tool_input"];
    let cwd = PathBuf::from(input["cwd"].as_str().unwrap_or("/"));
    let scope = Scope::new(&cwd);
    match tool {
        "Bash" => commands::line(
            args["command"].as_str().unwrap_or_default(),
            &mut scope.clone(),
            ctx,
        ),
        "Read" | "Grep" | "Glob" => {
            // `path` is where Grep/Glob search; a Glob pattern can name a path too.
            ["file_path", "path", "pattern"]
                .iter()
                .filter(|key| tool != "Grep" || **key != "pattern")
                .filter_map(|key| args[*key].as_str())
                .find_map(|path| ctx.protected_word(path, &scope))
                .map(|path| format!("{path} is protected; don't read it, ask the owner"))
        }
        "Write" | "Edit" | "MultiEdit" | "NotebookEdit" => {
            let file = args["file_path"]
                .as_str()
                .or_else(|| args["notebook_path"].as_str())
                .unwrap_or_default();
            if let Some(path) = ctx.protected_word(file, &scope) {
                return Some(format!("{path} is protected; ask the owner to change it"));
            }
            // In Full nothing reviews a write to a shell rc or a launch agent.
            if ctx.full {
                if let Err(path) = ctx.may_write_file(file, &scope) {
                    return Some(format!(
                        "{} is outside your own folders; ask the owner",
                        path.display()
                    ));
                }
            }
            None
        }
        _ => None,
    }
}

/// The hook's answer for Claude Code, or nothing to allow.
pub fn verdict(reason: Option<String>) -> Option<Value> {
    reason.map(|reason| {
        json!({
            "hookSpecificOutput": {
                "hookEventName": "PreToolUse",
                "permissionDecision": "deny",
                "permissionDecisionReason": format!("Blocked by the Hermes guard: {reason}."),
            }
        })
    })
}

/// `hermesd guard --home H --user-home U [--writable P]… [--worktrees P]…
/// [--allow-main] [--full]`: read one tool call on stdin and print a deny
/// when it must not run. Never fails the call on its own errors: a broken
/// guard must not wedge every bot.
pub fn run(args: &[String]) -> i32 {
    let value_of = |flag: &str| {
        args.iter()
            .position(|a| a == flag)
            .and_then(|i| args.get(i + 1))
            .map(PathBuf::from)
    };
    let all_of = |flag: &str| -> Vec<PathBuf> {
        args.windows(2)
            .filter(|pair| pair[0] == flag)
            .map(|pair| PathBuf::from(&pair[1]))
            .collect()
    };
    let (Some(home), Some(user_home)) = (value_of("--home"), value_of("--user-home")) else {
        eprintln!(
            "usage: hermesd guard --home <dir> --user-home <dir> [--writable <dir>]… \
             [--worktrees <dir>]… [--allow-main] [--full]"
        );
        return 2;
    };
    let mut writable = all_of("--writable");
    writable.extend(temp_dirs());
    let ctx = GuardContext {
        home,
        user_home,
        writable,
        worktrees: all_of("--worktrees"),
        allow_main: args.iter().any(|a| a == "--allow-main"),
        full: args.iter().any(|a| a == "--full"),
    };
    let mut input = String::new();
    if std::io::Read::read_to_string(&mut std::io::stdin(), &mut input).is_err() {
        return 0;
    }
    let Ok(call) = serde_json::from_str::<Value>(&input) else {
        return 0;
    };
    if let Some(answer) = verdict(decide(&call, &ctx)) {
        println!("{answer}");
    }
    0
}

fn temp_dirs() -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = [
        "/tmp",
        "/private/tmp",
        "/var/folders",
        "/private/var/folders",
    ]
    .into_iter()
    .map(PathBuf::from)
    .collect();
    dirs.push(std::env::temp_dir());
    dirs
}
