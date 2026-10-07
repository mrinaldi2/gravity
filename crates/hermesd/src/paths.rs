//! On-disk provisioning of projects and bots per the architecture plan §4.4.
//! Versioned files are the source of truth for user-editable configuration;
//! SQLite owns runtime and delivery state.
//!
//! Two files carry a bot's prompt, and the split matters:
//!
//! - `system.md` is **daemon-owned**. It holds the name, description and
//!   instructions, is regenerated from the database on every identity change,
//!   and is injected with `--append-system-prompt` at spawn.
//! - `workspace/CLAUDE.md` is **bot-owned**. It is written once at creation and
//!   never rewritten, because the bot maintains it as living context. It
//!   imports `workspace/FACTS.md`, seeded the same way, which is where the bot
//!   parks facts that must outlive a compaction.
//!
//! Before the split, instructions lived in `CLAUDE.md`: editing them either did
//! nothing (the daemon only wrote the file at creation) or would have clobbered
//! the bot's own memory. Keeping the two apart is what makes instructions
//! editable at all.

use std::fs::{self, OpenOptions};
use std::io::Write;
#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

use anyhow::Context;
use serde::{Deserialize, Serialize};

use crate::config::Config;
use ipc_hooks::ipc_hooks;

mod ipc_hooks;
pub mod no_follow;
mod own;
mod prompt;
mod prompt_sections;
#[cfg(test)]
mod tests;
mod trust;
#[cfg(unix)]
mod unix_hooks;
#[cfg(windows)]
mod windows_hooks;

pub use own::own_workspace;
pub use prompt::system_md;
pub use trust::{claude_config_path, trust_workspace, ConfigLocked};

#[derive(Debug, Serialize, Deserialize)]
pub struct BotFile {
    pub version: u32,
    pub id: String,
    pub name: String,
    /// Directory this bot lives in. Recorded because a rename changes `name`
    /// but deliberately leaves the directory alone.
    pub dir_name: String,
    pub project: String,
    pub created_at: String,
}

pub struct BotDirs {
    pub root: PathBuf,
    pub workspace: PathBuf,
}

/// Root directory for a project, keyed on its immutable `dir_name` rather than
/// its current name: renaming a project must not move the bots inside it.
pub fn project_dir(cfg: &Config, project_dir_name: &str) -> PathBuf {
    cfg.projects_dir().join(project_dir_name)
}

/// Root directory for a bot, keyed on its immutable `dir_name` rather than its
/// current name.
pub fn bot_dir(cfg: &Config, project_dir_name: &str, dir_name: &str) -> PathBuf {
    project_dir(cfg, project_dir_name)
        .join("bots")
        .join(dir_name)
}

/// Shared, project-scoped directory every bot may read and write. Substance
/// travels here as files; message bodies carry summaries and paths.
pub fn artifacts_dir(cfg: &Config, project_dir_name: &str) -> PathBuf {
    project_dir(cfg, project_dir_name).join("artifacts")
}

pub fn provision_project(
    cfg: &Config,
    project_id: &str,
    dir_name: &str,
    name: &str,
) -> anyhow::Result<()> {
    let dir = project_dir(cfg, dir_name);
    fs::create_dir_all(dir.join("bots"))?;
    fs::create_dir_all(dir.join("artifacts"))?;
    write_project_manifest(cfg, project_id, dir_name, name)
}

/// (Re)write `project.json`. Also called after a rename, so the manifest on
/// disk never disagrees with the database about what the project is called.
pub fn write_project_manifest(
    cfg: &Config,
    project_id: &str,
    dir_name: &str,
    name: &str,
) -> anyhow::Result<()> {
    let dir = project_dir(cfg, dir_name);
    let manifest = serde_json::json!({
        "version": 1, "id": project_id, "name": name, "dir_name": dir_name
    });
    atomic_write_json(&dir.join("project.json"), &manifest)
}

/// Update the `project` field of a bot's `bot.json` in place.
///
/// Rewriting the whole file would reset `created_at`, which is the one field
/// only the original provisioning knows. A bot whose file is missing or
/// unreadable is skipped: the manifest is descriptive, and a rename should not
/// fail over it.
pub fn set_bot_project_name(root: &Path, project_name: &str) -> anyhow::Result<()> {
    // Read through no link: a `bot.json` linked to an owner file would
    // otherwise be copied into the bot's folder (H-182).
    let Some(text) = no_follow::read(root, Path::new("bot.json")) else {
        return Ok(());
    };
    let Ok(mut manifest) = serde_json::from_str::<serde_json::Value>(&text) else {
        return Ok(());
    };
    let Some(object) = manifest.as_object_mut() else {
        return Ok(());
    };
    object.insert("project".to_string(), project_name.into());
    no_follow::write_json(root, Path::new("bot.json"), &manifest)
}

