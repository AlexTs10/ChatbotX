//! `packages/business/src/poll`: polls, their candidates and votes.
//!
//! Not ported yet: `chat-messages.ts` / `bot-strings.ts` (the poll cards a flow
//! sends; worker side) and the template edit locks of `create` / `update`
//! (`withTemplateLockedTransaction` / `assertReferencable`, template unit).

pub mod analytics;
pub mod links;
pub mod service;
pub mod state;
pub mod vote;

pub use analytics::{PollAnalytics, PollCandidateAnalytics, compute_poll_analytics};
pub use links::{PollLink, PollLinks, build_poll_links};
pub use state::{PollState, get_poll_state};
