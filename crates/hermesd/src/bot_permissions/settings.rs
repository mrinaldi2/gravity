//! The `--settings` file a bot starts with (H-031 §2, rule syntax per
//! CE-002): the classifier's context, the hard deny-list, the profile's
//! allow-list, the bot's extras and the guard hook.
//!
//! Every Read/Edit path rule is anchored (`//abs` or `~/`), never bare: a
//! bare path moves with the shell's working directory.

use std::path::Path;

use bus::{PermissionExtra, PermissionProfile};
use serde_json::{json, Value};

/// Everything the file is generated from.
pub struct SettingsInput<'a> {
    pub profile: PermissionProfile,
    pub extras: &'a [PermissionExtra],
    pub project_name: &'a str,
    /// The daemon's home (`~/.gravity`).
    pub home: &'a Path,
    pub workspace: &'a Path,
    pub artifacts: Option<&'a Path>,
    /// Folders outside the bot's own that it works in (`~/Developer`, …).
    pub trusted_paths: &'a [std::path::PathBuf],
    /// The served release builds and their staging folder, as configured
    /// and resolved: only the daemon writes there (CE-010 M3).
    pub served: &'a [std::path::PathBuf],
    pub repo_url: Option<&'a str>,
    pub port: u16,
    /// The command that runs the guard hook.
    pub guard_command: String,
    /// The owner's own lines for the classifier (`auto_mode_environment`
    /// and the project's entry in `project_auto_mode_environment`).
    pub extra_environment: &'a [String],
    /// The hand-applied settings file this replaces, folded in so nothing
    /// the owner tuned there is lost.
    pub interim: Option<&'a Value>,
}

/// An absolute path as a permission rule path: `//Users/x` on Unix,
/// `//c/Users/x` on Windows. A single leading `/` would mean "relative to
/// the settings file", not absolute.
pub fn rule_path(path: &Path) -> String {
    let text = path.display().to_string().replace('\\', "/");
    match text.split_once(":/") {
        Some((drive, rest)) if drive.len() == 1 => format!("//{}/{rest}", drive.to_lowercase()),
        _ => format!("/{text}"),
    }
}

/// The tools the guard hook sees: every one that runs a command or names a
/// path to read or write.
pub const GUARD_MATCHER: &str = "Bash|Read|Grep|Glob|Write|Edit|MultiEdit|NotebookEdit";

pub fn generate(input: &SettingsInput<'_>) -> Value {
    let mut deny = hard_deny(input);
    let mut allow = artifacts_allow(input.artifacts);
    if input.profile != PermissionProfile::Standard {
        allow.extend(TRUSTED_ALLOW.iter().map(|r| (*r).to_string()));
        for extra in input.extras {
            allow.extend(extra_allow(*extra, input.workspace));
        }
    }
    if !input.extras.contains(&PermissionExtra::DaemonRestart) {
        deny.push("Bash(launchctl *)".to_string());
    }
    if !input.extras.contains(&PermissionExtra::ReleaseMain) {
        // These fail fast; the guard is the real check (`git -C . push`).
        deny.extend(MAIN_DENY.iter().map(|r| (*r).to_string()));
    }
    if input.extras.contains(&PermissionExtra::Publish) {
        // DevOps may run its scripts but never rewrite them.
        for script in ["serve.sh", "publish.sh"] {
            deny.push(format!(
                "Edit({})",
                rule_path(&input.workspace.join("serve").join(script))
            ));
        }
    }
    let mut environment = environment(input);
    for line in input.extra_environment {
        if !environment.contains(line) {
            environment.push(line.clone());
        }
    }
    if let Some(interim) = input.interim {
        merge_strings(&mut environment, &interim["autoMode"]["environment"]);
        merge_strings(&mut deny, &interim["permissions"]["deny"]);
        merge_strings(&mut allow, &interim["permissions"]["allow"]);
    }
    let mut settings = json!({
        "autoMode": {
            "environment": environment,
            "soft_deny": [
                "$defaults",
                "Running `xcrun simctl` with `all` or `booted` instead of the bot's own simulator UDID."
            ]
        },
        "permissions": { "allow": allow, "deny": deny },
        "hooks": {
            "PreToolUse": [{
                "matcher": GUARD_MATCHER,
                "hooks": [{ "type": "command", "command": input.guard_command }]
            }]
        }
    });
    if input.profile == PermissionProfile::Full {
        // An unattended pty cannot answer the one-time bypass warning.
        settings["skipDangerousModePermissionPrompt"] = json!(true);
    }
    settings
}

