//! The Owner review setting (§4.3, ruling 7629a873), as a table.

use super::*;
use crate::prs::model::{Pr, PrState};

fn pr(flagged: bool) -> Pr {
    let now = chrono::Utc::now();
    Pr {
        id: "p".into(),
        project_id: "x".into(),
        number: 1,
        repo: "o/r".into(),
        item_id: "H-1".into(),
        branch: "b".into(),
        base: "main".into(),
        base_sha: "a".into(),
        head_sha: "h".into(),
        head_patch_id: "p".into(),
        remote_sha: "h".into(),
        moved_unreported: false,
        state: PrState::Open,
        author: "dev".into(),
        title: "t".into(),
        change_note: String::new(),
        owner_flagged: flagged,
        owner_flag_reason: None,
        merged_sha: None,
        merged_at: None,
        close_reason: None,
        opened_at: now,
        closed_at: None,
        updated_at: now,
        version: 1,
    }
}

fn shape(areas: &[&str], security: bool) -> Shape {
    Shape {
        areas: areas.iter().map(|a| (*a).to_string()).collect(),
        security,
    }
}

fn set(mode: Mode, areas: &[&str]) -> Settings {
    Settings {
        mode,
        areas: areas.iter().map(|a| (*a).to_string()).collect(),
    }
}

#[test]
fn no_row_means_every_pr_waits_for_the_owner() {
    assert_eq!(Settings::default().mode, Mode::All);
    assert!(required(
        &Settings::default(),
        &shape(&["docs-user"], false),
        &pr(false)
    ));
}

#[test]
fn areas_flagged_and_none_behave_and_security_always_needs_the_owner() {
    let ui = shape(&["ui"], false);
    let docs = shape(&["docs-user"], false);
    let security = shape(&["security"], true);
    let areas = set(Mode::Areas, &["ui"]);
    assert!(required(&areas, &ui, &pr(false)));
    assert!(
        !required(&areas, &docs, &pr(false)),
        "docs-only follows the setting"
    );
    assert!(required(&areas, &security, &pr(false)));
    let flagged = set(Mode::Flagged, &[]);
    assert!(!required(&flagged, &ui, &pr(false)));
    assert!(required(&flagged, &ui, &pr(true)));
    assert!(required(&flagged, &security, &pr(false)));
    let none = set(Mode::None, &[]);
    assert!(!required(&none, &security, &pr(true)), "none is none");
}
