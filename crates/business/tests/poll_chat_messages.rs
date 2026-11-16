//! Port of `packages/business/__tests__/poll-chat-messages.test.ts` (pure
//! functions; the flow-config step schemas are `chatbotx_core::flow_config`).

use chatbotx_business::poll::PollState;
use chatbotx_business::poll::bot_strings::get_poll_bot_strings;
use chatbotx_business::poll::chat_messages::{
    CandidateCard, PollCard, build_candidates_message, build_poll_cards_message,
    build_vote_registered_message, format_poll_subtitle,
};
use chatbotx_core::flow_config as fc;
use serde_json::json;

/// Unix ms of an ISO time.
fn ms(iso: &str) -> i64 {
    chrono::DateTime::parse_from_rfc3339(iso)
        .expect("ISO time")
        .timestamp_millis()
}

const IMAGE: &str = "https://example.com/p.png";
const STARTS: &str = "2026-07-05T16:23:00Z";

// --- bot strings ---------------------------------------------------------------------

#[test]
fn bot_strings_falls_back_to_english_for_an_unknown_locale() {
    assert_eq!(
        get_poll_bot_strings("xx").vote_registered,
        "Your vote was successfully registered."
    );
}

#[test]
fn bot_strings_translates_vi() {
    assert_ne!(get_poll_bot_strings("vi").vote, "Vote");
}

#[test]
fn bot_strings_matches_by_base_language() {
    assert!(std::ptr::eq(
        get_poll_bot_strings("pt-AO"),
        get_poll_bot_strings("pt-BR")
    ));
    assert_eq!(
        get_poll_bot_strings("zh").polls,
        get_poll_bot_strings("zh-CN").polls
    );
}

// --- formatPollSubtitle --------------------------------------------------------------

#[test]
fn format_poll_subtitle_formats_label_and_dates_in_the_workspace_timezone() {
    let text = format_poll_subtitle(
        PollState::Running,
        ms(STARTS),
        ms("2026-07-20T00:00:00Z"),
        "en",
        "Asia/Ho_Chi_Minh",
    );
    assert!(text.starts_with("Voting started | Jul 5, 2026"), "{text}");
    assert!(text.contains("11:23"), "{text}");
    assert!(text.chars().count() <= 80, "{text}");
}

#[test]
fn format_poll_subtitle_attaches_a_lowercase_am_pm_to_the_time_without_a_space() {
    let text = format_poll_subtitle(
        PollState::Running,
        ms("2026-07-05T00:00:00Z"),
        ms("2026-07-12T13:30:00Z"),
        "en",
        "UTC",
    );
    assert_eq!(
        text,
        "Voting started | Jul 5, 2026, 12:00am - Jul 12, 2026, 1:30pm"
    );
}

#[test]
fn format_poll_subtitle_leaves_24_hour_locales_unchanged() {
    let text = format_poll_subtitle(
        PollState::Running,
        ms("2026-07-05T00:00:00Z"),
        ms("2026-07-12T13:30:00Z"),
        "de",
        "UTC",
    );
    assert_eq!(
        text,
        "Abstimmung gestartet | 05.07.2026, 00:00 - 12.07.2026, 13:30"
    );
}

#[test]
fn format_poll_subtitle_with_an_invalid_workspace_timezone_falls_back_to_utc_instead_of_throwing() {
    let text = format_poll_subtitle(
        PollState::Running,
        ms(STARTS),
        ms("2026-07-20T00:00:00Z"),
        "en",
        "Not/AZone",
    );
    assert!(text.starts_with("Voting started | Jul 5, 2026"), "{text}");
    assert!(text.contains("4:23"), "{text}");
}

// --- buildPollCardsMessage -----------------------------------------------------------

#[test]
fn build_poll_cards_message_adds_vote_only_on_running_polls() {
    let polls = [
        PollCard {
            id: "1".into(),
            name: "Running".into(),
            image_url: IMAGE.into(),
            starts_at: ms(STARTS),
            ends_at: ms("2026-07-20T00:00:00Z"),
        },
        PollCard {
            id: "2".into(),
            name: "Ended".into(),
            image_url: IMAGE.into(),
            starts_at: ms(STARTS),
            ends_at: ms("2026-07-06T00:00:00Z"),
        },
    ];
    let message = build_poll_cards_message(
        &polls,
        "messenger",
        get_poll_bot_strings("en"),
        "en",
        "UTC",
        "e1",
        ms("2026-07-10T00:00:00Z"),
    );
    let parsed = fc::send_carousel_step_schema().safe_parse(&message.step);
    assert!(parsed.success, "{:?}", parsed.error);
    assert_eq!(message.step["cards"][1]["buttons"], json!([]));
    let buttons = message.step["cards"][0]["buttons"]
        .as_array()
        .expect("buttons");
    assert_eq!(buttons.len(), 1);
    assert_eq!(buttons[0]["label"], json!("Vote"));
    let id = buttons[0]["id"].as_str().expect("button id");
    assert_eq!(
        message.button_payloads[id]["payload"],
        json!({ "type": "poll_open", "pollId": "1" })
    );
}

// --- buildCandidatesMessage ----------------------------------------------------------

#[test]
fn build_candidates_message_builds_one_voting_card_per_candidate_with_image_fallback() {
    let candidates = [
        CandidateCard {
            id: "c1".into(),
            name: "A".into(),
            description: Some("x".repeat(200)),
            image_url: Some("https://example.com/a.png".into()),
        },
        CandidateCard {
            id: "c2".into(),
            name: "B".into(),
            description: None,
            image_url: None,
        },
    ];
    let message =
        build_candidates_message("9", IMAGE, &candidates, get_poll_bot_strings("en"), "e1");
    let parsed = fc::send_carousel_step_schema().safe_parse(&message.step);
    assert!(parsed.success, "{:?}", parsed.error);
    let cards = message.step["cards"].as_array().expect("cards");
    assert_eq!(cards.len(), 2);
    assert!(
        cards[0]["subtitle"]
            .as_str()
            .is_some_and(|s| s.chars().count() <= 80)
    );
    assert_eq!(cards[0]["image"]["url"], json!("https://example.com/a.png"));
    assert_eq!(cards[1]["image"]["url"], json!(IMAGE));
    let id = cards[1]["buttons"][0]["id"].as_str().expect("button id");
    assert_eq!(
        message.button_payloads[id]["payload"],
        json!({ "type": "poll_vote", "pollId": "9", "candidateId": "c2" })
    );
}

// --- buildVoteRegisteredMessage ------------------------------------------------------

#[test]
fn build_vote_registered_message_uses_send_text_on_whatsapp() {
    let message = build_vote_registered_message("whatsapp", get_poll_bot_strings("en"), "e1");
    assert_eq!(message.step["stepType"], json!("sendText"));
    let parsed = fc::send_text_step_schema().safe_parse(&message.step);
    assert!(parsed.success, "{:?}", parsed.error);
    let first = message
        .button_payloads
        .values()
        .next()
        .expect("one payload");
    assert_eq!(first["payload"], json!({ "type": "poll_list" }));
}

#[test]
fn build_vote_registered_message_uses_send_quick_reply_on_messenger() {
    let message = build_vote_registered_message("messenger", get_poll_bot_strings("en"), "e1");
    assert_eq!(message.step["stepType"], json!("sendQuickReply"));
    let parsed = fc::send_quick_reply_step_schema().safe_parse(&message.step);
    assert!(parsed.success, "{:?}", parsed.error);
}
