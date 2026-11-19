//! `packages/business/src/leaderboard` (Q.D13b): `config.ts`, `day.ts`,
//! `event-id.ts`, the masks of `ranking.ts` / `participants.ts` and the
//! platform tables of `database/partials/leaderboard.ts`. The service is
//! [`service`], the score writes are [`scoring`].
//!
//! The ranking is read from SQLite in rank order (`score DESC, scoredAt
//! ASC`); the Redis sorted set of the original is not ported (DECISIONS
//! 2026-11-19), so `rebuildRank` and the hourly ranking upkeep do nothing.

pub mod scoring;
pub mod service;

use chatbotx_db::repo::models::Leaderboard;
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};

use crate::errors::{BusinessError, Result};

/// `LEADERBOARD_MAX_SCORE`.
pub const MAX_SCORE: i64 = 89_999_999;
/// `LEADERBOARD_MAX_DURATION_SECONDS`.
pub const MAX_DURATION_SECONDS: i64 = 99_999_999;
/// `LEADERBOARD_LATE_EVENT_GRACE_MS`.
pub const LATE_EVENT_GRACE_MS: i64 = 10 * 60 * 1000;
/// `LEADERBOARD_PUBLIC_TOP_SIZE`.
pub const PUBLIC_TOP_SIZE: i64 = 50;
const DAY_MS: i64 = 24 * 60 * 60 * 1000;

/// `leaderboardStatuses`.
pub const STATUSES: &[&str] = &["draft", "active", "paused", "finished"];
/// `leaderboardPlatforms`.
pub const PLATFORMS: &[&str] = &[
    "messenger",
    "instagram",
    "whatsapp",
    "zalo",
    "telegram",
    "tiktok",
    "threads",
    "email",
    "gamification",
];
/// `leaderboardRuleKeys`.
pub const RULE_KEYS: &[&str] = &[
    "comments",
    "liveComments",
    "keywords",
    "storyMention",
    "storyReaction",
    "ctwaReactions",
    "survey",
    "follow",
    "message",
    "joinGroup",
    "groupDiscussion",
    "emailOpen",
    "emailClick",
    "minigameWin",
    "inGameShare",
    "referral",
];
/// `leaderboardAuditActions`.
pub const AUDIT_ACTIONS: &[&str] = &["adjust", "resetOne", "resetAll", "ban", "unban"];
/// `LEADERBOARD_FOLLOWER_CHECK_PLATFORMS`.
pub const FOLLOWER_CHECK_PLATFORMS: &[&str] = &["instagram", "zalo", "telegram"];
const POST_TRACKING_PLATFORMS: &[&str] = &["messenger", "instagram"];
const COMMENT_ONLY_FIELDS: &[&str] = &["minChars", "bonusPoints", "bonusMinutes"];

/// `LEADERBOARD_RULES_BY_PLATFORM`.
pub fn rules_by_platform(platform: &str) -> &'static [&'static str] {
    match platform {
        "messenger" => &["comments", "liveComments", "keywords", "referral"],
        "instagram" => &[
            "comments",
            "storyMention",
            "storyReaction",
            "keywords",
            "referral",
        ],
        "whatsapp" => &["ctwaReactions", "survey", "referral"],
        "zalo" => &["follow", "message", "referral"],
        "telegram" => &["joinGroup", "groupDiscussion", "referral"],
        "tiktok" => &["comments", "keywords"],
        "threads" => &["comments"],
        "email" => &["emailOpen", "emailClick"],
        "gamification" => &["minigameWin", "inGameShare", "referral"],
        _ => &[],
    }
}

/// `LEADERBOARD_PLATFORM_CHANNEL`: the inbox channel of a platform.
pub fn platform_channel(platform: &str) -> Option<&'static str> {
    Some(match platform {
        "messenger" => "messenger",
        "instagram" => "instagram",
        "whatsapp" => "whatsapp",
        "zalo" => "zalo",
        "telegram" => "telegram",
        "tiktok" => "tiktok",
        "threads" => "threads",
        "email" => "smtp",
        _ => return None,
    })
}

/// `LEADERBOARD_ACTIONS_BY_STATUS`.
pub fn actions_by_status(status: &str) -> &'static [&'static str] {
    match status {
        "draft" => &["edit", "publish", "delete"],
        "active" => &["pause", "edit", "resetPoints", "finish", "export"],
        "paused" => &["start", "edit", "export", "delete"],
        "finished" => &["export", "triggerRewards", "delete"],
        _ => &[],
    }
}

/// `assertActionAllowed`.
pub(crate) fn assert_action_allowed(status: &str, action: &str) -> Result<()> {
    if actions_by_status(status).contains(&action) {
        Ok(())
    } else {
        Err(BusinessError::exception(
            format!("Cannot {action} a {status} leaderboard"),
            "leaderboardActionNotAllowed",
            409,
        ))
    }
}

