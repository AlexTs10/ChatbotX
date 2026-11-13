//! `poll/links.ts` (`buildPollLinks`).

use crate::inbox::links::{LinkInbox, can_receive_ref, get_inbox_links};
use crate::referral::RefConfig;

/// One chat link of a poll.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PollLink {
    pub inbox_id: i64,
    pub channel: String,
    pub name: String,
    pub url: String,
}

/// `PollLinks`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PollLinks {
    pub omnichannel_url: String,
    pub links: Vec<PollLink>,
}

/// `buildPollLinks`: the omnichannel link and one link per ref-capable inbox.
pub fn build_poll_links(
    app_url: &str,
    workspace_id: i64,
    inboxes: &[LinkInbox],
    poll_id: i64,
) -> PollLinks {
    todo!("P3.6-D1")
}