/// What the bot's `--permission-mode` is, when the profile sets one.
pub fn mode(profile: PermissionProfile) -> Option<&'static str> {
    match profile {
        PermissionProfile::Standard => None,
        PermissionProfile::Trusted => Some("auto"),
        PermissionProfile::Full => Some("bypassPermissions"),
    }
}

/// Add every string in `extra` that is not already there, keeping order.
fn merge_strings(into: &mut Vec<String>, extra: &Value) {
    for value in extra
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
    {
        if !into.iter().any(|existing| existing == value) {
            into.push(value.to_string());
        }
    }
}

fn environment(input: &SettingsInput<'_>) -> Vec<String> {
    let brand = crate::brand::DISPLAY_NAME;
    let bots = input.home.join("projects");
    let mut trusted = vec![format!(
        "every bot's own workspace under {}/<project>/bots/<bot>/workspace",
        bots.display()
    )];
    trusted.extend(
        input
            .trusted_paths
            .iter()
            .map(|p| format!("{} and its git worktrees", p.display())),
    );
    if let Some(artifacts) = input.artifacts {
        trusted.push(format!(
            "the project's shared artifacts at {}",
            artifacts.display()
        ));
    }
    let mut lines = vec![
        "$defaults".to_string(),
        format!(
            "Organization: {brand}, the owner's personal multi-agent app. This bot works in the project \"{}\".",
            input.project_name
        ),
        format!("Trusted local paths: {}.", trusted.join("; ")),
        "Host containment: the owner's own computer. Many bots run as the same user and share the disk; another bot's workspace, simulator, process or port is a neighbour that must not be touched.".to_string(),
        format!(
            "Key internal services: the {brand} service on 127.0.0.1:{}, which bots reach over MCP.",
            input.port
        ),
        format!(
            "Sensitive data locations: {}, ~/.ssh, ~/.claude.json, and the owner's personal files outside the trusted paths.",
            input.home.join("secrets").display()
        ),
    ];
    if let Some(url) = input.repo_url {
        lines.push(format!("Source control: {url} and its branches."));
    }
    lines
}

fn hard_deny(input: &SettingsInput<'_>) -> Vec<String> {
    let home = rule_path(input.home);
    let home_name = input
        .home
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut deny = vec![
        format!("Read({home}/secrets/**)"),
        format!("Edit({home}/secrets/**)"),
        format!("Bash(*{home_name}/secrets*)"),
        // A Read deny also covers Grep, Glob, Edit and Write (CE-003 M1).
        "Read(~/.ssh/**)".to_string(),
        "Read(~/.claude.json)".to_string(),
        "Edit(~/.claude/settings.json)".to_string(),
        "Edit(~/.claude/settings.local.json)".to_string(),
        "Edit(~/.claude.json)".to_string(),
        format!("Edit({home}/bot-settings.json)"),
        format!("Edit({home}/projects/**/.claude/settings.json)"),
        format!("Edit({home}/projects/**/.claude/settings.local.json)"),
        format!("Edit({home}/projects/**/settings.gen.json)"),
    ];
    deny.extend(
        super::CONFIG_FILES
            .iter()
            .map(|name| format!("Edit({home}/{name})")),
    );
    // Every bot, DevOps included: builds go in through the daemon only.
    deny.extend(
        input
            .served
            .iter()
            .map(|dir| format!("Edit({}/**)", rule_path(dir))),
    );
    deny.extend(STATIC_DENY.iter().map(|r| (*r).to_string()));
    deny
}

