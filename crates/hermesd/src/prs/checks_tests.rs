//! H-311: which definition each check runs for a PR that changes
//! `checks.toml`.

use super::with_head_defs;
use crate::board::policy::parse_checks;
use crate::prs::check_model::CheckResult;

const BASE: &str = r#"
[[check]]
name = "same"
run = "true"

[[check]]
name = "fixed"
run = "exit 1"

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
name = "new"
run = "true"
"#;

#[test]
fn unchanged_keep_base_changed_and_added_run_heads_removed_are_named() {
    let base = parse_checks(BASE).unwrap();
    let head = parse_checks(HEAD).unwrap();
    let checks = with_head_defs(&base, &head, &["src/x".to_string()]);
    let run = |name: &str| {
        checks
            .iter()
            .find(|c| c.name == name)
            .map(|c| (c.run.clone(), c.needs.clone()))
    };
    assert_eq!(run("same"), Some(("true".into(), vec![])));
    assert_eq!(
        run("fixed"),
        Some(("true".into(), vec!["cargo".to_string()]))
    );
    assert_eq!(run("new"), Some(("true".into(), vec![])));
    assert_eq!(run("gone"), None, "removed in the head: not run");
    let said = checks
        .iter()
        .find(|c| c.name == ".hermes/checks.toml")
        .unwrap();
    assert_eq!(said.result, CheckResult::Pass);
    assert_eq!(
        said.note.as_deref(),
        Some("this PR's checks.toml runs its own definition of fixed; adds new; removes gone")
    );
}

#[test]
fn a_checks_toml_that_changes_no_check_adds_no_row() {
    let base = parse_checks(BASE).unwrap();
    let checks = with_head_defs(&base, &base, &["src/x".to_string()]);
    assert!(checks.iter().all(|c| c.name != ".hermes/checks.toml"));
    assert_eq!(checks.len(), 3);
}
