use super::*;
use bus::contract::board as c;

#[test]
fn every_tool_has_its_message_and_every_enum_resolves() {
    for tool in all_tools() {
        assert!(schemas().contains_key(tool.message), "{}", tool.message);
    }
    // Panics on an enum with no spellings.
    let all = board_tool_list(&[Role::Lead, Role::Tester, Role::Devops]);
    assert_eq!(
        all.len(),
        BOARD_TOOLS.len()
            + super::super::releases::RELEASE_TOOLS.len()
            + super::super::meetings::MEETING_TOOLS.len()
            + super::super::prs::PR_TOOLS.len()
    );
    assert!(!json!(all).to_string().contains("project_id"));
}

#[test]
fn the_list_follows_roles() {
    let names = |roles: &[Role]| -> Vec<String> {
        board_tool_list(roles)
            .iter()
            .map(|t| t["name"].as_str().unwrap().to_string())
            .collect()
    };
    let dev = names(&[Role::Dev]);
    assert!(dev.contains(&"item_move".to_string()));
    assert!(!dev.contains(&"item_assign".to_string()));
    assert!(!dev.contains(&"item_check_ac".to_string()));
    assert!(names(&[Role::Lead]).contains(&"item_rank".to_string()));
    assert!(names(&[Role::Tester]).contains(&"item_check_ac".to_string()));
}

#[test]
fn short_names_decode_and_come_back_short() {
    let args = json!({"type": "bug", "title": "Crash", "platforms": ["ios", "DESKTOP"],
                      "priority": "p1", "size": "S"});
    let req: c::ItemCreate = decode("ItemCreate", &args, "p1").unwrap();
    assert_eq!(req.project_id, "p1");
    assert_eq!(req.r#type, c::ItemType::Bug as i32);
    assert_eq!(
        req.platforms,
        [c::Platform::Ios as i32, c::Platform::Desktop as i32]
    );
    assert_eq!(req.priority, Some(c::Priority::P1 as i32));

    let update: c::ItemUpdate = decode(
        "ItemUpdate",
        &json!({"id": "H-1", "expected_version": 3, "labels": []}),
        "p1",
    )
    .unwrap();
    assert_eq!(
        update.labels.map(|l| l.values.len()),
        Some(0),
        "an empty list is a value"
    );
    assert!(update.platforms.is_none(), "a missing list is no change");

    let err = decode::<c::ItemCreate>("ItemCreate", &json!({"type": "story", "title": "x"}), "p")
        .unwrap_err()
        .to_string();
    assert!(err.contains("epic, feature, bug, spike, chore"), "{err}");
    let err = decode::<c::ItemMove>("ItemMove", &json!({"id": "H-1"}), "p")
        .unwrap_err()
        .to_string();
    assert!(err.contains("'expected_version' is required") || err.contains("'to' is required"));
    assert!(decode::<c::ItemGet>("ItemGet", &json!({"id": "H-1", "x": 1}), "p").is_err());

    let out =
        friendly(json!({"category": "COLUMN_CATEGORY_DOING", "roles": ["ROLE_REVIEWER_ARCH"]}));
    assert_eq!(
        out,
        json!({"category": "doing", "roles": ["reviewer.arch"]})
    );
}
