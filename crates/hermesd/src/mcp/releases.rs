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
use crate::board::release::publish::{self, Publish};
use crate::board::release::{
    blockers, cancel, deploy, deployed_via, lifecycle, load, machines, model::parse_arg, package,
    plan, Caller,
};
use crate::db::NewReleaseTest;

use super::board_schema::{decode, shared, tool, Audience, BoardTool};

pub(super) const RELEASE_TOOLS: &[BoardTool] = &[
    tool("release_list", "ReleaseList", Audience::Everyone),
    tool("release_get", "ReleaseGet", Audience::Everyone),
    tool("release_create", "ReleaseCreate", Audience::Devops),
    tool("release_plan", "ReleasePlan", Audience::LeadOrDevops),
    tool("release_items", "ReleaseItems", Audience::LeadOrDevops),
    tool(
        "release_assemble",
        "ReleaseAssemble",
        Audience::LeadOrDevops,
    ),
    tool(
        "release_attach_build",
        "ReleaseAttachBuild",
        Audience::Devops,
    ),
    tool("release_publish", "ReleasePublish", Audience::Devops),
    tool("release_update", "ReleaseUpdate", Audience::Devops),
    // The lead may cancel a plan it made (H-137); a package is DevOps's.
    tool("release_cancel", "ReleaseCancel", Audience::LeadOrDevops),
    tool(
        "release_deployed_via",
        "ReleaseDeployedVia",
        Audience::LeadOrDevops,
    ),
    tool("release_test", "ReleaseTest", Audience::Tester),
    tool("release_machines", "ReleaseMachines", Audience::Everyone),
    tool("release_machines_set", "ReleaseMachinesSet", Audience::Lead),
    tool("machine_name_set", "MachineNameSet", Audience::Lead),
    shared(
        "release_submit",
        "ReleaseSubmit",
        Audience::Devops,
        "Freeze a release with its builds, move its items to Owner testing and ask the owner \
         to rule on it in the release review. Refused with every item that isn't ready.",
    ),
    tool("release_deploy", "ReleaseDeploy", Audience::Devops),
    tool("release_rollback", "ReleaseRollback", Audience::Devops),
    tool("release_pause", "ReleasePause", Audience::Devops),
    tool("release_resume", "ReleaseResume", Audience::Devops),
    tool("install_release", "InstallRelease", Audience::Tester),
    tool(
        "install_quiesce",
        "InstallQuiesce",
        Audience::TesterOrDevops,
    ),
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
    // Never `owner_blockers` to a bot: they name other bots' permission
    // prompts and the owner's questions (ARCH M1). Owner clients only.
    let payload = |r: &crate::board::release::model::Release| r.to_json();
    let released = |r: crate::board::release::model::Release| Ok(json!({ "release": payload(&r) }));
    match name {
        "release_list" => {
            let _: c::ReleaseList = decode("ReleaseList", args, project)?;
            let all = app.db.board_read(|t| t.releases(project))?;
            Ok(json!({ "releases": all.iter().map(payload).collect::<Vec<_>>() }))
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
                    from: req.from.as_deref(),
                },
            )?)
        }
        "release_plan" => {
            let req: c::ReleasePlan = decode("ReleasePlan", args, project)?;
            if let Some(card) = &req.work_item {
                blockers::own_card(app, project, card)?;
            }
            let release = plan::plan(
                app,
                &me,
                &plan::NewPlan {
                    name: &req.name,
                    display_version: req.display_version.as_deref(),
                    items: &req.items,
                    changelog: req.changelog.as_deref().unwrap_or_default(),
                },
            )?;
            match &req.work_item {
                Some(card) => {
                    blockers::set_work_item(app, &me, &release.id, card)?;
                    released(app.db.board_read(|t| load(t, project, &release.id))?)
                }
                None => released(release),
            }
        }
        "release_items" => {
            let req: c::ReleaseItems = decode("ReleaseItems", args, project)?;
            released(plan::change_items(
                app,
                &me,
                &req.release_id,
                &req.add,
                &req.remove,
                req.reason.as_deref(),
            )?)
        }
        "release_assemble" => {
            let req: c::ReleaseAssemble = decode("ReleaseAssemble", args, project)?;
            released(plan::assemble(app, &me, &req.release_id)?)
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
                source_commit: req
                    .source_commit
                    .map(|c| c.trim().to_string())
                    .filter(|c| !c.is_empty()),
            };
            released(assemble::attach_build(app, &me, &req.release_id, &build)?)
        }
        "release_publish" => {
            let req: c::ReleasePublish = decode("ReleasePublish", args, project)?;
            let (release, published) = publish::publish(
                app,
                &me,
                &Publish {
                    release_id: &req.release_id,
                    file: &req.file,
                    platform: req.platform.as_deref(),
                    version: req.version.as_deref(),
                    bundle_id: req.bundle_id.as_deref(),
                    source_commit: req.source_commit.as_deref(),
                },
            )?;
            Ok(json!({ "release": payload(&release), "published": published }))
        }
        "release_update" => {
            let req: c::ReleaseUpdate = decode("ReleaseUpdate", args, project)?;
            // The work card may be set at any open stage; the text only
            // while the package is assembling.
            if let Some(card) = &req.work_item {
                blockers::set_work_item(app, &me, &req.release_id, card)?;
            }
            let text = req.display_version.is_some()
                || req.changelog.is_some()
                || req.how_to_test.is_some();
            if req.work_item.is_some() && !text {
                return released(app.db.board_read(|t| load(t, project, &req.release_id))?);
            }
            let steps = req.how_to_test.map(|l| how_to_test(&l.values));
            released(package::update(
                app,
                &me,
                &req.release_id,
                req.display_version.as_deref(),
                req.changelog.as_deref(),
                steps.as_ref(),
            )?)
        }
        "release_deployed_via" => {
            let req: c::ReleaseDeployedVia = decode("ReleaseDeployedVia", args, project)?;
            released(deployed_via::deployed_via(
                app,
                &me,
                &req.release_id,
                &req.via_release_id,
            )?)
        }
        "release_cancel" => {
            let req: c::ReleaseCancel = decode("ReleaseCancel", args, project)?;
            let done = cancel::cancel(app, &me, &req.release_id, req.reason.as_deref())?;
            Ok(json!({
                "cancelled": done.event.to_json(),
                "predecessor": done.predecessor.map(|r| r.to_json()),
            }))
        }
        "release_test" => {
            let req: c::ReleaseTest = decode("ReleaseTest", args, project)?;
            let test = NewReleaseTest {
                machine: req.machine.trim(),
                tester: &me.bot.id,
                build_sha256: req.build_sha256.trim(),
                result: req.result.trim(),
                checks_passed: req.checks_passed.unwrap_or(0),
                checks_total: req.checks_total.unwrap_or(0),
                log_artifact: req.log_artifact.as_deref(),
            };
            released(package::record_test(app, &me, &req.release_id, &test)?)
        }
        "release_machines" => {
            let _: c::ReleaseMachines = decode("ReleaseMachines", args, project)?;
            app.db.board_read(|t| machines::view(t, project))
        }
        "release_machines_set" => {
            let req: c::ReleaseMachinesSet = decode("ReleaseMachinesSet", args, project)?;
            me.require(Role::Lead, "set the computers a release is tested on")?;
            // A lead's list narrows testing only; deploys still reach every
            // tester's computer (ARCH-R55 M1).
            app.db
                .board_tx(|t| machines::set(t, project, &req.machines, machines::SetBy::Lead))
        }
        "machine_name_set" => {
            let req: c::MachineNameSet = decode("MachineNameSet", args, project)?;
            me.require(Role::Lead, "name this computer")?;
            app.db.board_tx(|t| {
                machines::set_name(t, &req.name)?;
                machines::view(t, project)
            })
        }
        "release_pause" => {
            let req: c::ReleasePause = decode("ReleasePause", args, project)?;
            let release = app.db.board_read(|t| load(t, project, &req.release_id))?;
            released(lifecycle::pause(
                app,
                &me.actor(),
                &me.roles,
                &release.id,
                &req.reason,
            )?)
        }
        "release_resume" => {
            let req: c::ReleaseResume = decode("ReleaseResume", args, project)?;
            let release = app.db.board_read(|t| load(t, project, &req.release_id))?;
            released(lifecycle::resume(app, &me.actor(), &me.roles, &release.id)?)
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
            let mut answer = deploy::install(app, &me, &req.release_id)?;
            with_command(&mut answer);
            Ok(answer)
        }
        "install_quiesce" => {
            let req: c::InstallQuiesce = decode("InstallQuiesce", args, project)?;
            let ask = crate::quiesce::tool::Ask {
                action: req.action.trim(),
                release_id: &req.release_id,
                version: req.version.as_deref(),
                binary_sha256: req.binary_sha256.as_deref(),
            };
            crate::quiesce::tool::call(app, &me, &ask)
        }
        "deploy_confirm" => {
            let req: c::DeployConfirm = decode("DeployConfirm", args, project)?;
            let result = parse_arg("result", &req.result, DeployResult::CONFIRMABLE)?;
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

/// `install_release` also says how to run the install on this computer:
/// this daemon's binary by its exact path, which the install extra allows
/// without a prompt (H-166).
/// The release id is the gate's own, never the caller's text.
pub(super) fn with_command(answer: &mut Value) {
    let Some(id) = answer["release_id"].as_str().filter(|id| {
        id.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    }) else {
        return;
    };
    let binary = crate::bot_permissions::this_binary();
    let args = format!("release install {id}");
    answer["command"] = json!(crate::bot_permissions::hermesd_command(&binary, &args));
}

/// The contract's how-to-test steps as stored: `[{item_id?, platform, steps}]`.
fn how_to_test(steps: &[c::HowToTest]) -> Value {
    json!(steps
        .iter()
        .map(|s| json!({"item_id": s.item_id, "platform": s.platform, "steps": s.steps}))
        .collect::<Vec<_>>())
}
