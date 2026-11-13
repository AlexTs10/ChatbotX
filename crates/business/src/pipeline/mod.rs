//! `packages/business/src/pipeline`: pipelines, stages, custom fields, members,
//! items and their activity. Money (`PipelineItem.value`, activity values,
//! stage totals) stays the stored x 10,000 integer in this module; the API
//! layer converts it to major units.

pub mod access;
pub mod currency;
pub mod events;
pub mod naming;
pub mod service;
pub mod settings;
pub mod structure;

pub use access::{PipelineAccessScope, PipelineAction, PipelinePermissions};
pub use currency::{EffectiveCurrency, PipelineView};
