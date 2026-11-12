//! Business logic for ChatbotX, shared by `api`, `jobs` and `rt`.
//!
//! P3.1 ports the shared backend packages:
//!
//! | TypeScript package            | Rust module          |
//! |-------------------------------|----------------------|
//! | `packages/utils`              | [`utils`]            |
//! | `packages/encryption`         | [`encryption`]       |
//! | `packages/variables`          | [`variables`]        |
//! | `packages/logger`             | [`logger`]           |
//! | `packages/decision-config`    | [`decision_config`]  |
//!
//! [`env`] and [`js`] hold the small runtime shims the ports need (an
//! injectable environment, and the JavaScript number / string / date
//! semantics the original code relies on).

/// A string enum (a zod `z.enum([...])`): `ALL` in declaration order,
/// `as_str`, `parse`, `Display` and serde as the string.
macro_rules! string_enum {
    ($(#[$meta:meta])* pub enum $name:ident { $($(#[$vmeta:meta])* $variant:ident = $text:literal),+ $(,)? }) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub enum $name { $($(#[$vmeta])* $variant),+ }

        impl $name {
            /// Every value, in declaration order (`.options`).
            pub const ALL: &'static [$name] = &[$($name::$variant),+];

            /// The string value.
            pub fn as_str(self) -> &'static str {
                match self { $($name::$variant => $text),+ }
            }

            /// The value for a string, or `None` when it is not one.
            pub fn parse(value: &str) -> Option<$name> {
                match value { $($text => Some($name::$variant),)+ _ => None }
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(self.as_str())
            }
        }

        impl serde::Serialize for $name {
            fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                s.serialize_str(self.as_str())
            }
        }

        impl<'de> serde::Deserialize<'de> for $name {
            fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<$name, D::Error> {
                let text = <String as serde::Deserialize>::deserialize(d)?;
                $name::parse(&text).ok_or_else(|| {
                    serde::de::Error::custom(format!("Invalid option: {text}"))
                })
            }
        }
    };
}

pub mod decision_config;
pub mod encryption;
pub mod env;
pub mod js;
pub mod logger;
pub mod utils;
pub mod variables;

pub use chatbotx_kv::{Clock, SystemClock, TestClock};
