//! `poll/chat-messages.ts`: the flow steps the poll bot sends (poll cards,
//! candidate cards, the vote-registered reply) and their button payloads.

use std::collections::BTreeMap;

use chatbotx_core::flow_config as fc;
use chatbotx_core::utils::id::create_id;
use chrono::{Datelike, TimeZone, Timelike};
use serde_json::{Value, json};

use super::bot_strings::{PollBotStrings, get_poll_bot_strings};
use super::{PollState, get_poll_state};

const CARD_TEXT_MAX: usize = 80;
const DEFAULT_BUTTON_LABEL_MAX: usize = 20;

/// One part of a formatted date (`Intl.DateTimeFormat#formatToParts`).
#[derive(Debug, Clone, Copy)]
enum Tok {
    Year,
    MonthName,
    /// `true` = two digits.
    MonthNum(bool),
    Day(bool),
    Hour(bool),
    Minute,
    DayPeriod,
    Lit(&'static str),
}

struct LocaleFormat {
    locale: &'static str,
    tokens: &'static [Tok],
    months: [&'static str; 12],
    am: &'static str,
    pm: &'static str,
    hour_cycle: &'static str,
}

/// CLDR medium date + short time (Node ICU) of the bot locales.
const FORMATS: &[LocaleFormat] = &[
    LocaleFormat {
        locale: "en",
        tokens: &[
            Tok::MonthName,
            Tok::Lit(" "),
            Tok::Day(false),
            Tok::Lit(", "),
            Tok::Year,
            Tok::Lit(", "),
            Tok::Hour(false),
            Tok::Lit(":"),
            Tok::Minute,
            Tok::Lit(" "),
            Tok::DayPeriod,
        ],
        months: [
            "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
        ],
        am: "AM",
        pm: "PM",
        hour_cycle: "h12",
    },
    LocaleFormat {
        locale: "ar",
        tokens: &[
            Tok::Day(true),
            Tok::Lit("‏/"),
            Tok::MonthNum(true),
            Tok::Lit("‏/"),
            Tok::Year,
            Tok::Lit("، "),
            Tok::Hour(false),
            Tok::Lit(":"),
            Tok::Minute,
            Tok::Lit(" "),
            Tok::DayPeriod,
        ],
        months: [
            "01", "02", "03", "04", "05", "06", "07", "08", "09", "10", "11", "12",
        ],
        am: "ص",
        pm: "م",
        hour_cycle: "h12",
    },
    LocaleFormat {
        locale: "az",
        tokens: &[
            Tok::Day(false),
            Tok::Lit(" "),
            Tok::MonthName,
            Tok::Lit(" "),
            Tok::Year,
            Tok::Lit(", "),
            Tok::Hour(true),
            Tok::Lit(":"),
            Tok::Minute,
        ],
        months: [
            "yan", "fev", "mar", "apr", "may", "iyn", "iyl", "avq", "sen", "okt", "noy", "dek",
        ],
        am: "",
        pm: "",
        hour_cycle: "h23",
    },
    LocaleFormat {
        locale: "da",
        tokens: &[
            Tok::Day(false),
            Tok::Lit(". "),
            Tok::MonthName,
            Tok::Lit(" "),
            Tok::Year,
            Tok::Lit(", "),
            Tok::Hour(true),
            Tok::Lit("."),
            Tok::Minute,
        ],
        months: [
            "jan.", "feb.", "mar.", "apr.", "maj", "jun.", "jul.", "aug.", "sep.", "okt.", "nov.",
            "dec.",
        ],
        am: "",
        pm: "",
        hour_cycle: "h23",
    },
    LocaleFormat {
        locale: "de",
        tokens: &[
            Tok::Day(true),
            Tok::Lit("."),
            Tok::MonthNum(true),
            Tok::Lit("."),
            Tok::Year,
            Tok::Lit(", "),
            Tok::Hour(true),
            Tok::Lit(":"),
            Tok::Minute,
        ],
        months: [
            "01", "02", "03", "04", "05", "06", "07", "08", "09", "10", "11", "12",
        ],
        am: "",
        pm: "",
        hour_cycle: "h23",
    },
    LocaleFormat {
        locale: "es",
        tokens: &[
            Tok::Day(false),
            Tok::Lit(" "),
            Tok::MonthName,
            Tok::Lit(" "),
            Tok::Year,
            Tok::Lit(", "),
            Tok::Hour(false),
            Tok::Lit(":"),
            Tok::Minute,
        ],
        months: [
            "ene", "feb", "mar", "abr", "may", "jun", "jul", "ago", "sept", "oct", "nov", "dic",
        ],
        am: "",
        pm: "",
        hour_cycle: "h23",
    },
    LocaleFormat {
        locale: "fi",
        tokens: &[
            Tok::Day(false),
            Tok::Lit("."),
            Tok::MonthNum(false),
            Tok::Lit("."),
            Tok::Year,
            Tok::Lit(" klo "),
            Tok::Hour(false),
            Tok::Lit("."),
            Tok::Minute,
        ],
        months: [
            "1", "2", "3", "4", "5", "6", "7", "8", "9", "10", "11", "12",
        ],
        am: "",
        pm: "",
        hour_cycle: "h23",
    },
    LocaleFormat {
        locale: "fr",
        tokens: &[
            Tok::Day(false),
            Tok::Lit(" "),
            Tok::MonthName,
            Tok::Lit(" "),
            Tok::Year,
            Tok::Lit(", "),
            Tok::Hour(true),
            Tok::Lit(":"),
            Tok::Minute,
        ],
        months: [
            "janv.", "févr.", "mars", "avr.", "mai", "juin", "juil.", "août", "sept.", "oct.",
            "nov.", "déc.",
        ],
        am: "",
        pm: "",
        hour_cycle: "h23",
    },
    LocaleFormat {
        locale: "he",
        tokens: &[
            Tok::Day(false),
            Tok::Lit(" ב"),
            Tok::MonthName,
            Tok::Lit(" "),
            Tok::Year,
            Tok::Lit(", "),
            Tok::Hour(false),
            Tok::Lit(":"),
            Tok::Minute,
        ],
        months: [
            "ינו׳", "פבר׳", "מרץ", "אפר׳", "מאי", "יוני", "יולי", "אוג׳", "ספט׳", "אוק׳", "נוב׳",
            "דצמ׳",
        ],
        am: "",
        pm: "",
        hour_cycle: "h23",
    },
    LocaleFormat {
        locale: "id",
        tokens: &[
            Tok::Day(false),
            Tok::Lit(" "),
            Tok::MonthName,
            Tok::Lit(" "),
            Tok::Year,
            Tok::Lit(", "),
            Tok::Hour(true),
            Tok::Lit("."),
            Tok::Minute,
        ],
        months: [
            "Jan", "Feb", "Mar", "Apr", "Mei", "Jun", "Jul", "Agu", "Sep", "Okt", "Nov", "Des",
        ],
        am: "",
        pm: "",
        hour_cycle: "h23",
    },
    LocaleFormat {
        locale: "it",
        tokens: &[
            Tok::Day(false),
            Tok::Lit(" "),
            Tok::MonthName,
            Tok::Lit(" "),
            Tok::Year,
            Tok::Lit(", "),
            Tok::Hour(true),
            Tok::Lit(":"),
            Tok::Minute,
        ],
        months: [
            "gen", "feb", "mar", "apr", "mag", "giu", "lug", "ago", "set", "ott", "nov", "dic",
        ],
        am: "",
        pm: "",
        hour_cycle: "h23",
    },
    LocaleFormat {
        locale: "ja",
        tokens: &[
            Tok::Year,
            Tok::Lit("/"),
            Tok::MonthNum(true),
            Tok::Lit("/"),
            Tok::Day(true),
            Tok::Lit(" "),
            Tok::Hour(false),
            Tok::Lit(":"),
            Tok::Minute,
        ],
        months: [
            "01", "02", "03", "04", "05", "06", "07", "08", "09", "10", "11", "12",
        ],
        am: "",
        pm: "",
        hour_cycle: "h23",
    },
    LocaleFormat {
        locale: "nl",
        tokens: &[
            Tok::Day(false),
            Tok::Lit(" "),
            Tok::MonthName,
            Tok::Lit(" "),
            Tok::Year,
            Tok::Lit(", "),
            Tok::Hour(true),
            Tok::Lit(":"),
            Tok::Minute,
        ],
        months: [
            "jan", "feb", "mrt", "apr", "mei", "jun", "jul", "aug", "sep", "okt", "nov", "dec",
        ],
        am: "",
        pm: "",
        hour_cycle: "h23",
    },
    LocaleFormat {
        locale: "pt-BR",
        tokens: &[
            Tok::Day(false),
            Tok::Lit(" de "),
            Tok::MonthName,
            Tok::Lit(" de "),
            Tok::Year,
            Tok::Lit(", "),
            Tok::Hour(true),
            Tok::Lit(":"),
            Tok::Minute,
        ],
        months: [
            "jan.", "fev.", "mar.", "abr.", "mai.", "jun.", "jul.", "ago.", "set.", "out.", "nov.",
            "dez.",
        ],
        am: "",
        pm: "",
        hour_cycle: "h23",
    },
    LocaleFormat {
        locale: "pt-PT",
        tokens: &[
            Tok::Day(true),
            Tok::Lit("/"),
            Tok::MonthNum(true),
            Tok::Lit("/"),
            Tok::Year,
            Tok::Lit(", "),
            Tok::Hour(true),
            Tok::Lit(":"),
            Tok::Minute,
        ],
        months: [
            "01", "02", "03", "04", "05", "06", "07", "08", "09", "10", "11", "12",
        ],
        am: "",
        pm: "",
        hour_cycle: "h23",
    },
    LocaleFormat {
        locale: "ro",
        tokens: &[
            Tok::Day(false),
            Tok::Lit(" "),
            Tok::MonthName,
            Tok::Lit(" "),
            Tok::Year,
            Tok::Lit(", "),
            Tok::Hour(true),
            Tok::Lit(":"),
            Tok::Minute,
        ],
        months: [
            "ian.", "feb.", "mar.", "apr.", "mai", "iun.", "iul.", "aug.", "sept.", "oct.", "nov.",
            "dec.",
        ],
        am: "",
        pm: "",
        hour_cycle: "h23",
    },
    LocaleFormat {
        locale: "sv",
        tokens: &[
            Tok::Day(false),
            Tok::Lit(" "),
            Tok::MonthName,
            Tok::Lit(" "),
            Tok::Year,
            Tok::Lit(" "),
            Tok::Hour(true),
            Tok::Lit(":"),
            Tok::Minute,
        ],
        months: [
            "jan.", "feb.", "mars", "apr.", "maj", "juni", "juli", "aug.", "sep.", "okt.", "nov.",
            "dec.",
        ],
        am: "",
        pm: "",
        hour_cycle: "h23",
    },
    LocaleFormat {
        locale: "tr",
        tokens: &[
            Tok::Day(false),
            Tok::Lit(" "),
            Tok::MonthName,
            Tok::Lit(" "),
            Tok::Year,
            Tok::Lit(" "),
            Tok::Hour(true),
            Tok::Lit(":"),
            Tok::Minute,
        ],
        months: [
            "Oca", "Şub", "Mar", "Nis", "May", "Haz", "Tem", "Ağu", "Eyl", "Eki", "Kas", "Ara",
        ],
        am: "",
        pm: "",
        hour_cycle: "h23",
    },
    LocaleFormat {
        locale: "vi",
        tokens: &[
            Tok::Hour(true),
            Tok::Lit(":"),
            Tok::Minute,
            Tok::Lit(" "),
            Tok::Day(false),
            Tok::Lit(" "),
            Tok::MonthName,
            Tok::Lit(", "),
            Tok::Year,
        ],
        months: [
            "thg 1", "thg 2", "thg 3", "thg 4", "thg 5", "thg 6", "thg 7", "thg 8", "thg 9",
            "thg 10", "thg 11", "thg 12",
        ],
        am: "",
        pm: "",
        hour_cycle: "h23",
    },
    LocaleFormat {
        locale: "zh-CN",
        tokens: &[
            Tok::Year,
            Tok::Lit("年"),
            Tok::MonthNum(false),
            Tok::Lit("月"),
            Tok::Day(false),
            Tok::Lit("日 "),
            Tok::Hour(true),
            Tok::Lit(":"),
            Tok::Minute,
        ],
        months: [
            "1", "2", "3", "4", "5", "6", "7", "8", "9", "10", "11", "12",
        ],
        am: "",
        pm: "",
        hour_cycle: "h23",
    },
    LocaleFormat {
        locale: "zh-TW",
        tokens: &[
            Tok::Year,
            Tok::Lit("年"),
            Tok::MonthNum(false),
            Tok::Lit("月"),
            Tok::Day(false),
            Tok::Lit("日 "),
            Tok::DayPeriod,
            Tok::Hour(false),
            Tok::Lit(":"),
            Tok::Minute,
        ],
        months: [
            "1", "2", "3", "4", "5", "6", "7", "8", "9", "10", "11", "12",
        ],
        am: "凌晨",
        pm: "下午",
        hour_cycle: "h12",
    },
];

/// `new Intl.DateTimeFormat(language)`: the exact locale, then its base
/// language, then `en` (ICU's default for an unknown tag).
fn locale_format(language: &str) -> &'static LocaleFormat {
    let base = |tag: &str| tag.split(['-', '_']).next().unwrap_or("").to_lowercase();
    FORMATS
        .iter()
        .find(|f| f.locale.eq_ignore_ascii_case(language))
        .or_else(|| FORMATS.iter().find(|f| base(f.locale) == base(language)))
        .unwrap_or(&FORMATS[0])
}

/// `formatCompact`: the date with a lowercase am/pm attached to the time.
fn format_compact(format: &LocaleFormat, tz: chrono_tz::Tz, at_ms: i64) -> String {
    let Some(local) = tz.timestamp_millis_opt(at_ms).single() else {
        return String::new();
    };
    let pad = |n: u32, two: bool| {
        if two {
            format!("{n:02}")
        } else {
            n.to_string()
        }
    };
    let hour = local.hour();
    let shown_hour = if format.hour_cycle == "h12" {
        match hour % 12 {
            0 => 12,
            h => h,
        }
    } else {
        hour
    };
    let mut out = String::new();
    for (index, tok) in format.tokens.iter().enumerate() {
        match tok {
            Tok::Year => out.push_str(&local.year().to_string()),
            Tok::MonthName => out.push_str(format.months[local.month0() as usize]),
            Tok::MonthNum(two) => out.push_str(&pad(local.month(), *two)),
            Tok::Day(two) => out.push_str(&pad(local.day(), *two)),
            Tok::Hour(two) => out.push_str(&pad(shown_hour, *two)),
            Tok::Minute => out.push_str(&format!("{:02}", local.minute())),
            Tok::DayPeriod => {
                let period = if hour < 12 { format.am } else { format.pm };
                out.push_str(&period.to_lowercase());
            }
            Tok::Lit(text) => {
                let before_period = matches!(format.tokens.get(index + 1), Some(Tok::DayPeriod));
                if !(before_period && !text.is_empty() && text.chars().all(char::is_whitespace)) {
                    out.push_str(text);
                }
            }
        }
    }
    out
}

/// `truncate`: at most `max` characters, with `…`.
fn truncate(value: &str, max: usize) -> String {
    let chars: Vec<char> = value.chars().collect();
    if chars.len() <= max {
        return value.to_string();
    }
    let head: String = chars[..max.saturating_sub(1)].iter().collect();
    format!("{}…", head.trim_end())
}

fn state_label(state: PollState, strings: &PollBotStrings) -> &'static str {
    match state {
        PollState::Running => strings.voting_started,
        PollState::Ended => strings.voting_is_over,
        PollState::NotStarted => strings.voting_starts_soon,
    }
}

