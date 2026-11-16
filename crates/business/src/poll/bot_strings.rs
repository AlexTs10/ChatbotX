//! `poll/bot-strings.ts`: the poll bot texts per locale.

/// `PollBotStrings`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PollBotStrings {
    pub vote: &'static str,
    pub polls: &'static str,
    pub voting_started: &'static str,
    pub voting_is_over: &'static str,
    pub voting_starts_soon: &'static str,
    pub vote_registered: &'static str,
    pub no_open_polls: &'static str,
}

/// `getPollBotStrings`: the exact locale, then its base language, then `en`.
pub fn get_poll_bot_strings(_language: &str) -> &'static PollBotStrings {
    todo!("P3.6-D1")
}
