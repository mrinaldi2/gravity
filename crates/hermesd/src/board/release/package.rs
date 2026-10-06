//! What a package carries for testing (H-020 §6.4): the changelog and the
//! how-to-test steps DevOps fills in, and each required machine's result
//! against the exact build. Submit refuses until every required machine has
//! passed the builds being frozen.

use std::sync::Arc;

use serde_json::Value;

use crate::app::AppState;
use crate::board::model::{Platform, Role};
use crate::db::{BoardTx, NewReleaseTest};
use crate::decisions::{conflict, invalid};

use super::machines;
use super::model::{Release, ReleaseStatus};
use super::{load, Caller};

/// Results a machine reports for a package.
pub const TEST_RESULTS: &[&str] = &["pass", "fail", "blocked"];

/// The computers a package must pass on (H-115): frozen at submit, else the
/// list the owner or lead set, or every tester's computer (see [`machines`]).
pub fn required_machines(t: &BoardTx<'_>, release: &Release) -> anyhow::Result<Vec<String>> {
    machines::tested_on(t, release)
}

fn item_platforms(t: &BoardTx<'_>, release: &Release) -> anyhow::Result<Vec<Platform>> {
    let mut platforms = Vec::new();
    for ri in &release.items {
        platforms.extend(
            t.item(&ri.item_id)?
                .map(|i| i.platforms)
                .unwrap_or_default(),
        );
    }
    platforms.sort_unstable_by_key(|p| p.as_str());
    platforms.dedup();
    Ok(platforms)
}

/// The platforms `machine` tests this package for (ARCH-R25 F2): those whose
/// `required_machines` list it, or with none listing it, the items'
/// platforms when the machine is required as a tester machine.
fn machine_platforms(
    t: &BoardTx<'_>,
    release: &Release,
    machine: &str,
) -> anyhow::Result<Vec<Platform>> {
    let configured: Vec<Platform> = t
        .settings(&release.project_id)?
        .iter()
        .flat_map(|s| s.required_machines.iter())
        .filter(|(_, machines)| machines.iter().any(|m| m == machine))
        .map(|(p, _)| *p)
        .collect();
    if !configured.is_empty() {
        return Ok(configured);
    }
    if required_machines(t, release)?.iter().any(|m| m == machine) {
        return item_platforms(t, release);
    }
    Ok(Vec::new())
}

/// A build is for a platform when it names it, alone or with a variant
/// (`desktop-mac` is a desktop build).
fn build_is_for(build_platform: &str, platform: Platform) -> bool {
    let p = platform.as_str();
    build_platform == p
        || build_platform
            .strip_prefix(p)
            .is_some_and(|rest| rest.starts_with('-'))
}

/// Every required machine has a pass against one of the package's current
/// builds; the refusal names the ones that haven't. No required machine at
/// all is a refusal, never a vacuous pass (ARCH-R25 F1).
pub fn check_tested(t: &BoardTx<'_>, release: &Release) -> anyhow::Result<()> {
    let builds: Vec<&str> = release.builds.iter().map(|b| b.sha256.as_str()).collect();
    let required = required_machines(t, release)?;
    if required.is_empty() {
        return Err(conflict(format!(
            "release {} has no computer to test it on: give a bot the tester role, or set the \
             computers with release_machines_set",
            release.name
        )));
    }
    let missing: Vec<String> = required
        .into_iter()
        .filter(|m| {
            !release.tests.iter().any(|r| {
                r.machine == *m && r.result == "pass" && builds.contains(&r.build_sha256.as_str())
            })
        })
        .collect();
    if missing.is_empty() {
        return Ok(());
    }
    Err(conflict(format!(
        "release {} isn't tested on {}: each needs a pass on the current builds (release_test)",
        release.name,
        missing.join(", ")
    )))
}

/// Fill in what the owner reads while the package is assembling.
pub fn update(
    app: &Arc<AppState>,
    me: &Caller<'_>,
    release_id: &str,
    display_version: Option<&str>,
    changelog: Option<&str>,
    how_to_test: Option<&Value>,
) -> anyhow::Result<Release> {
    me.require(Role::Devops, "edit a release")?;
    if let Some(steps) = how_to_test {
        check_how_to_test(steps)?;
    }
    app.db.board_tx(|t| {
        let release = load(t, &me.bot.project_id, release_id)?;
        if !release.status.is_unsubmitted() {
            return Err(conflict(format!(
                "release {} is {}; it is frozen once submitted",
                release.name,
                release.status.as_str()
            )));
        }
        t.update_release_text(&release.id, display_version, changelog, how_to_test)?;
        Ok(t.release(&release.id)?.expect("loaded"))
    })
}

