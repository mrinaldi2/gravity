//! Who may pause this computer for a release's install besides the tester
//! holding the deploy task (H-166): the project's DevOps, while the release
//! has a deployment or rollback under way on this computer. DevOps swaps the
//! app itself when the tester can't, and needs the same pause first.

use std::sync::Arc;

use serde_json::{json, Value};

use crate::app::AppState;
use crate::board::model::Role;
use crate::decisions::forbidden;

use super::model::DeployAction;
use super::{deploy, load, machines, Caller};

/// The release's builds for this computer's open deployment, as
/// `install_release` answers them, when `me` is the project's DevOps.
pub fn devops_here(app: &Arc<AppState>, me: &Caller<'_>, release_id: &str) -> anyhow::Result<Value> {
    me.require(Role::Devops, "pause this computer for an install it doesn't hold")?;
    let (release, here) = app.db.board_read(|t| {
        Ok((
            load(t, &me.bot.project_id, release_id)?,
            machines::this_computer(t)?,
        ))
    })?;
    let Some(deployment) = release
        .deployments
        .iter()
        .find(|d| d.machine == here && d.result.is_none())
    else {
        return Err(forbidden(format!(
            "release {} has no deployment under way on {here}; DevOps pauses a computer only \
             for its own install",
            release.name
        )));
    };
    if deployment.action == DeployAction::Deploy {
        deploy::gate_open(app, &release)?;
    }
    Ok(json!({
        "release_id": release.id,
        "name": release.name,
        "machine": here,
        "action": deployment.action.as_str(),
        "builds": release.builds.iter().map(|b| json!({
            "platform": b.platform, "version": b.version, "sha256": b.sha256,
        })).collect::<Vec<_>>(),
    }))
}