fn button_label_max(channel: &str) -> usize {
    fc::get_channel_flow_policy(Some(channel))
        .map_or(DEFAULT_BUTTON_LABEL_MAX, |p| p.limits.button_label)
}

/// `makeButton`: a default button and its payload entry.
fn make_button(
    label: &str,
    channel: &str,
    execution_id: &str,
    payload: Value,
    payloads: &mut BTreeMap<String, Value>,
) -> Value {
    let button =
        fc::button_step_default_fn(&json!({ "label": truncate(label, button_label_max(channel)) }));
    let button_id = button["id"].as_str().unwrap_or_default().to_string();
    payloads.insert(
        button_id.clone(),
        json!({ "executionId": execution_id, "buttonId": button_id, "payload": payload }),
    );
    button
}

fn make_card(title: &str, subtitle: String, image_url: Option<&str>, buttons: Vec<Value>) -> Value {
    let mut card = json!({
        "id": create_id(),
        "stepType": "sendCard",
        "title": truncate(title, CARD_TEXT_MAX),
        "subtitle": subtitle,
        "buttons": buttons,
    });
    if let Some(url) = image_url.filter(|u| !u.is_empty()) {
        let mut image = fc::send_image_step_default_fn();
        image["mode"] = json!("url");
        image["url"] = json!(url);
        image["buttons"] = json!([]);
        card["image"] = image;
    }
    card
}

