use std::{fmt, str::FromStr};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{Result, WonderError};

macro_rules! id_type {
    ($name:ident) => {
        #[doc = concat!("Identifies a ", stringify!($name), " value")]
        #[derive(
            Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize,
        )]
        #[serde(transparent)]
        pub struct $name(Uuid);

        impl $name {
            /// Creates a new identifier
            #[must_use]
            pub fn new() -> Self {
                Self(Uuid::new_v4())
            }

            /// Wraps an existing [`Uuid`]
            #[must_use]
            pub const fn from_uuid(uuid: Uuid) -> Self {
                Self(uuid)
            }

            /// Returns the underlying [`Uuid`]
            #[must_use]
            pub const fn as_uuid(self) -> Uuid {
                self.0
            }

            /// Parses an identifier from a string
            pub fn parse(value: &str) -> Result<Self> {
                value.parse::<Uuid>().map(Self).map_err(|err| {
                    WonderError::validation(format!("invalid {}: {err}", stringify!($name)))
                })
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(formatter)
            }
        }

        impl From<Uuid> for $name {
            fn from(value: Uuid) -> Self {
                Self(value)
            }
        }

        impl From<$name> for Uuid {
            fn from(value: $name) -> Self {
                value.0
            }
        }

        impl FromStr for $name {
            type Err = uuid::Error;

            fn from_str(value: &str) -> std::result::Result<Self, Self::Err> {
                value.parse::<Uuid>().map(Self)
            }
        }
    };
}

id_type!(SessionId);
id_type!(MessageId);
id_type!(CommandId);
id_type!(ToolUseId);
id_type!(TaskId);
id_type!(FleetId);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn id_display_round_trips() {
        let id = SessionId::new();
        let parsed = SessionId::parse(&id.to_string()).expect("valid id");
        assert_eq!(id, parsed);
    }

    #[test]
    fn parse_rejects_invalid_id() {
        let error = MessageId::parse("not-a-uuid").expect_err("invalid id");
        assert!(error.to_string().contains("invalid MessageId"));
    }
}