/// One parsed `leaderboardRuleSchema`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Rule {
    pub enabled: bool,
    pub points: i64,
    pub max_per_day: Option<i64>,
    pub min_chars: Option<i64>,
    pub bonus_points: Option<i64>,
    pub bonus_minutes: Option<i64>,
}

/// `leaderboard.rules[key]` (absent → `None`).
pub fn rule_of(rules: &Value, key: &str) -> Option<Rule> {
    let rule = rules.get(key).filter(|r| r.is_object())?;
    let int = |k: &str| rule.get(k).and_then(Value::as_i64);
    Some(Rule {
        enabled: rule
            .get("enabled")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        points: int("points").unwrap_or(0),
        max_per_day: int("maxPerDay"),
        min_chars: int("minChars"),
        bonus_points: int("bonusPoints"),
        bonus_minutes: int("bonusMinutes"),
    })
}

/// `leaderboardRuleSchema.parse({})` merged with `overrides`.
fn rule_json(enabled: bool, points: i64, extra: &[(&str, i64)]) -> Value {
    let mut rule = json!({
        "enabled": enabled,
        "points": points,
        "maxPerDay": null,
        "minChars": null,
        "bonusPoints": null,
        "bonusMinutes": null,
    });
    for (key, value) in extra {
        rule[*key] = json!(value);
    }
    rule
}

/// `defaultLeaderboardRules(platform)`.
pub fn default_rules(platform: &str) -> Value {
    let referral = || rule_json(true, 50, &[("maxPerDay", 10)]);
    let comments = |min| rule_json(true, 30, &[("minChars", min)]);
    let mut rules = Map::new();
    for key in rules_by_platform(platform) {
        let rule = match (platform, *key) {
            ("messenger", "comments") => {
                rule_json(true, 30, &[("minChars", 30), ("bonusMinutes", 60)])
            }
            ("messenger", "liveComments") => rule_json(true, 300, &[]),
            ("instagram" | "tiktok" | "threads", "comments") => comments(20),
            ("instagram", "storyMention") => rule_json(true, 100, &[("maxPerDay", 2)]),
            ("zalo", "message") | ("email", "emailClick") => {
                rule_json(false, 0, &[("maxPerDay", 2)])
            }
            (_, "referral") => referral(),
            _ => rule_json(false, 0, &[]),
        };
        rules.insert((*key).to_owned(), rule);
    }
    Value::Object(rules)
}

/// The create / update input (`LeaderboardUpsertInput`, already parsed).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct UpsertInput {
    pub name: String,
    pub description: String,
    pub cover_image_url: String,
    pub starts_at: i64,
    pub ends_at: i64,
    pub platform: String,
    pub inbox_id: Option<i64>,
    pub minigame_id: Option<i64>,
    /// `{ postScope, postIds, telegramChatIds }`.
    pub scope: Value,
    pub rules: Value,
    pub followers_only: bool,
    pub tag_ids: Vec<i64>,
    pub reset_time: String,
    pub cooldown_seconds: i64,
    pub referral_welcome_flow_id: Option<i64>,
    /// `{ flowId, topN, autoOnFinish }`.
    pub reward_settings: Value,
}