fn parsed(schema: fc::z::Schema, step: Value) -> Value {
    let result = schema.safe_parse(&step);
    if result.success {
        result.data.unwrap_or(step)
    } else {
        step
    }
}

fn make_carousel(cards: Vec<Value>) -> Value {
    let mut step = fc::send_carousel_step_default_fn();
    step["cards"] = Value::Array(cards);
    parsed(fc::send_carousel_step_schema(), step)
}

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
/// time zone (`Intl` medium date + short time; an invalid zone is `en` /
/// UTC), at most 80 characters.
pub fn format_poll_subtitle(
    state: PollState,
    starts_at: i64,
    ends_at: i64,
    language: &str,
    timezone: &str,
) -> String {
    let (format, tz) = match timezone.parse::<chrono_tz::Tz>() {
        Ok(tz) => (locale_format(language), tz),
        Err(_) => (&FORMATS[0], chrono_tz::UTC),
    };
    let label = state_label(state, get_poll_bot_strings(language));
    truncate(
        &format!(
            "{label} | {} - {}",
            format_compact(format, tz, starts_at),
            format_compact(format, tz, ends_at)
        ),
        CARD_TEXT_MAX,
    )
}

/// `buildPollCardsMessage`: one card per poll; `Vote` only on running polls.
#[allow(clippy::too_many_arguments)]
pub fn build_poll_cards_message(
    polls: &[PollCard],
    channel: &str,
    strings: &PollBotStrings,
    language: &str,
    timezone: &str,
    execution_id: &str,
    now: i64,
) -> PollChatMessage {
    let mut payloads = BTreeMap::new();
    let cards = polls
        .iter()
        .map(|poll| {
            let state = get_poll_state(poll.starts_at, poll.ends_at, now);
            let buttons = if state == PollState::Running {
                vec![make_button(
                    strings.vote,
                    channel,
                    execution_id,
                    json!({ "type": "poll_open", "pollId": poll.id }),
                    &mut payloads,
                )]
            } else {
                vec![]
            };
            make_card(
                &poll.name,
                format_poll_subtitle(state, poll.starts_at, poll.ends_at, language, timezone),
                Some(&poll.image_url),
                buttons,
            )
        })
        .collect();
    PollChatMessage {
        step: make_carousel(cards),
        button_payloads: payloads,
    }
}

