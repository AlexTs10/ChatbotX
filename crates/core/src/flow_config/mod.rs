//! Port of `packages/flow-config`: the flow graph schemas (steps, nodes,
//! states), channel rules, import/export remapping and flow authoring.
//!
//! Schemas are built with [`z`], a runtime port of the zod 4 semantics the
//! TypeScript package relies on (same paths, messages, defaults). The
//! skeletons were generated once from the zod definitions by
//! `conversion/tools/zod-rust-gen`; closures are hand-ported.
//!
//! Dropped (DECISIONS 2026-11-11): the Execute JavaScript step.

#![allow(clippy::too_many_lines)]

/// Defines `fn name() -> Schema`, built once.
macro_rules! schema {
    ($vis:vis fn $name:ident = $e:expr) => {
        $vis fn $name() -> $crate::flow_config::z::Schema {
            todo!(concat!("P3.4 ", stringify!($name)))
        }
    };
}

/// Defines a `*DefaultFn` (fresh ids on every call).
macro_rules! default_fn {
    ($vis:vis fn $name:ident = $e:expr) => {
        $vis fn $name() -> serde_json::Value {
            todo!(concat!("P3.4 ", stringify!($name)))
        }
    };
}

pub mod authoring;
pub mod channel_rules;
pub mod deep_link;
pub mod definition;
pub mod deps;
pub mod event;
pub mod field_reference;
pub mod import_export;
pub mod migrations;
pub mod nodes;
pub mod publish_schema;
pub mod routable_handle;
pub mod shared;
pub mod states;
pub mod step_input_resolution;
pub mod steps;
pub mod template_structure;
pub mod types;
pub mod util;
pub mod validation_codes;
pub mod z;

pub use authoring::*;
pub use channel_rules::*;
pub use deep_link::*;
pub use definition::*;
pub use event::*;
pub use field_reference::*;
pub use import_export::*;
pub use migrations::*;
pub use nodes::*;
pub use publish_schema::*;
pub use routable_handle::*;
pub use shared::*;
pub use states::*;
pub use step_input_resolution::*;
pub use steps::*;
pub use template_structure::*;
pub use types::*;
pub use util::*;
pub use validation_codes::*;

/// Imports shared by every flow-config module.
pub mod prelude {
    pub use std::sync::LazyLock;

    pub use serde_json::{Map, Value, json};

    pub use super::deps;
    pub use super::z::{self, IssueCode, PathSegment, Payload, RawIssue, RefineCtx, Schema, path};
    pub use super::*;
    pub use crate::utils::id::create_id;
}