const STATIC_DENY: &[&str] = &[
    "Bash(git push --force*)",
    "Bash(git push -f *)",
    "Bash(git push * --force*)",
    "Bash(git push * -f)",
    "Bash(git push * +*)",
    "Bash(rm -rf /*)",
    "Bash(rm -rf ~*)",
    "Bash(*tailscale funnel *)",
    "Bash(*Tailscale funnel *)",
    // Connectors that spend credits or publish (H-031 §2).
    "mcp__claude_ai_Higgsfield",
    "mcp__claude_ai_Kaggle",
];

/// Pushes to `main`, for every bot without `release_main`.
const MAIN_DENY: &[&str] = &[
    "Bash(git push * main)",
    "Bash(git push * HEAD:main)",
    "Bash(git push * *:main)",
    "Bash(gh pr merge*)",
];

/// Routine build, test and housekeeping commands that skip the classifier in
/// Trusted and Full. Wildcarded interpreters and `pnpm run` would be dropped
/// by auto mode anyway, so they are not listed.
const TRUSTED_ALLOW: &[&str] = &[
    "Bash(cargo build)",
    "Bash(cargo build *)",
    "Bash(cargo test)",
    "Bash(cargo test *)",
    "Bash(cargo clippy *)",
    "Bash(cargo fmt *)",
    "Bash(cargo clean)",
    "Bash(pnpm install)",
    "Bash(pnpm install --frozen-lockfile)",
    "Bash(xcodebuild *)",
    "Bash(xcrun simctl *)",
    "Bash(git worktree add *)",
    "Bash(git worktree remove *)",
    "Bash(git worktree prune)",
    "Bash(df -h*)",
];

/// Reading and editing the project's shared artifacts, granted in every
/// profile (the folder sits outside the workspace).
fn artifacts_allow(artifacts: Option<&Path>) -> Vec<String> {
    artifacts
        .map(crate::paths::artifacts_allow_rules)
        .unwrap_or_default()
}

fn extra_allow(extra: PermissionExtra, workspace: &Path) -> Vec<String> {
    match extra {
        PermissionExtra::Publish => {
            // As Git Bash spells the command on Windows: `/`, never `\`.
            let serve = format!("{}/serve", workspace.display()).replace('\\', "/");
            let mut rules = Vec::new();
            for action in ["start", "stop", "status"] {
                rules.push(format!("Bash(serve/serve.sh {action})"));
                rules.push(format!("Bash({serve}/serve.sh {action})"));
            }
            rules.push("Bash(serve/publish.sh *)".to_string());
            rules.push(format!("Bash({serve}/publish.sh *)"));
            // The daemon checks the role and this extra again (H-020 §6.6).
            rules.push("Bash(hermesd release publish *)".to_string());
            rules
        }
        PermissionExtra::DaemonRestart => daemon_restart_allow(),
        PermissionExtra::AppRestart => vec![
            "Bash(scripts/dev.sh *)".to_string(),
            "Bash(./scripts/dev.sh *)".to_string(),
        ],
        // Lifts the main denies above; nothing to allow without review.
        PermissionExtra::ReleaseMain => Vec::new(),
        PermissionExtra::Install => {
            if cfg!(windows) {
                vec![
                    "PowerShell(Start-Process msiexec *)".to_string(),
                    "PowerShell(& .\\*-setup.exe *)".to_string(),
                ]
            } else {
                vec![
                    "Bash(ditto -xk * /Applications/*)".to_string(),
                    "Bash(xattr -dr com.apple.quarantine /Applications/*)".to_string(),
                    "Bash(open -a *)".to_string(),
                    "Bash(xcrun simctl install *)".to_string(),
                ]
            }
        }
    }
}

#[cfg(target_os = "macos")]
fn daemon_restart_allow() -> Vec<String> {
    // SAFETY: getuid has no preconditions and cannot fail.
    let uid = unsafe { libc::getuid() };
    vec![format!(
        "Bash(launchctl kickstart -k gui/{uid}/{})",
        crate::service::SERVICE_LABEL
    )]
}

#[cfg(not(target_os = "macos"))]
fn daemon_restart_allow() -> Vec<String> {
    Vec::new()
}