/// `buildCandidatesMessage`: one voting card per candidate (the poll image
/// when the candidate has none).
pub fn build_candidates_message(
    poll_id: &str,
    poll_image_url: &str,
    candidates: &[CandidateCard],
    strings: &PollBotStrings,
    execution_id: &str,
) -> PollChatMessage {
    let mut payloads = BTreeMap::new();
    let cards = candidates
        .iter()
        .map(|candidate| {
            let button = make_button(
                strings.vote,
                "",
                execution_id,
                json!({ "type": "poll_vote", "pollId": poll_id, "candidateId": candidate.id }),
                &mut payloads,
            );
            let image = candidate
                .image_url
                .as_deref()
                .filter(|u| !u.is_empty())
                .unwrap_or(poll_image_url);
            make_card(
                &candidate.name,
                truncate(
                    candidate.description.as_deref().unwrap_or(""),
                    CARD_TEXT_MAX,
                ),
                Some(image),
                vec![button],
            )
        })
        .collect();
    PollChatMessage {
        step: make_carousel(cards),
        button_payloads: payloads,
    }
}

/// `buildVoteRegisteredMessage`: `sendQuickReply` where the channel fully
/// supports it, else `sendText`, with a `poll_list` button.
pub fn build_vote_registered_message(
    channel: &str,
    strings: &PollBotStrings,
    execution_id: &str,
) -> PollChatMessage {
    let mut payloads = BTreeMap::new();
    let button = make_button(
        strings.polls,
        channel,
        execution_id,
        json!({ "type": "poll_list" }),
        &mut payloads,
    );
    let quick_reply = fc::get_channel_flow_policy(Some(channel))
        .and_then(|p| p.steps.get("sendQuickReply").copied())
        == Some("full");
    let step = if quick_reply {
        parsed(
            fc::send_quick_reply_step_schema(),
            fc::send_quick_reply_step_default_fn(
                &json!({ "message": strings.vote_registered, "buttons": [button] }),
            ),
        )
    } else {
        parsed(
            fc::send_text_step_schema(),
            fc::send_text_step_default_fn(
                &json!({ "text": strings.vote_registered, "buttons": [button] }),
            ),
        )
    };
    PollChatMessage {
        step,
        button_payloads: payloads,
    }
}
