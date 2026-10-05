//! The release tools (H-020 §1.6, §2): DevOps assembles, submits, deploys
//! and rolls back; testers install and confirm; every bot can read. Owner
//! rulings never come through here: `release_rule` is a WebSocket request.

use std::sync::Arc;

use bus::contract::board as c;
use serde_json::{json, Value};

use crate::app::AppState;
use crate::board::model::Role;
use crate::board::release::assemble::{self, NewPackage};
use crate::board::release::model::{DeployResult, ReleaseBuild, Smoke};
use crate::board::release::{deploy, load, model::parse_arg, Caller};

use super::board_schema::{decode, shared, tool, Audience, BoardTool};

pub(super) const RELEASE_TOOLS: &[BoardTool] = &[
    tool("release_list", "ReleaseList", Audience::Everyone),
    tool("release_get", "ReleaseGet", Audience::Everyone),
    tool("release_create", "ReleaseCreate", Audience::Devops),
    tool(
        "release_attach_build",
        "ReleaseAttachBuild",
        Audience::Devops,
    ),
    shared(
        "release_submit",
        "ReleaseSubmit",
        Audience::Devops,
        "Freeze a release with its builds, move its items to Owner testing and ask the owner \
         to rule on it in the release review. Refused with every item that isn't ready.",
    ),
    tool("release_deploy", "ReleaseDeploy", Audience::Devops),
    tool("release_rollback", "ReleaseRollback", Audience::Devops),
    tool("install_release", "InstallRelease", Audience::Tester),
    tool("deploy_confirm", "DeployConfirm", Audience::TesterOrDevops),
];

pub(super) fn handles(name: &str) -> bool {
    RELEASE_TOOLS.iter().any(|t| t.name == name)
}

pub(super) fn call(
    app: &Arc<AppState>,
    bot: &bus::Bot,
    roles: Vec<Role>,
    name: &str,
    args: &Value,
) -> anyhow::Result<Value> {
    let me = Caller { bot, roles };
    let project = bot.project_id.as_str();
    let released = |r: crate::board::release::model::Release| Ok(json!({ "release": r.to_json() }));
    match name {
        "release_list" => {
            let _: c::ReleaseList = decode("ReleaseList", args, project)?;
            let all = app.db.board_read(|t| t.releases(project))?;
            Ok(json!({ "releases": all.iter().map(|r| r.to_json()).collect::<Vec<_>>() }))
        }
        "release_get" => {
            let req: c::ReleaseGet = decode("ReleaseGet", args, project)?;
            released(app.db.board_read(|t| load(t, project, &req.release_id))?)
        }
        "release_create" => {
            let req: c::ReleaseCreate = decode("ReleaseCreate", args, project)?;
            released(assemble::create(
                app,
                &me,
                &NewPackage {
                    name: &req.name,
                    display_version: req.display_version.as_deref(),
                    items: &req.items,
                    changelog: req.changelog.as_deref().unwrap_or_default(),
                    how_to_test: json!([]),
                },
            )?)
        }
        "release_attach_build" => {
            let req: c::ReleaseAttachBuild = decode("ReleaseAttachBuild", args, project)?;
            let build = ReleaseBuild {
                platform: req.platform.trim().to_string(),
                version: req.version.trim().to_string(),
                artifact: req.artifact.trim().to_string(),
                url: req.url,
                install_url: req.install_url,
                sha256: req.sha256.trim().to_string(),
                built_at: bus::now(),
            };
            released(assemble::attach_build(app, &me, &req.release_id, &build)?)
        }
        "release_submit" => {
            let req: c::ReleaseSubmit = decode("ReleaseSubmit", args, project)?;
            released(assemble::submit(app, &me, &req.release_id)?)
        }
        "release_deploy" => {
            let req: c::ReleaseDeploy = decode("ReleaseDeploy", args, project)?;
            released(deploy::deploy(
                app,
                &me,
                &req.release_id,
                req.machine.trim(),
            )?)
        }
        "release_rollback" => {
            let req: c::ReleaseRollback = decode("ReleaseRollback", args, project)?;
            released(deploy::rollback(
                app,
                &me,
                &req.release_id,
                req.machine.trim(),
            )?)
        }
        "install_release" => {
            let req: c::InstallRelease = decode("InstallRelease", args, project)?;
            deploy::install(app, &me, &req.release_id)
        }
        "deploy_confirm" => {
            let req: c::DeployConfirm = decode("DeployConfirm", args, project)?;
            let result = parse_arg("result", &req.result, DeployResult::ALL)?;
            let smoke = req
                .smoke
                .as_deref()
                .map(|s| parse_arg("smoke", s, Smoke::ALL))
                .transpose()?;
            released(deploy::confirm(
                app,
                &me,
                &req.release_id,
                req.machine.trim(),
                result,
                smoke,
                req.log_artifact.as_deref(),
            )?)
        }
        other => anyhow::bail!("unknown tool: {other}"),
    }
}
