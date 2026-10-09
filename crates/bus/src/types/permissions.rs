//! How much a project's bots may do without asking (H-031). The daemon turns
//! a profile plus each bot's extras into the bot's `--settings` file.

use serde::{Deserialize, Serialize};

/// A project's permission profile. Projects without one are `Standard`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionProfile {
    /// Claude Code's own auto mode, with the project's context and the hard
    /// deny-list: what bots had before profiles existed, minus false blocks.
    #[default]
    Standard,
    /// Auto mode plus narrow allow-rules for routine build, test and git
    /// work, so bots stop asking for what they were asked to do.
    Trusted,
    /// Bypass mode: only the deny-list and the guard hook stop a bot. Meant
    /// for bots in a container, VM or dedicated user account.
    Full,
}

impl PermissionProfile {
    pub const ALL: [PermissionProfile; 3] = [Self::Standard, Self::Trusted, Self::Full];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Standard => "standard",
            Self::Trusted => "trusted",
            Self::Full => "full",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|p| p.as_str() == text)
    }
}

/// A power granted to one bot on top of its project's profile. Never granted
/// fleet-wide: an allow there would hand it to every bot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionExtra {
    /// Run its own `serve/serve.sh` and `serve/publish.sh` (DevOps).
    Publish,
    /// Restart the Hermes service through launchd (DevOps). Every other
    /// bot is denied `launchctl` outright.
    DaemonRestart,
    /// Restart its own development app with `scripts/dev.sh`.
    AppRestart,
    /// Install a build into /Applications or its own simulator (testers).
    Install,
    /// Push and merge to `main` (DevOps only). Every other bot is denied
    /// that, by rule and by the guard, in every profile.
    ReleaseMain,
    /// Pause every project on this computer for an install (H-117). The
    /// daemon also requires DevOps or `install`, and an approved release.
    Quiesce,
    /// Build the Windows installer with `hermesd release build-installer`,
    /// which runs the repo's own script only as committed (H-117 X3, H-104).
    BuildInstallers,
    /// Fast-forward `main` to a mergeable PR's head with `hermesd pr merge`
    /// (H-284), on the daemon's `pr_merge` task. Off for every bot until the
    /// owner grants it; a raw push to `main` stays refused.
    PrMerge,
}

impl PermissionExtra {
    pub const ALL: [PermissionExtra; 8] = [
        Self::Publish,
        Self::DaemonRestart,
        Self::AppRestart,
        Self::Install,
        Self::ReleaseMain,
        Self::Quiesce,
        Self::BuildInstallers,
        Self::PrMerge,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Publish => "publish",
            Self::DaemonRestart => "daemon_restart",
            Self::AppRestart => "app_restart",
            Self::Install => "install",
            Self::ReleaseMain => "release_main",
            Self::Quiesce => "quiesce",
            Self::BuildInstallers => "build_installers",
            Self::PrMerge => "pr_merge",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|e| e.as_str() == text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spellings_round_trip_and_match_serde() {
        for profile in PermissionProfile::ALL {
            assert_eq!(PermissionProfile::parse(profile.as_str()), Some(profile));
            assert_eq!(serde_json::to_value(profile).unwrap(), profile.as_str());
        }
        for extra in PermissionExtra::ALL {
            assert_eq!(PermissionExtra::parse(extra.as_str()), Some(extra));
            assert_eq!(serde_json::to_value(extra).unwrap(), extra.as_str());
        }
        assert_eq!(PermissionProfile::default(), PermissionProfile::Standard);
    }
}
