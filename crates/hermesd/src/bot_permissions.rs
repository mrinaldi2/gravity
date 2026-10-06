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

mod exact;
pub mod guard;
mod settings;
mod shell;
#[cfg(test)]
mod tests;

pub use exact::{command as hermesd_command, this_binary};
pub use settings::{generate, mode, rule_path, SettingsInput};

/// The generated settings file's name, in the bot's directory.
pub const SETTINGS_FILE: &str = "settings.gen.json";

/// A bot's stored permission settings, read at each start.
pub struct Stored {
    pub profile: PermissionProfile,
    pub extras: Vec<PermissionExtra>,
    pub repo_url: Option<String>,
    /// The project's DevOps here: the one bot without the installer deny
    /// rules (B8).
    pub devops: bool,
}

/// Full is not selectable yet (CE-004 (b)): the guard can't cover its own
/// absence, and nothing else reviews a Full bot's calls. The code stays for
/// when bots run as a separate user or in a VM; flip this then.
pub const FULL_ENABLED: bool = false;

/// The profile a bot actually runs with: a project stored as Full falls back
/// to Trusted while Full is disabled.
pub fn effective(profile: PermissionProfile) -> PermissionProfile {
    if profile == PermissionProfile::Full && !FULL_ENABLED {
        PermissionProfile::Trusted
    } else {
        profile
    }
}

impl Stored {
    pub fn load(db: &crate::db::Db, bot: &bus::Bot) -> anyhow::Result<Self> {
        let stored = db.project_permission_profile(&bot.project_id)?;
        let profile = effective(stored);
        if profile != stored {
            tracing::warn!(
                bot = %bot.name,
                "project is stored as Full, which is disabled; running as Trusted"
            );
        }
        Ok(Self {
            profile,
            extras: db.bot_permission_extras(&bot.id)?,
            repo_url: db.project_repo(&bot.project_id)?.map(|r| r.url),
            devops: db
                .project_roles(&bot.project_id)?
                .iter()
                .any(|r| r.role == crate::board::model::Role::Devops && r.bot_id == bot.id),
        })
    }
}

/// What the daemon knows when it starts one bot.
pub struct BotStart<'a> {
    pub cfg: &'a crate::config::Config,
    pub profile: PermissionProfile,
    pub extras: &'a [PermissionExtra],
    pub project_name: &'a str,
    /// The bot's name: its worktrees are `<repo>-wt-<slug>-*`.
    pub bot_name: &'a str,
    pub bot_root: &'a Path,
    pub workspace: &'a Path,
    pub artifacts: Option<&'a Path>,
    pub repo_url: Option<&'a str>,
    pub devops: bool,
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

    /// The served release builds and their staging folder, as configured
    /// and, where they exist, resolved.
    pub fn served_dirs(&self) -> Vec<PathBuf> {
        let root = crate::board::release::serve::served_root(self.cfg);
        let mut dirs = Vec::new();
        for dir in [crate::board::release::confine::staging_for(&root), root] {
            let real = dir.canonicalize().ok().filter(|real| *real != dir);
            dirs.push(dir);
            dirs.extend(real);
        }
        dirs
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
        // The trusted paths are the classifier's trust context, not a licence
        // to delete: only the bot's own `<repo>-wt-<bot>-*` worktrees are
        // (CE-003 M4, CE-004 F2).
        let writable = std::iter::once(self.bot_root.to_path_buf())
            .chain(self.artifacts.map(Path::to_path_buf))
            // Installing a build replaces the app in /Applications.
            .chain(
                (cfg!(target_os = "macos") && self.extras.contains(&PermissionExtra::Install))
                    .then(|| PathBuf::from("/Applications")),
            )
            .map(|dir| ("--writable", dir))
            .chain(self.served_dirs().into_iter().map(|dir| ("--served", dir)))
            .chain(
                self.trusted_paths()
                    .into_iter()
                    .map(|dir| ("--worktrees", dir)),
            );
        for (flag, dir) in writable {
            parts.push(flag.to_string());
            parts.push(quote(&dir.display().to_string()));
        }
        if !self.bot_name.is_empty() {
            parts.push("--bot".to_string());
            parts.push(quote(self.bot_name));
        }
        // DevOps cleans and resets the `<repo>-rel-*` release trees.
        if self.extras.contains(&PermissionExtra::Publish)
            || self.extras.contains(&PermissionExtra::ReleaseMain)
        {
            parts.push("--releases".to_string());
        }
        if self.profile == PermissionProfile::Full {
            parts.push("--full".to_string());
        }
        if self.extras.contains(&PermissionExtra::ReleaseMain) {
            parts.push("--allow-main".to_string());
        }
        parts.join(" ")
    }

    /// Write the settings file and return the CLI arguments that apply it,
    /// on top of `base` (the configured `claude_args`), from which a
    /// hand-applied `--settings` pair is removed and folded in instead.
    pub fn claude_args(&self, base: &[String]) -> anyhow::Result<Vec<String>> {
        let (mut args, interim) =
            without_interim_settings(base, &self.cfg.home, &self.cfg.user_home);
        if let Some(artifacts) = self.artifacts {
            // Outside the workspace, so the session needs it as a directory.
            args.push("--add-dir".to_string());
            args.push(artifacts.display().to_string());
        }
        let trusted = self.trusted_paths();
        let extra_environment: Vec<String> = self
            .cfg
            .auto_mode_environment
            .iter()
            .chain(
                self.cfg
                    .project_auto_mode_environment
                    .get(self.project_name)
                    .into_iter()
                    .flatten(),
            )
            .cloned()
            .collect();
        let settings = generate(&SettingsInput {
            profile: self.profile,
            extras: self.extras,
            project_name: self.project_name,
            home: &self.cfg.home,
            workspace: self.workspace,
            hermesd: &this_binary(),
            artifacts: self.artifacts,
            trusted_paths: &trusted,
            served: &self.served_dirs(),
            repo_url: self.repo_url,
            devops: self.devops,
            port: self.cfg.port,
            guard_command: self.guard_command(),
            extra_environment: &extra_environment,
            interim: interim.as_ref(),
        });
        let path = self.bot_root.join(SETTINGS_FILE);
        // Never through a link the bot left at that name (H-182).
        crate::paths::no_follow::write_json(self.bot_root, Path::new(SETTINGS_FILE), &settings)?;
        args.push("--settings".to_string());
        args.push(path.display().to_string());
        if let Some(mode) = mode(self.profile) {
            args.push("--permission-mode".to_string());
            args.push(mode.to_string());
        }
        Ok(args)
    }
}

