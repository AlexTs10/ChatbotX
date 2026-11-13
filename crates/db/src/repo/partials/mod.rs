//! Port of the parts of `packages/database/src/partials/` that the ported repositories
//! use, and the zod partials that have their own tests (P3.2b; a zod schema is a serde
//! type with a `parse_*` function that runs the zod checks).

pub mod appointment;
pub mod contact;
pub mod contact_activity;
pub mod credential;
pub mod shared;
pub mod thread_control;
pub mod whatsapp_call;
