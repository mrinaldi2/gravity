//! Policy files: parsing, required roles, waivers (H-267).

use std::collections::BTreeSet;

use super::ReviewRole::{Architect, Ce, Devops, Owner, Ux};
use super::*;

pub(super) const REVIEWERS: &str = include_str!("fixtures/reviewers.toml");
pub(super) const CHECKS: &str = include_str!("fixtures/checks.toml");

fn paths(list: &[&str]) -> Vec<String> {
    list.iter().map(|p| p.to_string()).collect()
}

fn roles(list: &[ReviewRole]) -> BTreeSet<ReviewRole> {
    list.iter().copied().collect()
}

fn fixture() -> Reviewers {
    parse_reviewers(REVIEWERS).unwrap()
}

#[test]
fn the_fixtures_parse() {
    assert_eq!(fixture().areas.len(), 7);
    let checks = parse_checks(CHECKS).unwrap();
    assert_eq!(checks.checks.len(), 5);
    let windows = checks.checks.iter().find(|c| c.name == "windows").unwrap();
    assert_eq!(windows.machine.as_deref(), Some("windows"));
    assert!(windows.required);
}

#[test]
fn required_roles_are_the_union_of_the_matched_areas() {
    let changed = paths(&[
        "crates/hermesd/src/peer/link.rs",
        "apps/desktop/src/App.tsx",
        "scripts/release-notes.sh",
    ]);
    assert_eq!(
        required_roles(&fixture(), &changed),
        roles(&[Architect, Ux, Ce, Devops])
    );
}

#[test]
fn a_change_no_area_matches_needs_the_architect() {
    let changed = paths(&["README.md"]);
    assert_eq!(required_roles(&fixture(), &changed), roles(&[Architect]));
    assert_eq!(
        required_roles(&Reviewers::default(), &changed),
        roles(&[Architect])
    );
}

#[test]
fn a_matched_area_replaces_the_default() {
    let changed = paths(&["docs/user/getting-started.md"]);
    // docs-user and docs-dev both match.
    assert_eq!(
        required_roles(&fixture(), &changed),
        roles(&[Architect, Ux])
    );
    let ui_only = parse_reviewers("[[area]]\nname='ui'\npaths=['ui/**']\nroles=['ux']").unwrap();
    assert_eq!(required_roles(&ui_only, &paths(&["ui/a.ts"])), roles(&[Ux]));
}

/// Whatever the files say, even with no area at all for `.hermes/`.
#[test]
fn a_policy_change_always_needs_architect_ce_and_the_owner() {
    let ui_only = parse_reviewers("[[area]]\nname='all'\npaths=['**']\nroles=['ux']").unwrap();
    for reviewers in [fixture(), Reviewers::default(), ui_only] {
        for file in [REVIEWERS_PATH, CHECKS_PATH] {
            let got = required_roles(&reviewers, &paths(&[file]));
            assert!(
                got.is_superset(&roles(&[Architect, Ce, Owner])),
                "{file}: {got:?}"
            );
        }
    }
    assert!(!touches_policy(&paths(&[".hermes/README.md"])));
}

#[test]
fn malformed_files_are_refused_with_the_file_named() {
    for bad in [
        "[[area]]\nname='x'\npaths=['a/**']\nroles=['wizard']",
        "[[area]]\nname='x'\npaths=['a/**']\nrole=['ux']",
        "[[area]]\nname='x'\npaths=[]\nroles=['ux']",
        "[[area]]\nname='x'\npaths=['a']\nroles=['ux']\n[[area]]\nname='x'\npaths=['b']\nroles=['ux']",
        "not toml",
    ] {
        let err = parse_reviewers(bad).unwrap_err().to_string();
        assert!(err.contains(REVIEWERS_PATH), "{bad}: {err}");
    }
    for bad in [
        "[[check]]\nname='x'\nrun=''",
        "[[check]]\nname='x'\nrun='true'\nneeds='cargo'",
        "[[check]]\nname='x'\nrun='a'\n[[check]]\nname='x'\nrun='b'",
    ] {
        let err = parse_checks(bad).unwrap_err().to_string();
        assert!(err.contains(CHECKS_PATH), "{bad}: {err}");
    }
}

