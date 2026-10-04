//! `hermesd guard`: the PreToolUse hook every bot runs (H-031 §2).
//!
//! Permission rules match command text, so `git -C . push -f`, `sh -c '…'`,
//! an absolute `/bin/rm` or a `python3 -c "open(…)"` walk past them. The
//! guard reads the tool call on stdin, parses it, and denies:
//! - a Read, Grep or Glob of a protected path;
//! - any mention of the daemon's secrets, `~/.ssh`, `~/.claude.json`,
//!   Claude settings files, the daemon config or a bot's generated settings;
//! - `rm`, `rmdir`, `mv`, `unlink` or an output redirect aimed outside the
//!   bot's own directory, the project's artifacts or the trusted paths;
//! - a forced `git push`, in any spelling;
//! - `pkill`/`killall`, and `simctl` against `all` or `booted` (CE-001).
//!
//! Hooks run in every permission mode, so this is the boundary that still
//! holds in Full. It cannot see paths a script computes at runtime.

use std::path::{Component, Path, PathBuf};

use serde_json::{json, Value};

use super::shell::{self, Words};

/// What the guard knows about the bot it guards, passed on its command line.
pub struct GuardContext {
    /// The daemon's home (`~/.gravity`).
    pub home: PathBuf,
    pub user_home: PathBuf,
    /// Where destructive commands may act: the bot's directory, the
    /// project's artifacts, the trusted paths and the temp dirs.
    pub writable: Vec<PathBuf>,
}

impl GuardContext {
    fn protected(&self) -> Vec<PathBuf> {
        vec![
            self.home.join("secrets"),
            self.home.join("gravityd.toml"),
            self.home.join("bot-settings.json"),
            self.user_home.join(".ssh"),
            self.user_home.join(".claude.json"),
            self.user_home.join(".claude").join("settings.json"),
            self.user_home.join(".claude").join("settings.local.json"),
        ]
    }

    /// Any of these in a path makes it protected wherever it lives, written
    /// relative (`.claude/settings.json`) or absolute.
    const PROTECTED_NAMES: [&'static str; 3] = [
        ".claude/settings.json",
        ".claude/settings.local.json",
        "settings.gen.json",
    ];

    fn mentions_protected(&self, text: &str) -> Option<String> {
        let text = self.expand_home(text).replace('\\', "/");
        for path in self.protected() {
            let path = path.display().to_string().replace('\\', "/");
            if text.contains(&path) {
                return Some(path);
            }
        }
        Self::PROTECTED_NAMES
            .iter()
            .find(|name| text.contains(*name))
            .map(|name| (*name).to_string())
    }

    fn expand_home(&self, text: &str) -> String {
        let home = format!("{}/", self.user_home.display());
        text.replace("${HOME}/", &home)
            .replace("$HOME/", &home)
            .replace("~/", &home)
    }

    fn resolve(&self, cwd: &Path, word: &str) -> PathBuf {
        let expanded = self.expand_home(word);
        let path = Path::new(&expanded);
        normalize(&if path.is_absolute() {
            path.to_path_buf()
        } else {
            cwd.join(path)
        })
    }

    fn may_change(&self, path: &Path) -> bool {
        path == Path::new("/dev/null")
            || self
                .writable
                .iter()
                .any(|root| path.starts_with(normalize(root)))
    }
}

