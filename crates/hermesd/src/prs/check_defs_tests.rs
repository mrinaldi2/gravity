//! H-311: which definition each check runs for a PR that changes
//! `checks.toml`, and how the PR's row names it (CE M1, S3).

use super::with_head_defs;
use crate::board::policy::{parse_checks, Checks};
use crate::prs::check_model::{CheckResult, NewCheck};

const MAIN: &str = r#"
[[check]]
name = "same"
run = "true"

[[check]]
name = "fixed"
run = "exit 1"

[[check]]
name = "optional"
run = "cargo test"

[[check]]
name = "narrowed"
run = "cargo test"
paths = ["src/**"]

[[check]]
name = "gone"
run = "exit 1"
"#;

const HEAD: &str = r#"
[[check]]
name = "same"
run = "true"

[[check]]
name = "fixed"
run = "true"
needs = ["cargo"]

[[check]]
name = "optional"
run = "cargo test"
required = false

[[check]]
name = "narrowed"
run = "cargo test"
paths = ["docs/**"]

[[check]]
name = "new"
run = "true"
"#;

fn checks(text: &str) -> Checks {
    parse_checks(text).unwrap()
}

fn ran(out: &[NewCheck]) -> Vec<(String, String)> {
    out.iter()
        .filter(|c| c.name != ".hermes/checks.toml")
        .map(|c| (c.name.clone(), c.run.clone()))
        .collect()
}

fn row(out: &[NewCheck]) -> Option<String> {
    let row = out.iter().find(|c| c.name == ".hermes/checks.toml")?;
    assert_eq!(row.result, CheckResult::Pass);
    row.note.clone()
}

fn changed() -> Vec<String> {
    vec!["src/x.rs".to_string(), ".hermes/checks.toml".to_string()]
}

/// CE M1: each kind is named plainly, the quiet weakenings included.
#[test]
fn each_kind_of_change_is_named_on_the_pr() {
    let out = with_head_defs(&checks(MAIN), &checks(MAIN), &checks(HEAD), &changed());
    assert_eq!(
        ran(&out),
        vec![
            ("same".to_string(), "true".to_string()),
            ("fixed".to_string(), "true".to_string()),
            ("new".to_string(), "true".to_string()),
        ]
    );
    assert_eq!(
        row(&out).as_deref(),
        Some(
            "this PR's checks.toml: fixed runs its own definition; optional skipped: no longer \
             required; narrowed skipped: paths no longer match; gone removed; new added"
        )
    );
}

#[test]
fn a_check_made_optional_is_skipped_and_named() {
    let head = MAIN.replace(
        "name = \"optional\"\nrun = \"cargo test\"",
        "name = \"optional\"\nrun = \"cargo test\"\nrequired = false",
    );
    let out = with_head_defs(&checks(MAIN), &checks(MAIN), &checks(&head), &changed());
    assert!(ran(&out).iter().all(|(n, _)| n != "optional"));
    assert_eq!(
        row(&out).as_deref(),
        Some("this PR's checks.toml: optional skipped: no longer required")
    );
}

#[test]
fn a_check_whose_paths_no_longer_match_is_skipped_and_named() {
    let head = MAIN.replace("paths = [\"src/**\"]", "paths = [\"docs/**\"]");
    let out = with_head_defs(&checks(MAIN), &checks(MAIN), &checks(&head), &changed());
    assert!(ran(&out).iter().all(|(n, _)| n != "narrowed"));
    assert_eq!(
        row(&out).as_deref(),
        Some("this PR's checks.toml: narrowed skipped: paths no longer match")
    );
}

#[test]
fn a_removed_check_is_named() {
    let head = MAIN.replace("[[check]]\nname = \"gone\"\nrun = \"exit 1\"\n", "");
    let out = with_head_defs(&checks(MAIN), &checks(MAIN), &checks(&head), &changed());
    assert!(ran(&out).iter().all(|(n, _)| n != "gone"));
    assert_eq!(
        row(&out).as_deref(),
        Some("this PR's checks.toml: gone removed")
    );
}

/// CE S3: on a branch behind main, main's own later edit isn't the PR's: the
/// check runs main's current definition, and nothing is named for it.
#[test]
fn mains_later_edits_are_not_the_prs() {
    let main_now = MAIN.replace(
        "name = \"fixed\"\nrun = \"exit 1\"",
        "name = \"fixed\"\nrun = \"cargo test --strict\"",
    );
    // The PR only adds a check; its `fixed` is still the fork's.
    let head = format!("{MAIN}\n[[check]]\nname = \"new\"\nrun = \"true\"\n");
    let out = with_head_defs(
        &checks(&main_now),
        &checks(MAIN),
        &checks(&head),
        &changed(),
    );
    let fixed = ran(&out).into_iter().find(|(n, _)| n == "fixed").unwrap();
    assert_eq!(fixed.1, "cargo test --strict", "main's current definition");
    assert_eq!(
        row(&out).as_deref(),
        Some("this PR's checks.toml: new added")
    );
}

#[test]
fn a_checks_toml_that_changes_no_check_adds_no_row() {
    let out = with_head_defs(&checks(MAIN), &checks(MAIN), &checks(MAIN), &changed());
    assert_eq!(row(&out), None);
    assert_eq!(ran(&out).len(), 5, "every check of main runs");
}