/// Everything the daemon needs to lay out or refresh a bot's files.
pub struct BotProvision<'a> {
    pub project_name: &'a str,
    /// The project's frozen directory name; `project_name` is display only.
    pub project_dir_name: &'a str,
    pub bot_id: &'a str,
    pub name: &'a str,
    pub dir_name: &'a str,
    pub description: &'a str,
    pub instructions: &'a str,
    pub daemon_port: u16,
    pub bot_token_env: &'a str,
    /// Stated verbatim in the generated prompt so bots can budget against it.
    pub max_bots_per_project: usize,
    /// Workers the project may run at once on this machine.
    pub max_workers_per_project: usize,
    /// Whether this bot is a temporary worker, here for one task.
    pub temporary: bool,
    /// The project's shared git repository, when it has one.
    pub repo: Option<bus::ProjectRepo>,
    /// Absolute path of the project's shared artifacts directory, stated in
    /// the prompt so bots know where substance goes.
    pub artifacts_dir: String,
    /// The peers this project is linked through, by name. Empty when the
    /// team lives on this machine alone.
    pub linked_machines: Vec<String>,
    /// Whether the bot has a browser of its own (see `crate::browser`), and
    /// whether it may also drive the owner's Chrome.
    pub own_browser: bool,
    pub user_chrome: bool,
    /// The task limits this machine enforces for the project, stated in
    /// the prompt as numbers (ARCH-R59 b).
    pub task_limits: crate::config::TaskLimits,
}

/// Create the bot's directory tree: `bot.json`, `system.md`, `mcp.json`, and a
/// workspace with `CLAUDE.md`, `FACTS.md` and cooperative `.claude/settings.json`
/// (permissions plus lifecycle hooks that report state to the daemon).
pub fn provision_bot(cfg: &Config, spec: &BotProvision<'_>) -> anyhow::Result<BotDirs> {
    let root = bot_dir(cfg, spec.project_dir_name, spec.dir_name);
    let workspace = root.join("workspace");
    fs::create_dir_all(&root)?;
    // Below the bot's folder, nothing is written through a link (H-182).
    no_follow::create_dirs(&root, Path::new("workspace/.claude/skills"))?;

    let bot_file = BotFile {
        version: 1,
        id: spec.bot_id.to_string(),
        name: spec.name.to_string(),
        dir_name: spec.dir_name.to_string(),
        project: spec.project_name.to_string(),
        created_at: chrono::Utc::now().to_rfc3339(),
    };
    no_follow::write_json(
        &root,
        Path::new("bot.json"),
        &serde_json::to_value(&bot_file)?,
    )?;

    write_system_md(&root, spec)?;

    // The browser entry is added at spawn, when the session's config is
    // refreshed with whatever Node and Chrome the machine has then.
    let bus = crate::bus_auth::server_entry(cfg);
    write_mcp_config(&root, &bus, None)?;

    seed_memory_files(&workspace, spec.name)?;

    write_hook_settings(&workspace, &crate::bus_auth::hook_transport(cfg))?;

    Ok(BotDirs { root, workspace })
}

/// Keeps bots out of the daemon's secrets wherever the home lives, including
/// through the `~/.gravity` symlink the home migration leaves behind.
pub(crate) fn secrets_deny_rule() -> String {
    format!("Read(~/{}/secrets/**)", crate::brand::HOME_DIR_NAME)
}

pub(crate) fn legacy_secrets_deny_rule() -> String {
    format!("Read(~/{}/secrets/**)", crate::brand::LEGACY_HOME_DIR_NAME)
}

/// (Re)write the bot's `mcp.json`. Called at creation and again on every start,
/// so a change to the daemon's port, or to how bots reach the bus (H-044),
/// reaches existing bots without re-provisioning — the same reason
/// `write_hook_settings` runs each start. `bus` is the bus server's entry,
/// from `bus_auth::server_entry`.
pub fn write_mcp_config(
    root: &Path,
    bus: &serde_json::Value,
    browser: Option<&serde_json::Value>,
) -> anyhow::Result<()> {
    let mut mcp = serde_json::json!({ "mcpServers": {} });
    mcp["mcpServers"][crate::brand::ACTIVE_MCP_SERVER] = bus.clone();
    // The bot's own browser, when one can be started; see `crate::browser`.
    if let Some(browser) = browser {
        let mut server = browser.clone();
        server["type"] = serde_json::json!("stdio");
        mcp["mcpServers"][crate::browser::setup::SERVER] = server;
    }
    no_follow::write_json(root, Path::new("mcp.json"), &mcp)
}