/// `[{item_id?, platform, steps: [..]}]`, each with at least one step.
fn check_how_to_test(steps: &Value) -> anyhow::Result<()> {
    let ok = steps.as_array().is_some_and(|all| {
        all.iter().all(|s| {
            s["platform"].as_str().is_some_and(|p| !p.trim().is_empty())
                && s["steps"].as_array().is_some_and(|l| {
                    !l.is_empty()
                        && l.iter()
                            .all(|x| x.as_str().is_some_and(|x| !x.trim().is_empty()))
                })
        })
    });
    if ok {
        Ok(())
    } else {
        Err(invalid(
            "'how_to_test' is a list of {item_id?, platform, steps: [text, ...]}, each with steps",
        ))
    }
}

/// A tester's result for the package on their machine, against one of its
/// builds by sha256.
pub fn record_test(
    app: &Arc<AppState>,
    me: &Caller<'_>,
    release_id: &str,
    result: &NewReleaseTest<'_>,
) -> anyhow::Result<Release> {
    if !TEST_RESULTS.contains(&result.result) {
        return Err(invalid("'result' is pass, fail or blocked"));
    }
    app.db.board_tx(|t| {
        let release = load(t, &me.bot.project_id, release_id)?;
        // Each computer's own tester reports for it (H-115); `machine` may
        // be left out by a tester on one computer.
        let machine = machines::reporting_for(t, &release.project_id, &me.bot.id, result.machine)?;
        let result = &NewReleaseTest {
            machine: &machine,
            ..*result
        };
        if release.status != ReleaseStatus::Built {
            return Err(conflict(format!(
                "release {} is {}; test it once it has its builds and before it is submitted",
                release.name,
                release.status.as_str()
            )));
        }
        let Some(build) = release
            .builds
            .iter()
            .find(|b| b.sha256 == result.build_sha256)
        else {
            return Err(invalid(
                "'build_sha256' must be one of the release's builds",
            ));
        };
        let platforms = machine_platforms(t, &release, result.machine)?;
        if !platforms.iter().any(|p| build_is_for(&build.platform, *p)) {
            let names: Vec<&str> = platforms.iter().map(|p| p.as_str()).collect();
            // No build of the package is for this computer at all (S3).
            let any = release
                .builds
                .iter()
                .any(|b| platforms.iter().any(|p| build_is_for(&b.platform, *p)));
            if !any && !names.is_empty() {
                return Err(invalid(format!(
                    "release {} has no build for {} on {}; DevOps attaches one first",
                    release.name,
                    names.join(", "),
                    result.machine
                )));
            }
            return Err(invalid(format!(
                "that sha256 is the {} build; {} tests {} for this release",
                build.platform,
                result.machine,
                if names.is_empty() {
                    "no platform".to_string()
                } else {
                    names.join(", ")
                }
            )));
        }
        t.record_release_test(&release.id, result)?;
        Ok(t.release(&release.id)?.expect("loaded"))
    })
}

/// What the owner reads in the decision; the release review shows the same.
/// A successor says what it dropped from the package it replaces.
pub fn decision_body(release: &Release, before: Option<&Release>) -> String {
    let ids: Vec<&str> = release.items.iter().map(|i| i.item_id.as_str()).collect();
    let mut body = format!(
        "{} item(s) for your verdict: ship, hold or rework each one in the release review \
         (dashboard or phone). This decision is answered there, not here.\n\nItems: {}\n",
        ids.len(),
        ids.join(", ")
    );
    if let Some(before) = before {
        let dropped: Vec<&str> = before
            .items
            .iter()
            .map(|i| i.item_id.as_str())
            .filter(|id| !ids.contains(id))
            .collect();
        body.push_str(&format!(
            "Replaces release {}{}; it is a new build, so it needs a new ruling.\n",
            before.name,
            if dropped.is_empty() {
                String::new()
            } else {
                format!(", without {}", dropped.join(", "))
            }
        ));
    }
    for b in &release.builds {
        body.push_str(&format!(
            "Build {}: {} (sha256 {})\n",
            b.platform, b.version, b.sha256
        ));
    }
    for step in release.how_to_test.as_array().into_iter().flatten() {
        let steps: Vec<&str> = step["steps"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect();
        body.push_str(&format!(
            "\nTry it on {}{}:\n- {}\n",
            step["platform"].as_str().unwrap_or("any"),
            step["item_id"]
                .as_str()
                .map(|i| format!(" ({i})"))
                .unwrap_or_default(),
            steps.join("\n- ")
        ));
    }
    if !release.changelog.trim().is_empty() {
        body.push_str(&format!("\n{}\n", release.changelog.trim()));
    }
    body
}
