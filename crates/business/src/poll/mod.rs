//! `packages/business/src/poll`: polls, their candidates and votes.
//!
//! The poll cards a flow sends (`chat-messages.ts`) run in the worker
//! (`rich_response`). The template locks of `create` / `update`
//! (`withTemplateLockedTransaction` / `assertReferencable`) are not needed:
//! SQLite serializes the writes, so an uninstall never runs between the
//! check and the insert.

pub mod analytics;
pub mod bot_strings;
pub mod chat_messages;
pub mod links;
pub mod service;
pub mod state;
pub mod vote;

pub use analytics::{PollAnalytics, PollCandidateAnalytics, compute_poll_analytics};
pub use links::{PollLink, PollLinks, build_poll_links};
pub use state::{PollState, get_poll_state};
