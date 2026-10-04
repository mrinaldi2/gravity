//! The only place the board meets the wire contract: conversions between the
//! repository's types (`board::model`) and the contract's (`bus::contract`).
//! A codegen switch (protobuf, ruling dcf069e2) rewrites this file and nothing
//! else. Enum matches are exhaustive in both directions, so a variant added on
//! either side fails to compile until it is mapped.

use bus::contract::board as c;

use super::model as m;

macro_rules! map_enum {
    ($name:ident { $($variant:ident),+ $(,)? }) => {
        impl From<m::$name> for c::$name {
            fn from(value: m::$name) -> Self {
                match value {
                    $(m::$name::$variant => c::$name::$variant),+
                }
            }
        }

        impl From<c::$name> for m::$name {
            fn from(value: c::$name) -> Self {
                match value {
                    $(c::$name::$variant => m::$name::$variant),+
                }
            }
        }
    };
}

map_enum!(ColumnCategory {
    Inbox,
    Ready,
    Doing,
    Review,
    Verify,
    Approval,
    Deploying,
    Done,
    Cancelled
});
map_enum!(WipScope {
    Column,
    PerAssignee
});
map_enum!(Role {
    Lead,
    Coach,
    Devops,
    ReviewerArch,
    ReviewerUx,
    Tester,
    Dev
});
map_enum!(ItemType {
    Epic,
    Feature,
    Bug,
    Spike,
    Chore
});
map_enum!(Platform {
    Desktop,
    Ios,
    Daemon,
    Infra
});
map_enum!(Size { S, M, L });
map_enum!(Priority { P0, P1, P2, P3 });
map_enum!(PersonRole { Reviewer, Verifier });
map_enum!(VerificationResult {
    Pass,
    Fail,
    Blocked
});
map_enum!(LinkKind {
    Task,
    Decision,
    Artifact,
    Branch,
    Pr,
    Meeting,
    ItemBlocks,
    ItemRelates,
    ItemDuplicates
});
map_enum!(ItemEventKind {
    Created,
    Moved,
    Edited,
    Commented,
    Linked,
    Assigned,
    Blocked,
    Ranked
});
map_enum!(TemplateKind {
    ItemType,
    MeetingType
});

fn all<A, B: From<A>>(items: Vec<A>) -> Vec<B> {
    items.into_iter().map(B::from).collect()
}

/// A struct mapped field by field in both directions; `[list]` fields map
/// their elements, `(opt)` fields their contents.
macro_rules! map_struct {
    ($name:ident { $($field:ident),* } lists { $($list:ident),* } opts { $($opt:ident),* } plain { $($plain:ident),* }) => {
        impl From<m::$name> for c::$name {
            fn from(v: m::$name) -> Self {
                c::$name {
                    $($field: v.$field.into(),)*
                    $($list: all(v.$list),)*
                    $($opt: v.$opt.map(Into::into),)*
                    $($plain: v.$plain,)*
                }
            }
        }

        impl From<c::$name> for m::$name {
            fn from(v: c::$name) -> Self {
                m::$name {
                    $($field: v.$field.into(),)*
                    $($list: all(v.$list),)*
                    $($opt: v.$opt.map(Into::into),)*
                    $($plain: v.$plain,)*
                }
            }
        }
    };
}

map_struct!(BoardColumn { category, wip_scope } lists {} opts {}
    plain { project_id, key, name, ord, wip_limit, visible });
map_struct!(ProjectRole { role } lists {} opts {} plain { project_id, bot_id, machine });
map_struct!(Blocked {} lists {} opts {} plain { by, reason, since });
map_struct!(AcceptanceCriterion {} lists {} opts {}
    plain { idx, text, checked, checked_by, checked_at, machine });
map_struct!(ItemPerson { role } lists {} opts {} plain { bot_id });
map_struct!(ItemVerification { result } lists {} opts {} plain { machine, by, at, note });
map_struct!(Item { item_type, priority, category }
    lists { platforms, acceptance_criteria, people, verifications }
    opts { size, blocked }
    plain { id, seq, title, description, rank, column_key, assignee, parent_id, release_id,
            labels, created_by, created_at, updated_at, state_entered_at, done_at, version });