/// `a/./b/../c` → `a/c`, without touching the disk (the target may not exist).
fn normalize(path: &Path) -> PathBuf {
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

/// Why the tool call must not run, or `None` to let it through.
pub fn decide(input: &Value, ctx: &GuardContext) -> Option<String> {
    let tool = input["tool_name"].as_str().unwrap_or_default();
    let args = &input["tool_input"];
    let cwd = PathBuf::from(input["cwd"].as_str().unwrap_or("/"));
    match tool {
        "Bash" => bash(args["command"].as_str().unwrap_or_default(), &cwd, ctx),
        "Read" | "Grep" | "Glob" => {
            // `path` is where Grep/Glob search; a Glob pattern can name a path too.
            ["file_path", "path", "pattern"]
                .iter()
                .filter(|key| tool != "Grep" || **key != "pattern")
                .filter_map(|key| args[*key].as_str())
                .find_map(|path| {
                    ctx.mentions_protected(&ctx.resolve(&cwd, path).display().to_string())
                })
                .map(|path| format!("{path} is protected; don't read it, ask the owner"))
        }
        "Write" | "Edit" | "MultiEdit" | "NotebookEdit" => {
            let file = args["file_path"]
                .as_str()
                .or_else(|| args["notebook_path"].as_str())
                .unwrap_or_default();
            ctx.mentions_protected(&ctx.resolve(&cwd, file).display().to_string())
                .map(|path| format!("{path} is protected; ask the owner to change it"))
        }
        _ => None,
    }
}

fn bash(line: &str, cwd: &Path, ctx: &GuardContext) -> Option<String> {
    if let Some(path) = ctx.mentions_protected(line) {
        return Some(format!(
            "this command touches {path}, which is protected; don't reword it, ask the owner"
        ));
    }
    shell::commands(line)
        .iter()
        .find_map(|words| simple_command(words, cwd, ctx))
}

fn simple_command(words: &Words, cwd: &Path, ctx: &GuardContext) -> Option<String> {
    let at = shell::program_index(words)?;
    let name = shell::program(&words[at]);
    let rest = &words[at + 1..];
    if let Some(reason) = redirect_outside(words, cwd, ctx) {
        return Some(reason);
    }
    match name {
        "sh" | "bash" | "zsh" | "dash" => {
            let script = rest
                .iter()
                .position(|w| w == "-c")
                .and_then(|i| rest.get(i + 1))?;
            bash(script, cwd, ctx)
        }
        "pkill" | "killall" => Some(format!(
            "`{name}` could stop another bot's process; keep the PID you started and `kill <pid>`"
        )),
        "rm" | "rmdir" | "unlink" | "mv" | "shred" => {
            let target = rest
                .iter()
                .filter(|w| !w.starts_with('-') && !is_redirect(w))
                .map(|w| ctx.resolve(cwd, w))
                .find(|p| !ctx.may_change(p))?;
            Some(format!(
                "`{name}` would change {} outside your own folders; leave it alone and ask its owner",
                target.display()
            ))
        }
        "git" => git(rest),
        "xcrun" if rest.first().is_some_and(|w| w == "simctl") => simctl(&rest[1..]),
        "simctl" => simctl(rest),
        _ => None,
    }
}

fn is_redirect(word: &str) -> bool {
    word.trim_start_matches(|c: char| c.is_ascii_digit() || c == '&')
        .starts_with('>')
}

/// `> file`, `>> file`, `2>file`: where the output lands must be ours.
fn redirect_outside(words: &Words, cwd: &Path, ctx: &GuardContext) -> Option<String> {
    let mut targets = Vec::new();
    for (i, w) in words.iter().enumerate() {
        if !is_redirect(w) {
            continue;
        }
        let glued = w.trim_start_matches(|c: char| c.is_ascii_digit() || c == '&' || c == '>');
        if w.contains(">&") {
            continue; // fd duplication, not a file
        }
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
        .map(|t| ctx.resolve(cwd, t))
        .find(|p| !ctx.may_change(p))?;
    Some(format!(
        "output would be written to {} outside your own folders",
        target.display()
    ))
}

fn git(rest: &[String]) -> Option<String> {
    // Skip global options before the subcommand (`-C dir`, `-c k=v`, …).
    let mut i = 0;
    while let Some(w) = rest.get(i) {
        if w == "-C" || w == "-c" {
            i += 2;
        } else if w.starts_with('-') {
            i += 1;
        } else {
            break;
        }
    }
    if rest.get(i).map(String::as_str) != Some("push") {
        return None;
    }
    let forced = rest[i + 1..].iter().any(|w| {
        w.starts_with("--force")
            || w == "--mirror"
            || (w.starts_with('-') && !w.starts_with("--") && w.contains('f'))
            || (w.starts_with('+') && w.len() > 1)
    });
    forced
        .then(|| "forced pushes rewrite shared history; push normally or ask the owner".to_string())
}

fn simctl(rest: &[String]) -> Option<String> {
    rest.iter().any(|w| w == "booted" || w == "all").then(|| {
        "address only your own simulator, by its UDID (never `booted` or `all`)".to_string()
    })
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

/// `hermesd guard --home H --user-home U [--writable P]…`: read one tool call
/// on stdin and print a deny when it must not run. Never fails the call on
/// its own errors: a broken guard must not wedge every bot.
pub fn run(args: &[String]) -> i32 {
    let value_of = |flag: &str| {
        args.iter()
            .position(|a| a == flag)
            .and_then(|i| args.get(i + 1))
            .map(PathBuf::from)
    };
    let (Some(home), Some(user_home)) = (value_of("--home"), value_of("--user-home")) else {
        eprintln!("usage: hermesd guard --home <dir> --user-home <dir> [--writable <dir>]…");
        return 2;
    };
    let mut writable: Vec<PathBuf> = args
        .windows(2)
        .filter(|pair| pair[0] == "--writable")
        .map(|pair| PathBuf::from(&pair[1]))
        .collect();
    writable.extend(temp_dirs());
    let ctx = GuardContext {
        home,
        user_home,
        writable,
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