fn str_list(value: &Value) -> Vec<&str> {
    value
        .as_array()
        .map(|items| items.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default()
}

/// `assertValidLeaderboardConfig`.
pub fn assert_valid_config(input: &UpsertInput) -> Result<()> {
    if input.ends_at <= input.starts_at {
        return Err(BusinessError::validation(
            "endsAt",
            "End time must be after start time",
        ));
    }
    if (input.ends_at - input.starts_at) / 1000 > MAX_DURATION_SECONDS {
        return Err(BusinessError::validation(
            "endsAt",
            "A leaderboard can run for at most 3 years",
        ));
    }
    if input.platform == "gamification" {
        if input.minigame_id.is_none() {
            return Err(BusinessError::validation("minigameId", "Choose a minigame"));
        }
    } else if input.inbox_id.is_none() {
        return Err(BusinessError::validation("inboxId", "Choose a channel"));
    }
    let tracks_posts = POST_TRACKING_PLATFORMS.contains(&input.platform.as_str());
    let allowed = rules_by_platform(&input.platform);
    for (key, raw) in input.rules.as_object().into_iter().flatten() {
        if raw.is_null() {
            continue;
        }
        if !allowed.contains(&key.as_str()) {
            return Err(BusinessError::validation(
                &format!("rules.{key}"),
                format!("Rule \"{key}\" is not available on {}", input.platform),
            ));
        }
        let rule = rule_of(&input.rules, key).unwrap_or_default();
        if rule.enabled && rule.points <= 0 {
            return Err(BusinessError::validation(
                &format!("rules.{key}.points"),
                "Enter the points this rule awards",
            ));
        }
        if key != "comments" {
            for field in COMMENT_ONLY_FIELDS {
                if raw.get(*field).is_some_and(|v| !v.is_null()) {
                    return Err(BusinessError::validation(
                        &format!("rules.{key}.{field}"),
                        format!("\"{field}\" only applies to the comments rule"),
                    ));
                }
            }
            continue;
        }
        let bonus = rule.bonus_points.unwrap_or(0) > 0;
        if !tracks_posts && (bonus || rule.bonus_minutes.is_some_and(|m| m != 0)) {
            return Err(BusinessError::validation(
                "rules.comments.bonusPoints",
                format!(
                    "The golden-hour bonus is not available on {}",
                    input.platform
                ),
            ));
        }
        if bonus && rule.bonus_minutes.is_none_or(|m| m == 0) {
            return Err(BusinessError::validation(
                "rules.comments.bonusMinutes",
                "Set how many minutes the golden-hour bonus lasts",
            ));
        }
    }
    if input.scope["postScope"] == "specific" {
        if !tracks_posts {
            return Err(BusinessError::validation(
                "scope.postScope",
                format!(
                    "Scoring specific posts is not available on {}",
                    input.platform
                ),
            ));
        }
        if str_list(&input.scope["postIds"]).is_empty() {
            return Err(BusinessError::validation(
                "scope.postIds",
                "Choose at least one post",
            ));
        }
    }
    let enabled = |key| rule_of(&input.rules, key).is_some_and(|r| r.enabled);
    if input.platform == "telegram"
        && str_list(&input.scope["telegramChatIds"]).is_empty()
        && (enabled("joinGroup") || enabled("groupDiscussion"))
    {
        return Err(BusinessError::validation(
            "scope.telegramChatIds",
            "Choose at least one Telegram group or channel",
        ));
    }
    Ok(())
}

/// `isWithinLeaderboardWindow`.
pub fn is_within_window(leaderboard: &Leaderboard, at: i64) -> bool {
    let ends_at = leaderboard
        .finished_at
        .map_or(leaderboard.ends_at, |f| f.min(leaderboard.ends_at));
    at >= leaderboard.starts_at && at < ends_at
}

fn reset_offset_ms(reset_time: &str) -> i64 {
    let bytes = reset_time.as_bytes();
    let digits = |s: &[u8]| -> Option<i64> {
        s.iter()
            .all(u8::is_ascii_digit)
            .then(|| std::str::from_utf8(s).ok()?.parse().ok())?
    };
    if bytes.len() != 5 || bytes[2] != b':' {
        return 0;
    }
    match (digits(&bytes[..2]), digits(&bytes[3..])) {
        (Some(h), Some(m)) => (h * 60 + m) * 60 * 1000,
        _ => 0,
    }
}

/// `leaderboardDayStart`: the latest instant at or before `now` whose UTC
/// time is `resetTime`.
pub fn day_start(now: i64, reset_time: &str) -> i64 {
    let offset = reset_offset_ms(reset_time);
    (now - offset).div_euclid(DAY_MS) * DAY_MS + offset
}

/// `leaderboardDayKey` ("YYYY-MM-DD" of the leaderboard-local day).
pub fn day_key(now: i64, reset_time: &str) -> String {
    chatbotx_db::repo::iso(now - reset_offset_ms(reset_time))[..10].to_owned()
}

/// `deterministicLeaderboardEventId`: a positive 63-bit id of the parts.
pub fn event_id(parts: &[&str]) -> i64 {
    let digest = Sha256::digest(parts.join("\u{0}").as_bytes());
    let mut first = [0u8; 8];
    first.copy_from_slice(&digest[..8]);
    let value = (u64::from_be_bytes(first) & (u64::MAX >> 1)) as i64;
    if value == 0 { 1 } else { value }
}

fn first_char(word: &str) -> String {
    // `word.slice(0, 1)`: one UTF-16 unit; a BMP character in practice.
    word.chars().next().map(String::from).unwrap_or_default()
}

/// `maskParticipantName`: "Nguyễn Văn An" → "Nguyễn V*** An".
pub fn mask_name(full_name: Option<&str>) -> String {
    let words: Vec<&str> = full_name.unwrap_or("").split_whitespace().collect();
    match words.as_slice() {
        [] => "***".to_owned(),
        [word] => format!("{}***", first_char(word)),
        [first, middle @ .., last] => {
            let mut out = vec![(*first).to_owned()];
            if middle.is_empty() {
                out.push("***".to_owned());
            }
            out.extend(middle.iter().map(|w| format!("{}***", first_char(w))));
            out.push((*last).to_owned());
            out.join(" ")
        }
    }
}

/// `maskParticipantPhone`: only the last 3 digits.
pub fn mask_phone(phone: Option<&str>) -> Option<String> {
    let digits: String = phone?.chars().filter(char::is_ascii_digit).collect();
    if phone.is_some_and(str::is_empty) {
        return None;
    }
    let n = digits.chars().count();
    if n <= 3 {
        return Some("•".repeat(n));
    }
    Some(format!("{}{}", "•".repeat(n - 3), &digits[n - 3..]))
}

/// The `{ participants, totalPoints, newToday }` counters.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Stats {
    pub participants: i64,
    pub total_points: i64,
    pub new_today: i64,
}

