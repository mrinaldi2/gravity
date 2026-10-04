//! The mapping against the golden fixtures and the proto names.

use std::path::PathBuf;

use serde::de::DeserializeOwned;

use super::super::model as m;
use super::*;
use bus::contract::board as c;

fn fixture<T: DeserializeOwned>(name: &str) -> T {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join(format!("../bus/fixtures/board/{name}.json"));
    serde_json::from_str(&std::fs::read_to_string(&path).expect("fixture")).expect("decodes")
}

/// contract → model → contract changes nothing, for every golden fixture.
fn through_the_model<C, M>(name: &str)
where
    C: DeserializeOwned + Clone + PartialEq + std::fmt::Debug + From<M>,
    M: TryFrom<C, Error = MapError>,
{
    let wire: C = fixture(name);
    let model = M::try_from(wire.clone()).unwrap_or_else(|e| panic!("{name}: {e}"));
    assert_eq!(C::from(model), wire, "{name}");
}

#[test]
fn every_fixture_survives_the_mapping() {
    through_the_model::<c::BoardSettings, m::BoardSettings>("settings");
    through_the_model::<c::BoardColumn, m::BoardColumn>("column");
    through_the_model::<c::ProjectRole, m::ProjectRole>("role");
    through_the_model::<c::Item, m::Item>("item");
    through_the_model::<c::ItemCard, m::ItemCard>("card");
    through_the_model::<c::ItemLink, m::ItemLink>("link");
    through_the_model::<c::ItemComment, m::ItemComment>("comment");
    through_the_model::<c::ItemEvent, m::ItemEvent>("event");
    through_the_model::<c::Template, m::Template>("template");
    through_the_model::<c::Unmet, m::Unmet>("unmet");
}

/// A newer client's enum value, or an unset one, is refused, not guessed.
#[test]
fn unknown_or_unset_enum_values_are_refused() {
    assert_eq!(
        m::ColumnCategory::from_wire(0),
        Err(MapError::UnknownEnum("ColumnCategory"))
    );
    assert_eq!(
        m::ColumnCategory::from_wire(99),
        Err(MapError::UnknownEnum("ColumnCategory"))
    );
    let mut column: c::BoardColumn = fixture("column");
    column.wip_scope = 0;
    assert!(m::BoardColumn::try_from(column).is_err());
}

/// Every model variant has the contract value whose proto name spells the
/// same thing, so stored text and wire names cannot drift apart.
#[test]
fn stored_spellings_match_the_proto_names() {
    fn same<M: Copy + Into<i32>>(
        prefix: &str,
        all: &[M],
        text: fn(M) -> &'static str,
        name: fn(i32) -> Option<&'static str>,
    ) {
        for value in all {
            let stored = text(*value).to_uppercase().replace(['.', ':'], "_");
            let proto = name((*value).into()).expect("mapped to a defined value");
            assert_eq!(proto, format!("{prefix}_{stored}"));
        }
    }
    same(
        "COLUMN_CATEGORY",
        m::ColumnCategory::ALL,
        m::ColumnCategory::as_str,
        |n| c::ColumnCategory::try_from(n).ok().map(|e| e.as_str_name()),
    );
    same("WIP_SCOPE", m::WipScope::ALL, m::WipScope::as_str, |n| {
        c::WipScope::try_from(n).ok().map(|e| e.as_str_name())
    });
    same("ROLE", m::Role::ALL, m::Role::as_str, |n| {
        c::Role::try_from(n).ok().map(|e| e.as_str_name())
    });
    same("ITEM_TYPE", m::ItemType::ALL, m::ItemType::as_str, |n| {
        c::ItemType::try_from(n).ok().map(|e| e.as_str_name())
    });
    same("PLATFORM", m::Platform::ALL, m::Platform::as_str, |n| {
        c::Platform::try_from(n).ok().map(|e| e.as_str_name())
    });
    same("SIZE", m::Size::ALL, m::Size::as_str, |n| {
        c::Size::try_from(n).ok().map(|e| e.as_str_name())
    });
    same("PRIORITY", m::Priority::ALL, m::Priority::as_str, |n| {
        c::Priority::try_from(n).ok().map(|e| e.as_str_name())
    });
    same(
        "PERSON_ROLE",
        m::PersonRole::ALL,
        m::PersonRole::as_str,
        |n| c::PersonRole::try_from(n).ok().map(|e| e.as_str_name()),
    );
    same(
        "VERIFICATION_RESULT",
        m::VerificationResult::ALL,
        m::VerificationResult::as_str,
        |n| {
            c::VerificationResult::try_from(n)
                .ok()
                .map(|e| e.as_str_name())
        },
    );
    same("LINK_KIND", m::LinkKind::ALL, m::LinkKind::as_str, |n| {
        c::LinkKind::try_from(n).ok().map(|e| e.as_str_name())
    });
    same(
        "ITEM_EVENT_KIND",
        m::ItemEventKind::ALL,
        m::ItemEventKind::as_str,
        |n| c::ItemEventKind::try_from(n).ok().map(|e| e.as_str_name()),
    );
    same(
        "TEMPLATE_KIND",
        m::TemplateKind::ALL,
        m::TemplateKind::as_str,
        |n| c::TemplateKind::try_from(n).ok().map(|e| e.as_str_name()),
    );
}
