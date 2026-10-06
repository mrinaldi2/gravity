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
//!   temp dirs, the bot's own git worktrees (`<repo>-wt-<bot>-*`) in the
//!   trusted paths and, for a bot that publishes, the release worktrees
//!   (`<repo>-rel-*`);
//! - a forced `git push` or a remote branch deletion, in any spelling, and
//!   any push or merge to `main` unless the bot holds `release_main`;
//! - `pkill`/`killall`, and `simctl` against `all` or `booted` (CE-001);
//! - in Full, where nothing else reviews a call: inline interpreter code
//!   (`python3 -c`, `perl -e`, `node -e`, …), commands read from stdin,
//!   `eval`, decoded text fed into a substitution, and writes outside the
//!   bot's own folders;
//! - a Write, Edit or destructive command into the served release builds,
//!   for every bot, DevOps included: only the daemon writes there (CE-010
//!   M3). Reading them stays allowed. The same holds for the daemon's own
//!   state in `<home>/run` (CE-023 M1);
//! - `hermesd release install|quiesce` pointed at another config or home
//!   (CE-023 F1).
//!
//! Hooks run in every permission mode, so this is the boundary that still
//! holds in Full. It is a parser, not a sandbox: a script file can still
//! compute a path at runtime.
//!
//! It fails closed: a tool call it can't parse is denied, in every profile
//! (CE-004). It can't cover its own absence, though. If the binary the hook
//! names is gone (an app update moved `hermesd`), Claude Code treats the
//! hook's failure to start as a non-blocking error and runs the call: Standard
//! and Trusted still have the auto-mode classifier, Full has nothing. That is
//! one reason Full is not selectable yet (CE-004 (b)).

use std::path::PathBuf;

use serde_json::{json, Value};

mod cargo;
mod cargo_alias;
mod commands;
mod daemon_cli;
mod full;
mod git;
mod path_key;
pub(super) mod paths;
mod served;
mod targets;
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
    /// The bot's slug (`desktopdev`): when known, only `<repo>-wt-<slug>`
    /// and `<repo>-wt-<slug>-*` are its worktrees, not another bot's
    /// (CE-004 F2). `None`: any `-wt-` folder.
    pub bot_slug: Option<String>,
    /// The bot publishes releases: the `<repo>-rel-*` release worktrees in
    /// the trusted paths are its to clean and reset (CE-004 R1).
    pub releases: bool,
    /// The bot holds `release_main`: it may push and merge to `main`.
    pub allow_main: bool,
    /// The project runs in Full, where the guard is the only check.
    pub full: bool,
    /// The served release builds and their staging folder: no bot writes
    /// there. `<home>/releases` is always among them.
    pub served: Vec<PathBuf>,
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
            if let Some(path) = ctx.served_word(file, &scope) {
                return Some(format!(
                    "{path} is in the served release builds, which only the daemon writes; \
                     publish with `hermesd release publish`"
                ));
            }
            if let Some(path) = ctx.run_word(file, &scope) {
                return Some(format!(
                    "{path} is the daemon's own state, which only the daemon writes; \
                     leave it alone"
                ));
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

/// The hook's answer to one stdin payload: a deny when the call must not run
/// or can't be read. A payload the guard can't parse is refused, so a
/// malformed call is never waved through (CE-004).
pub fn answer(payload: &str, ctx: &GuardContext) -> Option<Value> {
    let reason = match serde_json::from_str::<Value>(payload) {
        Ok(call) if call["tool_name"].as_str().is_some_and(|t| !t.is_empty()) => {
            if call["tool_name"] == "Bash" && !call["tool_input"]["command"].is_string() {
                Some("it couldn't read this Bash call's command, so it is refused".to_string())
            } else {
                decide(&call, ctx)
            }
        }
        Ok(_) => Some("it couldn't tell which tool this call is for, so it is refused".to_string()),
        Err(_) => Some("it couldn't parse this tool call, so it is refused".to_string()),
    };
    verdict(reason)
}

/// A bot name as its worktree slug: `Desktop Dev` is `desktopdev`.
pub fn slug(bot_name: &str) -> String {
    bot_name
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

/// `hermesd guard --home H --user-home U [--writable P]… [--worktrees P]…
/// [--served P]… [--bot NAME] [--releases] [--allow-main] [--full]`: read one tool call on
/// stdin and print a deny when it must not run or can't be read.
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
             [--worktrees <dir>]… [--served <dir>]… [--bot <name>] [--releases] \
             [--allow-main] [--full]"
        );
        // Exit 2 blocks the call; stderr tells the bot why.
        return 2;
    };
    let mut writable = all_of("--writable");
    writable.extend(temp_dirs());
    let mut served = all_of("--served");
    served.push(home.join("releases"));
    let ctx = GuardContext {
        served,
        home,
        user_home,
        writable,
        worktrees: all_of("--worktrees"),
        bot_slug: args
            .iter()
            .position(|a| a == "--bot")
            .and_then(|i| args.get(i + 1))
            .map(|name| slug(name))
            .filter(|slug| !slug.is_empty()),
        releases: args.iter().any(|a| a == "--releases"),
        allow_main: args.iter().any(|a| a == "--allow-main"),
        full: args.iter().any(|a| a == "--full"),
    };
    let mut input = String::new();
    // Unreadable stdin leaves an empty payload, which `answer` refuses.
    if std::io::Read::read_to_string(&mut std::io::stdin(), &mut input).is_err() {
        input.clear();
    }
    if let Some(answer) = answer(&input, &ctx) {
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