#[test]
fn checks_run_for_the_paths_they_name() {
    let checks = parse_checks(CHECKS).unwrap();
    let changed = paths(&["apps/desktop/src/App.tsx"]);
    let names: Vec<&str> = checks
        .for_change(&changed)
        .map(|c| c.name.as_str())
        .collect();
    assert_eq!(names, ["desktop", "vr"]);
    let everywhere = parse_checks("[[check]]\nname='lint'\nrun='true'").unwrap();
    assert!(everywhere.checks[0].required);
    assert_eq!(everywhere.for_change(&paths(&["x"])).count(), 1);
}

#[test]
fn the_lead_waives_a_role_with_a_reason() {
    let required = roles(&[Architect, Ux, Ce]);
    let changed = paths(&["apps/desktop/src/App.tsx"]);
    let waiver = waive(
        &required,
        &changed,
        Ux,
        "  copy-only change  ",
        "lead-bot",
        true,
    )
    .unwrap();
    assert_eq!(waiver.reason, "copy-only change");
    assert_eq!(waiver.by, "lead-bot");
    assert_eq!(
        after_waivers(&required, &changed, &[waiver]),
        roles(&[Architect, Ce])
    );
}

#[test]
fn a_waiver_is_refused_for_ce_the_owner_no_reason_or_no_lead() {
    let required = roles(&[Architect, Ce, Owner]);
    let changed = paths(&["crates/x.rs"]);
    for role in [Ce, Owner] {
        let err = waive(&required, &changed, role, "trust me", "lead", true).unwrap_err();
        assert!(err.to_string().contains("never be waived"), "{err}");
    }
    assert!(waive(&required, &changed, Architect, " ", "lead", true).is_err());
    assert!(waive(&required, &changed, Architect, "why", "dev", false).is_err());
    assert!(waive(&required, &changed, Ux, "not needed", "lead", true).is_err());
}

/// H-313: on the policy files the lead can't waive architect, ce or the
/// owner, and the refusal says why; an ordinary PR's architect still can be.
#[test]
fn a_waiver_on_a_policy_pr_is_refused_with_the_reason() {
    for file in [REVIEWERS_PATH, CHECKS_PATH] {
        let changed = paths(&["crates/x.rs", file]);
        let required = required_roles(&fixture(), &changed);
        for role in [Architect, Ce, Owner] {
            let err = waive(&required, &changed, role, "trivial", "lead", true).unwrap_err();
            let text = err.to_string();
            assert!(text.contains("policy files"), "{file} {role:?}: {text}");
            assert!(text.contains(role.as_str()), "{file} {role:?}: {text}");
        }
    }
    let ordinary = paths(&["crates/x.rs"]);
    let required = roles(&[Architect]);
    let waiver = waive(&required, &ordinary, Architect, "trivial", "lead", true).unwrap();
    assert!(after_waivers(&required, &ordinary, &[waiver]).is_empty());
}

/// H-313: an architect waiver stored for a PR that later touches the policy
/// files takes nothing off.
#[test]
fn a_stored_architect_waiver_takes_nothing_off_a_policy_pr() {
    let changed = paths(&[REVIEWERS_PATH]);
    let required = required_roles(&fixture(), &changed);
    let stored = [Waiver {
        role: Architect,
        reason: "given before the policy change".into(),
        by: "lead".into(),
    }];
    assert_eq!(after_waivers(&required, &changed, &stored), required);
    assert!(after_waivers(&required, &changed, &stored).contains(&Architect));
}

#[test]
fn a_stored_waiver_for_ce_or_the_owner_takes_nothing_off() {
    let required = roles(&[Architect, Ce, Owner]);
    let forged: Vec<Waiver> = [Ce, Owner]
        .into_iter()
        .map(|role| Waiver {
            role,
            reason: "forged".into(),
            by: "lead".into(),
        })
        .collect();
    assert_eq!(
        after_waivers(&required, &paths(&["crates/x.rs"]), &forged),
        required
    );
}

#[test]
fn roles_map_to_board_roles() {
    assert_eq!(Architect.board_role(), Some("reviewer.arch"));
    assert_eq!(Ce.board_role(), Some("reviewer.ce"));
    assert_eq!(ReviewRole::Qa.board_role(), Some("tester"));
    assert_eq!(Owner.board_role(), None);
    assert_eq!(ReviewRole::parse("devops"), Some(Devops));
    assert_eq!(ReviewRole::parse("wizard"), None);
}
