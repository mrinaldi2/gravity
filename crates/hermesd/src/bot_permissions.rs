//! Permission profiles (H-031): what each bot may do without asking.
//!
//! At every start the daemon writes `bots/<bot>/settings.gen.json` from the
//! project's profile and the bot's extras, and launches the bot with
//! `--settings <that file>` and, for Trusted and Full, `--permission-mode`.
//! The file sits outside the workspace and is Edit-denied, so a bot cannot
//! widen its own permissions. Project-scope `.claude/settings.json` can't
//! carry modes or `autoMode` at all, which is why this goes through the CLI.

use std::path::{Path, PathBuf};
use std::sync::Once;

use bus::{PermissionExtra, PermissionProfile};
use serde_json::Value;

pub mod guard;
mod settings;
mod shell;
#[cfg(test)]
mod tests;

pub use settings::{generate, mode, rule_path, SettingsInput};

/// The generated settings file's name, in the bot's directory.
pub const SETTINGS_FILE: &str = "settings.gen.json";

/// A bot's stored permission settings, read at each start.
pub struct Stored {
    pub profile: PermissionProfile,
    pub extras: Vec<PermissionExtra>,
    pub repo_url: Option<String>,
}

impl Stored {
    pub fn load(db: &crate::db::Db, bot: &bus::Bot) -> anyhow::Result<Self> {
        Ok(Self {
            profile: db.project_permission_profile(&bot.project_id)?,
            extras: db.bot_permission_extras(&bot.id)?,
            repo_url: db.project_repo(&bot.project_id)?.map(|r| r.url),
        })
    }
}

/// What the daemon knows when it starts one bot.
pub struct BotStart<'a> {
    pub cfg: &'a crate::config::Config,
    pub profile: PermissionProfile,
    pub extras: &'a [PermissionExtra],
    pub project_name: &'a str,
    pub bot_root: &'a Path,
    pub workspace: &'a Path,
    pub artifacts: Option<&'a Path>,
    pub repo_url: Option<&'a str>,
}

impl BotStart<'_> {
    /// The trusted paths from config, with `~` expanded.
    pub fn trusted_paths(&self) -> Vec<PathBuf> {
        self.cfg
            .trusted_paths
            .iter()
            .map(|p| match p.strip_prefix("~/") {
                Some(rest) => self.cfg.user_home.join(rest),
                None => PathBuf::from(p),
            })
            .collect()
    }

    /// The guard hook's command line: this daemon binary, told whose folders
    /// are whose.
    pub fn guard_command(&self) -> String {
        let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("hermesd"));
        let mut parts = vec![
            quote(&exe.display().to_string()),
            "guard".to_string(),
            "--home".to_string(),
            quote(&self.cfg.home.display().to_string()),
            "--user-home".to_string(),
            quote(&self.cfg.user_home.display().to_string()),
        ];
        let writable = std::iter::once(self.bot_root.to_path_buf())
            .chain(self.artifacts.map(Path::to_path_buf))
            .chain(self.trusted_paths());
        for dir in writable {
            parts.push("--writable".to_string());
            parts.push(quote(&dir.display().to_string()));
        }
        parts.join(" ")
    }

    /// Write the settings file and return the CLI arguments that apply it,
    /// on top of `base` (the configured `claude_args`), from which a
    /// hand-applied `--settings` pair is removed and folded in instead.
    pub fn claude_args(&self, base: &[String]) -> anyhow::Result<Vec<String>> {
        let (mut args, interim) = without_interim_settings(base);
        if let Some(artifacts) = self.artifacts {
            // Outside the workspace, so the session needs it as a directory.
            args.push("--add-dir".to_string());
            args.push(artifacts.display().to_string());
        }
        let trusted = self.trusted_paths();
        let settings = generate(&SettingsInput {
            profile: self.profile,
            extras: self.extras,
            project_name: self.project_name,
            home: &self.cfg.home,
            workspace: self.workspace,
            artifacts: self.artifacts,
            trusted_paths: &trusted,
            repo_url: self.repo_url,
            port: self.cfg.port,
            guard_command: self.guard_command(),
            interim: interim.as_ref(),
        });
        let path = self.bot_root.join(SETTINGS_FILE);
        crate::paths::atomic_write_json(&path, &settings)?;
        args.push("--settings".to_string());
        args.push(path.display().to_string());
        if let Some(mode) = mode(self.profile) {
            args.push("--permission-mode".to_string());
            args.push(mode.to_string());
        }
        Ok(args)
    }
}

/// The configured args without a hand-applied `--settings <file>` (the
/// pre-H-031 setup), and that file's contents when it could be read. Two
/// `--settings` would fight; the profile's file carries the old one's rules.
pub fn without_interim_settings(base: &[String]) -> (Vec<String>, Option<Value>) {
    static NOTED: Once = Once::new();
    let mut args = Vec::with_capacity(base.len());
    let mut interim = None;
    let mut iter = base.iter();
    while let Some(arg) = iter.next() {
        let path = if arg == "--settings" {
            iter.next().cloned()
        } else if let Some(path) = arg.strip_prefix("--settings=") {
            Some(path.to_string())
        } else {
            args.push(arg.clone());
            continue;
        };
        if let Some(path) = path {
            NOTED.call_once(|| {
                tracing::info!(
                    %path,
                    "claude_args carries --settings; its rules are folded into each bot's generated \
                     settings. It can be removed from the config"
                );
            });
            interim = std::fs::read_to_string(&path)
                .ok()
                .and_then(|text| serde_json::from_str(&text).ok());
        }
    }
    (args, interim)
}

fn quote(text: &str) -> String {
    if cfg!(windows) {
        format!("\"{text}\"")
    } else {
        format!("'{}'", text.replace('\'', r"'\''"))
    }
}
