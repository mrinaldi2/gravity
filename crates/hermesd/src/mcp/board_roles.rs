//! `role_set` for the lead (H-099): the roles the lead may give, never
//! reviewer.ce and never a reviewer role to itself (H-268 ARCH M1).

use std::sync::Arc;

use bus::contract::board as c;
use serde_json::{json, Value};

use crate::app::AppState;
use crate::board::team::{set_role, Assigner};

use super::board::{out, Me};

pub(super) fn role_set(app: &Arc<AppState>, me: &Me, req: c::RoleSet) -> anyhow::Result<Value> {
    let project = me.bot.project_id.as_str();
    set_role(app, project, &req, Assigner::Lead(&me.bot.id))?;
    let roles: Vec<c::ProjectRole> = app
        .db
        .project_roles(project)?
        .into_iter()
        .map(Into::into)
        .collect();
    Ok(json!({ "roles": out(roles)? }))
}
