use super::*;

const FIXTURE: &str = include_str!("../../../tests/fixtures/board_import_backlog.md");

fn entry<'a>(parsed: &'a Parsed, id: &str) -> &'a Entry {
    parsed
        .entries
        .iter()
        .find(|e| e.source_id == id)
        .unwrap_or_else(|| panic!("no entry {id}"))
}

#[test]
fn rows_and_sections_map_to_entries_and_the_rest_is_reported() {
    let parsed = parse(FIXTURE);
    let ids: Vec<&str> = parsed
        .entries
        .iter()
        .map(|e| e.source_id.as_str())
        .collect();
    assert_eq!(
        ids,
        [
            "H-001", "H-002", "H-003", "H-004", "H-005", "H-006", "H-007", "H-009", "R1-I2", "B1",
            "REL-D-1", "U2", "H-011", "H-040"
        ]
    );
    let skipped: Vec<(&str, usize)> = parsed
        .skipped
        .iter()
        .map(|s| (s.id.as_str(), s.line))
        .collect();
    assert_eq!(
        skipped,
        [
            ("R1-D1..D3, R1-I1", 17),
            ("B3", 19),
            ("G2+G3", 20),
            ("I0′", 22)
        ]
    );
    assert!(parsed.skipped[1].reason.contains("Approved w/ M1"));
    assert!(parsed.skipped[2].reason.contains("isn't one plain item id"));
}

#[test]
fn states_types_and_fields_map_without_guessing() {
    use ColumnCategory::*;
    let parsed = parse(FIXTURE);
    let cat = |id| entry(&parsed, id).category;
    assert_eq!(
        [
            cat("H-001"),
            cat("H-002"),
            cat("H-003"),
            cat("H-004"),
            cat("H-005")
        ],
        [Done, Review, Verify, Cancelled, Inbox]
    );
    assert_eq!([cat("H-006"), cat("B1"), cat("U2")], [Doing, Doing, Ready]);
    assert_eq!(
        [cat("REL-D-1"), cat("H-011"), cat("H-040")],
        [Deploying, Approval, Inbox]
    );

    assert_eq!(entry(&parsed, "H-006").item_type, ItemType::Bug);
    assert_eq!(entry(&parsed, "H-011").item_type, ItemType::Spike);
    assert_eq!(entry(&parsed, "H-002").item_type, ItemType::Feature);

    let h1 = entry(&parsed, "H-001");
    assert_eq!(h1.key, Key::Kept(1));
    assert_eq!(h1.size, Some(Size::M));
    assert!(h1.platforms.is_empty());
    assert_eq!(h1.warnings.len(), 1, "'all' names no platform");
    assert_eq!(h1.decisions, ["aaaa1111", "bbbb2222"]);
    assert_eq!(h1.owner.as_deref(), Some("Architect"));

    let h3 = entry(&parsed, "H-003");
    assert_eq!(h3.platforms, [Platform::Ios]);
    assert_eq!(h3.owner.as_deref(), Some("iOS QA"));
    assert_eq!(h3.tasks, ["1234abcd"]);

    let h5 = entry(&parsed, "H-005");
    assert_eq!(h5.platforms, [Platform::Desktop, Platform::Ios]);
    assert_eq!(h5.size, None);
    assert!(h5.warnings[0].contains("S-M"));

    let h7 = entry(&parsed, "H-007");
    assert_eq!(h7.artifacts, ["H-007-design.md", "mockups.html"]);
    assert!(h7.description.contains("## Why\n\nSee H-007-design.md"));
    assert!(h7
        .description
        .contains("- State: Done (H-007-design.md + mockups.html)"));

    let r = entry(&parsed, "R1-I2");
    assert_eq!(r.key, Key::Legacy("R1-I2".into()));
    assert_eq!(r.labels, ["rename", "was:R1-I2"]);
    assert_eq!(entry(&parsed, "B1").labels, ["was:B1"]);
    assert_eq!(entry(&parsed, "REL-D-1").size, None, "a dash is no size");

    let h40 = entry(&parsed, "H-040");
    assert_eq!(h40.key, Key::Kept(40));
    assert_eq!(h40.title, "Windows: name the holders of the home");
    assert!(h40.description.starts_with("- Source: the install failed"));
    assert!(h40.warnings[0].contains("no state"));
}

#[test]
fn a_table_needs_its_header_and_a_heading_needs_an_id() {
    let parsed = parse("| A | B |\n|---|---|\n| H-1 | x |\n\n### Notes\n- nothing\n");
    assert!(parsed.entries.is_empty());
    assert_eq!(
        parsed.skipped.len(),
        1,
        "a heading without a title is reported"
    );
}