impl Stats {
    pub fn to_json(self) -> Value {
        json!({
            "participants": self.participants,
            "totalPoints": self.total_points,
            "newToday": self.new_today,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn day_window_follows_the_reset_time() {
        // 2026-01-02T21:59:00Z with a 22:00 reset is still 2026-01-01's day.
        let now = 1_767_391_140_000;
        assert_eq!(day_key(now, "22:00"), "2026-01-01");
        assert_eq!(day_start(now, "22:00"), 1_767_304_800_000);
        assert_eq!(day_key(now + 60_000, "22:00"), "2026-01-02");
        assert_eq!(day_key(now, "bad"), "2026-01-02");
    }

    #[test]
    fn masks() {
        assert_eq!(mask_name(Some("Nguyễn Văn An")), "Nguyễn V*** An");
        assert_eq!(mask_name(Some("Ann Lee")), "Ann *** Lee");
        assert_eq!(mask_name(Some("Ann")), "A***");
        assert_eq!(mask_name(None), "***");
        assert_eq!(mask_phone(Some("+84 912-345-678")).unwrap(), "••••••••678");
        assert_eq!(mask_phone(Some("12")).unwrap(), "••");
        assert_eq!(mask_phone(None), None);
    }

    #[test]
    fn event_ids_are_stable_and_positive() {
        let a = event_id(&["1", "comments", "c1"]);
        assert_eq!(a, event_id(&["1", "comments", "c1"]));
        assert_ne!(a, event_id(&["1", "comments", "c2"]));
        assert!(a > 0);
    }

    fn config(platform: &str, rules: Value) -> UpsertInput {
        UpsertInput {
            name: "Board".into(),
            starts_at: 0,
            ends_at: 1000,
            platform: platform.into(),
            inbox_id: Some(1),
            minigame_id: Some(1),
            scope: json!({ "postScope": "all", "postIds": [], "telegramChatIds": [] }),
            rules,
            reset_time: "22:00".into(),
            reward_settings: json!({ "flowId": null, "topN": 3, "autoOnFinish": false }),
            ..UpsertInput::default()
        }
    }

    fn message(input: &UpsertInput) -> String {
        assert_valid_config(input).unwrap_err().message().to_owned()
    }

    #[test]
    fn config_rules() {
        for platform in PLATFORMS {
            assert!(assert_valid_config(&config(platform, default_rules(platform))).is_ok());
        }
        let mut c = config("messenger", json!({ "follow": rule_json(true, 1, &[]) }));
        assert_eq!(message(&c), "Rule \"follow\" is not available on messenger");
        c.rules = json!({ "comments": rule_json(true, 0, &[]) });
        assert_eq!(message(&c), "Enter the points this rule awards");
        c.rules = json!({ "referral": rule_json(true, 5, &[("minChars", 3)]) });
        assert_eq!(
            message(&c),
            "\"minChars\" only applies to the comments rule"
        );
        c.rules = json!({ "comments": rule_json(true, 5, &[("bonusPoints", 3)]) });
        assert_eq!(
            message(&c),
            "Set how many minutes the golden-hour bonus lasts"
        );
        let mut t = config(
            "tiktok",
            json!({ "comments": rule_json(true, 5, &[("bonusMinutes", 3)]) }),
        );
        assert_eq!(
            message(&t),
            "The golden-hour bonus is not available on tiktok"
        );
        t.rules = json!({});
        t.scope["postScope"] = json!("specific");
        assert_eq!(
            message(&t),
            "Scoring specific posts is not available on tiktok"
        );
        let tg = config("telegram", default_rules("telegram"));
        let mut tg2 = tg.clone();
        tg2.rules["joinGroup"] = rule_json(true, 5, &[]);
        assert_eq!(
            message(&tg2),
            "Choose at least one Telegram group or channel"
        );
        let mut late = config("zalo", json!({}));
        late.ends_at = 0;
        assert_eq!(message(&late), "End time must be after start time");
        late.ends_at = (MAX_DURATION_SECONDS + 1) * 1000;
        assert_eq!(message(&late), "A leaderboard can run for at most 3 years");
    }
}