map_struct!(ItemCard { item_type, priority } lists { platforms } opts { size }
    plain { id, title, rank, column_key, assignee, labels, blocked, stale, ac_checked, ac_total,
            version });
map_struct!(ItemLink { kind } lists {} opts {} plain { item_id, target, label, created_by, at });
map_struct!(ItemComment {} lists {} opts {} plain { id, item_id, author, body, reply_to, at });
map_struct!(ItemEvent { kind } lists {} opts {}
    plain { id, item_id, at, actor, from, to, field, note });
map_struct!(Template { kind } lists {} opts {} plain { project_id, name, version, body });

impl From<m::BoardSettings> for c::BoardSettings {
    fn from(v: m::BoardSettings) -> Self {
        c::BoardSettings {
            project_id: v.project_id,
            key: v.key,
            next_seq: v.next_seq,
            stale_after_hours: v.stale_after_hours,
            required_machines: v
                .required_machines
                .into_iter()
                .map(|(p, ms)| (p.into(), ms))
                .collect(),
            home_daemon_id: v.home_daemon_id,
            version: v.version,
        }
    }
}

impl From<c::BoardSettings> for m::BoardSettings {
    fn from(v: c::BoardSettings) -> Self {
        m::BoardSettings {
            project_id: v.project_id,
            key: v.key,
            next_seq: v.next_seq,
            stale_after_hours: v.stale_after_hours,
            required_machines: v
                .required_machines
                .into_iter()
                .map(|(p, ms)| (p.into(), ms))
                .collect(),
            home_daemon_id: v.home_daemon_id,
            version: v.version,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use serde::de::DeserializeOwned;

    use super::*;

    fn fixture<T: DeserializeOwned>(name: &str) -> T {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join(format!("../bus/fixtures/board/{name}.json"));
        serde_json::from_str(&std::fs::read_to_string(&path).expect("fixture")).expect("decodes")
    }

    /// contract → model → contract changes nothing, for every golden fixture.
    fn through_the_model<C, M>(name: &str)
    where
        C: DeserializeOwned + Clone + PartialEq + std::fmt::Debug + From<M>,
        M: From<C>,
    {
        let wire: C = fixture(name);
        let back: C = M::from(wire.clone()).into();
        assert_eq!(back, wire, "{name}");
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
    }

    /// The stored spelling of every variant is its wire spelling, so rows
    /// written before a codegen switch still read as the same values.
    #[test]
    fn stored_and_wire_spellings_agree() {
        fn same<M: Copy, C: serde::Serialize + From<M>>(
            all: &[M],
            text: impl Fn(M) -> &'static str,
        ) {
            for value in all {
                let wire = serde_json::to_value(C::from(*value)).expect("encodes");
                assert_eq!(wire, serde_json::Value::String(text(*value).to_string()));
            }
        }
        same::<_, c::ColumnCategory>(m::ColumnCategory::ALL, m::ColumnCategory::as_str);
        same::<_, c::WipScope>(m::WipScope::ALL, m::WipScope::as_str);
        same::<_, c::Role>(m::Role::ALL, m::Role::as_str);
        same::<_, c::ItemType>(m::ItemType::ALL, m::ItemType::as_str);
        same::<_, c::Platform>(m::Platform::ALL, m::Platform::as_str);
        same::<_, c::Size>(m::Size::ALL, m::Size::as_str);
        same::<_, c::Priority>(m::Priority::ALL, m::Priority::as_str);
        same::<_, c::PersonRole>(m::PersonRole::ALL, m::PersonRole::as_str);
        same::<_, c::VerificationResult>(m::VerificationResult::ALL, m::VerificationResult::as_str);
        same::<_, c::LinkKind>(m::LinkKind::ALL, m::LinkKind::as_str);
        same::<_, c::ItemEventKind>(m::ItemEventKind::ALL, m::ItemEventKind::as_str);
        same::<_, c::TemplateKind>(m::TemplateKind::ALL, m::TemplateKind::as_str);
    }
}
