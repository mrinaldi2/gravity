//! The only place the board meets the wire contract: conversions between the
//! repository's types (`board::model`) and the protobuf messages generated
//! from `proto/hermes/board/v1` (`bus::contract::board`).
//!
//! model → contract is infallible. contract → model is `TryFrom`: protobuf
//! enums are open (a newer client may send a number this build does not
//! know) and message fields may be unset, so a value the model cannot hold is
//! refused rather than guessed. Enum matches are exhaustive both ways, so a
//! variant added on either side fails to compile until it is mapped.

use bus::contract::board as c;
use bus::contract::pbjson_types::Timestamp;
use chrono::{DateTime, Utc};

use super::model as m;

/// A contract value the model has no place for.
#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum MapError {
    #[error("{0} is unset or not a known value")]
    UnknownEnum(&'static str),
    #[error("{0} is missing")]
    Missing(&'static str),
    #[error("{0} is out of range")]
    OutOfRange(&'static str),
}

macro_rules! map_enum {
    ($name:ident { $($variant:ident),+ $(,)? }) => {
        impl From<m::$name> for c::$name {
            fn from(value: m::$name) -> Self {
                match value {
                    $(m::$name::$variant => c::$name::$variant),+
                }
            }
        }

        impl From<m::$name> for i32 {
            fn from(value: m::$name) -> Self {
                c::$name::from(value) as i32
            }
        }

        impl TryFrom<c::$name> for m::$name {
            type Error = MapError;

            fn try_from(value: c::$name) -> Result<Self, MapError> {
                match value {
                    c::$name::Unspecified => Err(MapError::UnknownEnum(stringify!($name))),
                    $(c::$name::$variant => Ok(m::$name::$variant)),+
                }
            }
        }

        impl m::$name {
            /// From the number a message field carries.
            pub fn from_wire(number: i32) -> Result<Self, MapError> {
                c::$name::try_from(number)
                    .map_err(|_| MapError::UnknownEnum(stringify!($name)))?
                    .try_into()
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
    Ranked,
    TaskDone,
    TaskExpired,
    TaskCancelled,
    Unlinked
});
map_enum!(TemplateKind {
    ItemType,
    MeetingType
});

/// Each enum's spellings, short (the model's, which bots use: "doing",
/// "reviewer.arch") and wire ("COLUMN_CATEGORY_DOING"), by proto enum name.
macro_rules! spellings {
    ($($name:ident),+ $(,)?) => {
        pub fn spellings(enum_name: &str) -> Option<Vec<(&'static str, &'static str)>> {
            match enum_name {
                $(stringify!($name) => Some(
                    m::$name::ALL
                        .iter()
                        .map(|v| (v.as_str(), c::$name::from(*v).as_str_name()))
                        .collect(),
                ),)+
                _ => None,
            }
        }

        /// Every enum's spellings at once, for rewriting output.
        pub fn all_spellings() -> Vec<(&'static str, &'static str)> {
            [$(stringify!($name)),+]
                .iter()
                .flat_map(|name| spellings(name).unwrap_or_default())
                .collect()
        }
    };
}

spellings!(
    ColumnCategory,
    WipScope,
    Role,
    ItemType,
    Platform,
    Size,
    Priority,
    PersonRole,
    VerificationResult,
    LinkKind,
    ItemEventKind,
    TemplateKind,
);

pub(crate) fn stamp(at: DateTime<Utc>) -> Option<Timestamp> {
    Some(Timestamp {
        seconds: at.timestamp(),
        nanos: i32::try_from(at.timestamp_subsec_nanos()).unwrap_or(0),
    })
}

pub(crate) fn at(value: Option<Timestamp>, field: &'static str) -> Result<DateTime<Utc>, MapError> {
    let t = value.ok_or(MapError::Missing(field))?;
    let nanos = u32::try_from(t.nanos).map_err(|_| MapError::OutOfRange(field))?;
    DateTime::from_timestamp(t.seconds, nanos).ok_or(MapError::OutOfRange(field))
}

pub(crate) fn maybe_at(
    value: Option<Timestamp>,
    field: &'static str,
) -> Result<Option<DateTime<Utc>>, MapError> {
    value.map(|t| at(Some(t), field)).transpose()
}

pub(crate) fn wire_list<E: Into<i32>>(values: Vec<E>) -> Vec<i32> {
    values.into_iter().map(Into::into).collect()
}

pub(crate) fn model_list<E>(
    numbers: Vec<i32>,
    from: fn(i32) -> Result<E, MapError>,
) -> Result<Vec<E>, MapError> {
    numbers.into_iter().map(from).collect()
}

pub(crate) fn all<A, B: TryFrom<A, Error = MapError>>(items: Vec<A>) -> Result<Vec<B>, MapError> {
    items.into_iter().map(B::try_from).collect()
}

mod entities;
mod items;
#[cfg(test)]
mod tests;
