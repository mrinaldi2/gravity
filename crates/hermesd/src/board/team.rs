//! The team's setup on a board (H-099): who holds which role, and how much
//! each column may hold. The owner sets both in the app; the lead assigns
//! roles over MCP (`role_set`). Each change is pushed, so open boards refetch.

use std::sync::Arc;

use bus::contract::board as c;

use super::feed::{Change, ChangeKind};
use super::model::{ProjectRole, Role};
use crate::app::AppState;

/// A bot of the project by id or name, stand-ins for a peer's bots included.
pub fn project_bot(app: &AppState, project_id: &str, name_or_id: &str) -> anyhow::Result<String> {
    let bots = app.db.list_bots(Some(project_id))?;
    bots.iter()
        .find(|b| b.id == name_or_id)
        .or_else(|| {
            bots.iter()
                .find(|b| b.name.eq_ignore_ascii_case(name_or_id))
        })
        .map(|b| b.id.clone())
        .ok_or_else(|| anyhow::anyhow!("no bot '{name_or_id}' in this project"))
}

/// The roles the lead may give or take away over MCP. Lead, DevOps and
/// tester are the owner's alone (ARCH-R30 M1).
const LEAD_ASSIGNS: [Role; 4] = [Role::Dev, Role::Coach, Role::ReviewerArch, Role::ReviewerUx];

/// Gives the bot the role, or takes it away. A tester's machine is updated
/// in place. The owner assigns any role; the lead only `LEAD_ASSIGNS`.
pub fn set_role(
    app: &Arc<AppState>,
    project_id: &str,
    req: &c::RoleSet,
    by_owner: bool,
) -> anyhow::Result<()> {
    let role = Role::from_wire(req.role)?;
    anyhow::ensure!(
        by_owner || LEAD_ASSIGNS.contains(&role),
        "only the owner assigns {}",
        role.as_str()
    );
    let bot_id = project_bot(app, project_id, req.bot.trim())?;
    anyhow::ensure!(
        app.db.board_settings(project_id)?.is_some(),
        "this project has no board yet"
    );
    let mut feed = app.board.writer();
    if req.remove == Some(true) {
        app.db.remove_project_role(project_id, role, &bot_id)?;
    } else {
        let machine = req
            .machine
            .as_deref()
            .map(str::trim)
            .filter(|m| !m.is_empty());
        anyhow::ensure!(
            machine.is_none() || role == Role::Tester,
            "only a tester has a machine"
        );
        app.db.set_project_role(&ProjectRole {
            project_id: project_id.to_string(),
            role,
            bot_id,
            machine: machine.map(str::to_string),
        })?;
    }
    feed.publish(changed(project_id, ChangeKind::SettingsChanged));
    Ok(())
}

/// Sets a column's WIP limit, or clears it. Zero is no limit too.
pub fn set_column_limit(
    app: &Arc<AppState>,
    project_id: &str,
    column_key: &str,
    limit: Option<u32>,
) -> anyhow::Result<()> {
    let limit = limit.filter(|l| *l > 0);
    let mut feed = app.board.writer();
    anyhow::ensure!(
        app.db.set_column_limit(project_id, column_key, limit)?,
        "no column '{column_key}' on this board"
    );
    feed.publish(changed(project_id, ChangeKind::ColumnsChanged));
    Ok(())
}

fn changed(project_id: &str, kind: ChangeKind) -> Change<'_> {
    Change {
        project_id,
        kind,
        item_id: "",
        card: None,
        from_column: None,
    }
}
