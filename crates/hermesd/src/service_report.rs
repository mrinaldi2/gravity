//! `hermesd service status --json`: the facts the desktop app reads on launch
//! to decide whether this machine's service needs installing, finishing or
//! repairing. Facts only; the app decides what they add up to.

use serde::Serialize;

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct Report {
    /// The managed binary, under its current or pre-rename name.
    pub binary: bool,
    /// This release's service definition: the launchd plist or the task.
    pub service: bool,
    /// Whether the current service has a running process; `None` where the
    /// platform cannot tell.
    pub service_running: Option<bool>,
    /// A service from before the rename that this install would take over.
    pub legacy_service: bool,
    pub legacy_running: Option<bool>,
    /// `service install` would move the home first.
    pub migration_pending: bool,
    /// The home was moved by a finished migration.
    pub migrated: bool,
    /// The port clients reach the daemon on.
    pub port: u16,
    /// What `/health` on `port` reports; `None` when nothing answers.
    pub version: Option<String>,
    /// How this hermesd build knows the owner's app (H-114).
    pub identity: crate::bus_auth::app_identity::Identity,
}

/// The parts of a [`Report`] that come from the home rather than the
/// service manager.
pub struct HomeState {
    pub migration_pending: bool,
    pub migrated: bool,
}