/// Rewrite `system.md` after an identity change. Cheap and idempotent, so it is
/// also safe to run for every bot at startup to migrate rows provisioned before
/// instructions moved into this file.
pub fn write_system_md(root: &Path, spec: &BotProvision<'_>) -> anyhow::Result<()> {
    let content = prompt::system_md(spec);
    let path = root.join("system.md");
    // Skip the write when nothing changed, so startup regeneration does not
    // churn mtimes across every bot on every boot.
    if let Ok(existing) = fs::read_to_string(&path) {
        if existing == content {
            return Ok(());
        }
    }
    fs::create_dir_all(root)?;
    no_follow::write(root, Path::new("system.md"), content.as_bytes())
}

/// Seed the bot-owned memory files: `CLAUDE.md` (living context) and the
/// `FACTS.md` it imports. Each is written only when missing, so this is safe to
/// call on every start — that is how a bot provisioned before `FACTS.md`
/// existed gets one without its own writing ever being clobbered.
pub fn seed_memory_files(workspace: &Path, name: &str) -> anyhow::Result<()> {
    for (file, body) in [
        ("CLAUDE.md", prompt::claude_md(name)),
        ("FACTS.md", prompt::facts_md(name)),
    ] {
        // Only when nothing is there, a link included (H-182).
        no_follow::create_new(workspace, Path::new(file), body.as_bytes())?;
    }
    Ok(())
}

/// (Re)write the workspace's cooperative `.claude/settings.json`. Also called
/// on every bot start so existing bots pick up hook/setting changes.
pub fn write_hook_settings(
    workspace: &Path,
    transport: &crate::bus_auth::HookTransport,
) -> anyhow::Result<()> {
    // Never through a `.claude` the bot linked elsewhere, such as the owner's
    // `~/.claude` (H-182).
    no_follow::create_dirs(workspace, Path::new(".claude"))?;
    #[cfg(unix)]
    let settings = unix_hooks::settings(transport);
    #[cfg(windows)]
    let settings = windows_hooks::settings(workspace, transport)?;
    no_follow::write_json(workspace, Path::new(".claude/settings.json"), &settings)
}

/// Permission rules granting access to the project's shared artifacts
/// directory — the one place outside the workspace a bot may read and write.
/// They go into the bot's generated `--settings` file (H-031), never the
/// workspace's `settings.json`: allow rules there make Claude Code's trust
/// dialog warn about pre-approved permissions.
///
/// The paths are anchored with `//` (absolute). A single leading `/` means
/// "relative to the settings file", so the rules used to match nothing.
/// `Edit(path)` covers every file-editing tool, `Write` included.
pub fn artifacts_allow_rules(artifacts_dir: &Path) -> Vec<String> {
    let dir = crate::bot_permissions::rule_path(artifacts_dir);
    vec![format!("Read({dir}/**)"), format!("Edit({dir}/**)")]
}

pub fn atomic_write_json(path: &Path, value: &serde_json::Value) -> anyhow::Result<()> {
    let tmp = path.with_extension("json.tmp");
    let data = serde_json::to_string_pretty(value)?;
    fs::write(&tmp, data).with_context(|| format!("writing {}", tmp.display()))?;
    fs::rename(&tmp, path).with_context(|| format!("renaming into {}", path.display()))?;
    Ok(())
}

pub(super) fn atomic_write_private_json(
    path: &Path,
    value: &serde_json::Value,
) -> anyhow::Result<()> {
    let tmp = path.with_extension(format!("json.tmp-{}", uuid::Uuid::new_v4()));
    let result = (|| -> anyhow::Result<()> {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        options.mode(0o600);
        let mut file = options
            .open(&tmp)
            .with_context(|| format!("creating {}", tmp.display()))?;
        crate::permissions::private(&tmp, false)?;
        serde_json::to_writer_pretty(&mut file, value)
            .with_context(|| format!("writing {}", tmp.display()))?;
        file.write_all(b"\n")
            .with_context(|| format!("writing {}", tmp.display()))?;
        file.sync_all()
            .with_context(|| format!("syncing {}", tmp.display()))?;
        fs::rename(&tmp, path)
            .with_context(|| format!("renaming {} to {}", tmp.display(), path.display()))?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}
