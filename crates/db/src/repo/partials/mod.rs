//! Port of the parts of `packages/database/src/partials/` that the ported repositories
//! use, and the zod partials that have their own tests (P3.2b; a zod schema is a serde
//! type with a `parse_*` function that runs the zod checks).

pub mod ai;
pub mod appointment;
pub mod automated_response;
pub mod broadcast;
pub mod broadcast_plan_policy;
pub mod channel;
pub mod comment_automation;
pub mod contact;
pub mod contact_activity;
pub mod credential;
pub mod integration_whatsapp;
pub mod keyword_condition_schema;
pub mod messaging_ad;
pub mod shared;
pub mod thread_control;
pub mod whatsapp_call;
pub mod working_hours;
