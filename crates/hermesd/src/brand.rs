//! Every name the daemon shows, registers or reads, in one place.
//!
//! The project is The Hermes. Names that live on disk or in running bots'
//! history keep their pre-rename values until the home migration moves them;
//! those are the `LEGACY_*` and on-disk constants below, and nothing else in
//! the daemon spells them out.

use std::ffi::OsString;

/// The product name, for titles and anything the owner reads once.
pub const DISPLAY_NAME: &str = "The Hermes";

/// The name in running copy ("watch it from Hermes", "[Hermes] restarting").
pub const SHORT_NAME: &str = "Hermes";

/// Machine-facing identifier, e.g. the Codex client name.
pub const SLUG: &str = "the-hermes";

/// Daemon environment variables are read under this prefix first.
pub const ENV_PREFIX: &str = "THEHERMES_";

/// Read when the `ENV_PREFIX` variable is unset, so existing setups keep working.
pub const LEGACY_ENV_PREFIX: &str = "GRAVITY_";

/// The bus's MCP server name after the switch.
pub const MCP_SERVER: &str = "hermes-bus";

/// The bus's MCP server name before the rename. Transcripts written under it
/// stay readable forever.
pub const LEGACY_MCP_SERVER: &str = "gravity-bus";

/// The name bots are registered under: `mcp.json`, Codex's config, the MCP
/// `serverInfo` and the system prompt all use it.
pub const ACTIVE_MCP_SERVER: &str = MCP_SERVER;

/// Tool-name prefixes of the bus in transcripts, newest first. Sessions from
/// before the rename keep `mcp__gravity-bus__*` calls forever, so readers
/// accept both.
pub const BUS_TOOL_PREFIXES: [&str; 2] = ["mcp__hermes-bus__", "mcp__gravity-bus__"];

/// The bus tool a transcript's tool name refers to, under either name.
pub fn bus_tool(name: &str) -> Option<&str> {
    BUS_TOOL_PREFIXES
        .iter()
        .find_map(|prefix| name.strip_prefix(prefix))
}

/// The daemon home under the user's home directory.
pub const HOME_DIR_NAME: &str = ".thehermes";

/// The home before the rename. `hermesd migrate-home` moves it to
/// [`HOME_DIR_NAME`] and leaves a symlink here for one release.
pub const LEGACY_HOME_DIR_NAME: &str = ".gravity";

/// Stem of the daemon's own files in its home: `hermesd.toml`, `.port`,
/// `.lock`, `.reclaims`, `logs/hermesd.{out,err}.log`, `hermesd-task.*`.
pub const DAEMON_FILE_STEM: &str = "hermesd";

/// The same files before the rename; the home migration renames them.
pub const LEGACY_DAEMON_FILE_STEM: &str = "gravityd";

/// The launchd agent label on macOS.
pub const LAUNCHD_LABEL: &str = "com.manuelrinaldi.thehermesd";

/// The label releases before the rename installed under. `service install`
/// unloads it before loading [`LAUNCHD_LABEL`], so the two never both run.
pub const LEGACY_LAUNCHD_LABEL: &str = "in.mikolajczuk.gravityd";

/// Prefix of the per-user scheduled task on Windows.
pub const WINDOWS_TASK: &str = "The Hermes";

/// The task prefix before the rename, removed by `service install`.
pub const LEGACY_WINDOWS_TASK: &str = "Gravity";

/// `<stem><suffix>` in the daemon home, e.g. `daemon_file(".toml")`.
pub fn daemon_file(suffix: &str) -> String {
    format!("{DAEMON_FILE_STEM}{suffix}")
}

/// The pre-rename name of [`daemon_file`].
pub fn legacy_daemon_file(suffix: &str) -> String {
    format!("{LEGACY_DAEMON_FILE_STEM}{suffix}")
}

/// Prefix of on-disk markers and branch names written into bots' repos and
/// workspaces (`.gravity-worktree.json`, `gravity/<bot>`). Existing worktrees
/// and branches in bots' repositories are recognised by it, and those live
/// outside the home, so the home migration leaves it as it is.
pub const ON_DISK_SLUG: &str = "gravity";

/// The variable a bot's session reads its bus token from. Bots also get
/// [`LEGACY_BOT_TOKEN_ENV`], so a config written before the rename still
/// authenticates until it is regenerated.
pub const BOT_TOKEN_ENV: &str = "THEHERMES_TOKEN";
pub const LEGACY_BOT_TOKEN_ENV: &str = "GRAVITY_TOKEN";

/// The token variables a bot session starts with: the current name and the
/// legacy one, carrying the same token.
pub fn bot_token_vars(token: String) -> Vec<(String, String)> {
    vec![
        (BOT_TOKEN_ENV.to_string(), token.clone()),
        (LEGACY_BOT_TOKEN_ENV.to_string(), token),
    ]
}

/// Author of the commits a worker saves, and its non-routable address.
pub const WORKER_GIT_SUFFIX: &str = "(Hermes worker)";
pub const WORKER_GIT_EMAIL: &str = "worker@thehermes.invalid";

/// `THEHERMES_<name>`.
pub fn env_name(name: &str) -> String {
    format!("{ENV_PREFIX}{name}")
}

/// `GRAVITY_<name>`, still honoured and still set for bots.
pub fn legacy_env_name(name: &str) -> String {
    format!("{LEGACY_ENV_PREFIX}{name}")
}

/// Reads `THEHERMES_<name>`, falling back to `GRAVITY_<name>`.
pub fn env_var_os(name: &str) -> Option<OsString> {
    read_env(name, |key| std::env::var_os(key))
}

fn read_env(name: &str, get: impl Fn(&str) -> Option<OsString>) -> Option<OsString> {
    get(&env_name(name)).or_else(|| get(&legacy_env_name(name)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<OsString> {
        let map: HashMap<String, OsString> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), OsString::from(v)))
            .collect();
        move |key| map.get(key).cloned()
    }

    #[test]
    fn the_new_prefix_wins_over_the_old_one() {
        let get = env(&[("THEHERMES_HOME", "/new"), ("GRAVITY_HOME", "/old")]);
        assert_eq!(read_env("HOME", get), Some(OsString::from("/new")));
    }

    #[test]
    fn the_old_prefix_still_works_alone() {
        let get = env(&[("GRAVITY_HOME", "/old")]);
        assert_eq!(read_env("HOME", get), Some(OsString::from("/old")));
        assert_eq!(read_env("HOME", env(&[])), None);
    }

    #[test]
    fn bus_tools_are_read_under_both_names() {
        assert_eq!(
            bus_tool("mcp__hermes-bus__send_message"),
            Some("send_message")
        );
        assert_eq!(
            bus_tool("mcp__gravity-bus__send_message"),
            Some("send_message")
        );
        assert_eq!(bus_tool("mcp__playwright__browser_click"), None);
        for (prefix, server) in BUS_TOOL_PREFIXES
            .iter()
            .zip([MCP_SERVER, LEGACY_MCP_SERVER])
        {
            assert_eq!(*prefix, format!("mcp__{server}__"));
        }
    }

    #[test]
    fn names_follow_the_prefixes() {
        assert_eq!(env_name("TOKEN"), "THEHERMES_TOKEN");
        assert_eq!(legacy_env_name("TOKEN"), "GRAVITY_TOKEN");
        assert_eq!(BOT_TOKEN_ENV, env_name("TOKEN"));
        assert_eq!(LEGACY_BOT_TOKEN_ENV, legacy_env_name("TOKEN"));
    }
}
