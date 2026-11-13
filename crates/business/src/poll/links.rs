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
    let links = get_inbox_links(app_url, inboxes, Some(&RefConfig::Poll { poll_id }))
        .into_iter()
        .filter(|(inbox, _)| can_receive_ref(&inbox.channel))
        .map(|(inbox, url)| PollLink {
            inbox_id: inbox.id,
            channel: inbox.channel,
            name: inbox.name,
            url,
        })
        .collect();
    PollLinks {
        omnichannel_url: format!("{app_url}/l/{workspace_id}/p/{poll_id}"),
        links,
    }
}
