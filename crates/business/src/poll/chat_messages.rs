//! `poll/chat-messages.ts`: the flow steps the poll bot sends (poll cards,
//! candidate cards, the vote-registered reply) and their button payloads.

use std::collections::BTreeMap;

use serde_json::Value;

use super::PollState;
use super::bot_strings::PollBotStrings;

/// `PollChatMessage`: the step (flow-config JSON) and the payload of each of
/// its buttons by button id (`RichButtonPayloadEntry`).
#[derive(Debug, Clone, PartialEq)]
pub struct PollChatMessage {
    pub step: Value,
    pub button_payloads: BTreeMap<String, Value>,
}

/// One poll of [`build_poll_cards_message`] (times in unix ms).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PollCard {
    pub id: String,
    pub name: String,
    pub image_url: String,
    pub starts_at: i64,
    pub ends_at: i64,
}

/// One candidate of [`build_candidates_message`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandidateCard {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub image_url: Option<String>,
}

/// `formatPollSubtitle`: `<state label> | <start> - <end>` in the workspace
/// time zone (`Intl` medium date + short time; an invalid zone or locale is
/// `en` / UTC), at most 80 characters.
pub fn format_poll_subtitle(
    _state: PollState,
    _starts_at: i64,
    _ends_at: i64,
    _language: &str,
    _timezone: &str,
) -> String {
    todo!("P3.6-D1")
}

/// `buildPollCardsMessage`: one card per poll; `Vote` only on running polls.
#[allow(clippy::too_many_arguments)]
pub fn build_poll_cards_message(
    _polls: &[PollCard],
    _channel: &str,
    _strings: &PollBotStrings,
    _language: &str,
    _timezone: &str,
    _execution_id: &str,
    _now: i64,
) -> PollChatMessage {
    todo!("P3.6-D1")
}

/// `buildCandidatesMessage`: one voting card per candidate (the poll image
/// when the candidate has none).
pub fn build_candidates_message(
    _poll_id: &str,
    _poll_image_url: &str,
    _candidates: &[CandidateCard],
    _strings: &PollBotStrings,
    _execution_id: &str,
) -> PollChatMessage {
    todo!("P3.6-D1")
}

/// `buildVoteRegisteredMessage`: `sendText` on WhatsApp, `sendQuickReply`
/// elsewhere, with a `poll_list` button.
pub fn build_vote_registered_message(
    _channel: &str,
    _strings: &PollBotStrings,
    _execution_id: &str,
) -> PollChatMessage {
    todo!("P3.6-D1")
}