/// The file the pre-H-031 setup applied by hand with `--settings`.
pub const INTERIM_SETTINGS_FILE: &str = "bot-settings.json";

/// The daemon's config in its home, under both names: `migrate-home` renames
/// `gravityd.toml` to `hermesd.toml`, and the lock must follow it. Both are
/// protected, so a home on either side of the move is covered. (R2 adds
/// `brand::daemon_file`; once both branches land this can derive from it.)
pub const CONFIG_FILES: [&str; 2] = ["gravityd.toml", "hermesd.toml"];

/// The daemon home's folder name on either side of `migrate-home`. For one
/// release the old name stays as a symlink to the new home, and the guard
/// protects paths spelled through it too (CE-006 G1).
pub const HOME_NAMES: [&str; 2] = [".gravity", ".thehermes"];

/// The configured args without a hand-applied `--settings <file>` (the
/// pre-H-031 setup), and that file's contents. Two `--settings` would fight;
/// the profile's file carries the old one's rules instead.
///
/// `<home>/bot-settings.json` is folded in even once the argument is gone,
/// so removing it never drops the owner's trust lines (CE-003 M2): they
/// move to `auto_mode_environment`, and the file can then be deleted.
pub fn without_interim_settings(
    base: &[String],
    home: &Path,
    user_home: &Path,
) -> (Vec<String>, Option<Value>) {
    static NOTED: Once = Once::new();
    let mut args = Vec::with_capacity(base.len());
    let mut named = None;
    let mut iter = base.iter();
    while let Some(arg) = iter.next() {
        if arg == "--settings" {
            named = iter.next().cloned().or(named);
        } else if let Some(path) = arg.strip_prefix("--settings=") {
            named = Some(path.to_string());
        } else {
            args.push(arg.clone());
        }
    }
    let path = match &named {
        Some(path) => match path.strip_prefix("~/") {
            Some(rest) => user_home.join(rest),
            None => PathBuf::from(path),
        },
        None => home.join(INTERIM_SETTINGS_FILE),
    };
    if named.is_none() && !path.exists() {
        return (args, None);
    }
    let interim = match std::fs::read_to_string(&path) {
        Ok(text) => match serde_json::from_str(&text) {
            Ok(value) => Some(value),
            Err(error) => {
                tracing::warn!(path = %path.display(), %error, "the interim settings file isn't valid JSON; its rules are not applied");
                None
            }
        },
        Err(error) => {
            tracing::warn!(path = %path.display(), %error, "can't read the interim settings file; its rules are not applied");
            None
        }
    };
    if interim.is_some() {
        NOTED.call_once(|| {
            tracing::info!(
                path = %path.display(),
                "the interim settings file is folded into each bot's generated settings. Move its \
                 autoMode.environment lines into auto_mode_environment in the config, then remove \
                 any --settings from claude_args and delete the file"
            );
        });
    }
    (args, interim)
}

pub(crate) fn quote(text: &str) -> String {
    if cfg!(windows) {
        format!("\"{text}\"")
    } else {
        format!("'{}'", text.replace('\'', r"'\''"))
    }
}
