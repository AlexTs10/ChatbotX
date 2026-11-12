//! `systemFieldTypes`, `contactSources` and the small business helpers the
//! system fields use: `@chatbotx.io/business/contact-locale`
//! (`normalizeStoredTimezone`, `offsetFromStoredTimezone`,
//! `languageFromLocale`), `resolveGenderLabel` and
//! `isWorkspaceScheduledForDeletion`.

use std::sync::OnceLock;

use regex::Regex;

use super::models::WorkspaceModel;
use crate::js;
use crate::utils::tz::Zone;

string_enum! {
    /// `systemFieldTypes`.
    pub enum SystemFieldType {
        FirstName = "first_name",
        LastName = "last_name",
        FullName = "full_name",
        Email = "email",
        Phone = "phone",
        UserCountry = "user_country",
        UserState = "user_state",
        UserCity = "user_city",
        InboxLink = "inbox_link",
        Gender = "gender",
        Locale = "locale",
        Language = "language",
        Locale2 = "locale2",
        IgUserName = "ig_user_name",
        IgFollowers = "ig_followers",
        IgVerified = "ig_verified",
        IgFollowBusiness = "ig_follow_business",
        IgBusinessFollowUser = "ig_business_follow_user",
        ProfilePic = "profile_pic",
        Timezone = "timezone",
        TimezoneName = "timezone_name",
        UserId = "user_id",
        SubscribedDate = "subscribed_date",
        FbChatLink = "fb_chat_link",
        Me = "me",
        UserCode = "user_code",
        LastBtnTitle = "last_btn_title",
        LastBttTitle = "last_btt_title",
        Empty = "empty",
        LastInteraction = "last_interaction",
        LastOrder = "last_order",
        LastOpportunityId = "last_opportunity_id",
        LastSeen = "last_seen",
        LastInput = "last_input",
        LastInputType = "last_input_type",
        LastTextInput = "last_text_input",
        ConsecutiveFailedReply = "consecutive_failed_reply",
        LastRef = "last_ref",
        UserChannel = "user_channel",
        UserHash = "user_hash",
        MinigamePlayToken = "minigame_play_token",
        MiniAppToken = "mini_app_token",
        ShopToken = "shop_token",
        LeaderboardViewToken = "leaderboard_view_token",
        ReferralName = "referral_name",
        UserTags = "user_tags",
        UserExternalId = "user_external_id",
        UserSource = "user_source",
        AssignedAdminName = "assigned_admin_name",
        AssignedAdminEmail = "assigned_admin_email",
        AssignedAdminId = "assigned_admin_id",
        CurrentUserTime = "current_user_time",
        ChatHistory = "chat_history",
        ChatHistoryLarge = "chat_history_large",
        ChatHistoryDetails = "chat_history_details",
        ChatHistoryDetailsLarge = "chat_history_details_large",
        AiQueuedMessages = "ai.queued.messages",
        UserNotes = "user_notes",
        LastUserNote = "last_user_note",
        Webchat = "webchat",
        WebchatParentUrl = "webchat_parent_url",
        Avatar = "avatar",
        AccountId = "account_id",
        AccountName = "account_name",
        AccountImage = "account_image",
        ApiKey = "api_key",
        PageApiKey = "page_api_key",
        LastAd = "last_ad",
        LastCtwa = "last_ctwa",
        AdIds = "ad_ids",
        AdId = "ad_id",
        LastAdSourceUrl = "last_ad_source_url",
        LastAdSourcePlatform = "last_ad_source_platform",
        LastAdHeadline = "last_ad_headline",
        LastAdBody = "last_ad_body",
        LastFbComment = "last_fb_comment",
        LastPostId = "last_post_id",
        LastCommentId = "last_comment_id",
        LastStoryId = "last_story_id",
        TotalNewTagged = "total_new_tagged",
        TotalTagged = "total_tagged",
        LastLatitude = "last_latitude",
        LastLongitude = "last_longitude",
        LastErrorLog = "last_error_log",
        LastOutboundMessageAt = "last_outbound_message_at",
        LastCommentedPostText = "last_commented_post_text",
        LastCallRecorded = "last_call_recorded",
        LastCallTranscript = "last_call_transcript",
        LastStep = "last_step",
        CurrentStep = "current_step",
        BookingId = "booking_id",
        BookingCalendar = "booking_calendar",
        BookingDate = "booking_date",
        BookingLink = "booking_link",
        MemberName = "member_name",
        TeamName = "team_name",
        AdminName = "admin_name",
        AdminTeamName = "admin_team_name",
        LastInputFailure = "last_input_failure",
        LastPoints = "last_points",
        LastLeadId = "last_lead_id",
        LastFormId = "last_form_id",
        LastFormName = "last_form_name",
        LastAdgroupId = "last_adgroup_id",
        WorkspaceId = "workspace_id",
        WorkspaceName = "workspace_name",
        CurrentTime = "current_time",
        PageUserName = "page_user_name",
        WaUserId = "wa_user_id",
        WaUserName = "wa_user_name",    }
}

string_enum! {
    /// `contactSources`.
    pub enum ContactSource {
        InboundMessage = "inboundMessage",
        Webchat = "webchat",
        Ads = "ads",
        BotLink = "botLink",
        ChatPlugin = "chatPlugin",
        Comments = "comments",
        LiveComments = "liveComments",
        Imported = "imported",
        Api = "api",
        Direct = "direct",
        InboundWebhook = "inboundWebhook",
    }
}

pub use crate::utils::channel::ChannelType;

/// `CONTACT_AD_IDS_LIMIT`.
pub const CONTACT_AD_IDS_LIMIT: usize = 50;

/// `profileSnapshotChannels`.
pub const PROFILE_SNAPSHOT_CHANNELS: &[&str] = &["instagram"];

/// `supportsProfileSnapshot`.
pub fn supports_profile_snapshot(channel: &str) -> bool {
    PROFILE_SNAPSHOT_CHANNELS.contains(&channel)
}

/// `offsetToTimezoneMap`.
const OFFSET_TO_TIMEZONE: &[(&str, &str)] = &[
    ("-12", "Etc/GMT+12"),
    ("-11", "Pacific/Pago_Pago"),
    ("-10", "Pacific/Honolulu"),
    ("-9", "America/Anchorage"),
    ("-8", "America/Los_Angeles"),
    ("-7", "America/Denver"),
    ("-6", "America/Chicago"),
    ("-5", "America/New_York"),
    ("-4", "America/Halifax"),
    ("-3", "America/Sao_Paulo"),
    ("-2", "Atlantic/South_Georgia"),
    ("-1", "Atlantic/Azores"),
    ("0", "UTC"),
    ("1", "Europe/Berlin"),
    ("2", "Europe/Athens"),
    ("3", "Europe/Moscow"),
    ("4", "Asia/Dubai"),
    ("5", "Asia/Karachi"),
    ("6", "Asia/Dhaka"),
    ("7", "Asia/Bangkok"),
    ("8", "Asia/Singapore"),
    ("9", "Asia/Tokyo"),
    ("10", "Australia/Sydney"),
    ("11", "Pacific/Noumea"),
    ("12", "Pacific/Auckland"),
    ("13", "Pacific/Tongatapu"),
    ("14", "Pacific/Kiritimati"),
];

fn offset_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^([+-]?)(\d{1,2})(?::?([0-5]\d))?$").expect("valid regex"))
}

/// `normalizeOffsetKey`: `"+07:00"` → `"7"`, `"-5"` → `"-5"`,
/// `"5:30"` → `"5.5"`; `None` when the value is not an offset.
fn normalize_offset_key(value: &str) -> Option<String> {
    let caps = offset_re().captures(js::trim(value))?;
    let sign = if &caps[1] == "-" { -1.0 } else { 1.0 };
    let hours: f64 = caps[2].parse().ok()?;
    let minutes: f64 = caps.get(3).map_or(Some(0.0), |m| m.as_str().parse().ok())?;
    Some(js::number_to_string(sign * (hours + minutes / 60.0)))
}

fn non_blank(value: Option<&str>) -> Option<&str> {
    value.map(js::trim).filter(|v| !v.is_empty())
}

/// `normalizeStoredTimezone`: a stored offset (`"7"`, `"+07:00"`) maps to
/// its IANA zone; anything else is returned trimmed.
pub fn normalize_stored_timezone(value: Option<&str>) -> Option<String> {
    let trimmed = non_blank(value)?;
    match normalize_offset_key(trimmed) {
        Some(key) => Some(
            OFFSET_TO_TIMEZONE
                .iter()
                .find(|(k, _)| *k == key)
                .map_or(trimmed, |(_, zone)| zone)
                .to_owned(),
        ),
        None => Some(trimmed.to_owned()),
    }
}

/// `offsetFromStoredTimezone`: the signed hour offset (`"+7"`, `"-5"`,
/// `"0"`, `"+5.5"`) of a stored offset or of an IANA zone at `now_ms`.
pub fn offset_from_stored_timezone(value: Option<&str>, now_ms: i64) -> Option<String> {
    let trimmed = non_blank(value)?;
    let signed = |hours: f64, text: String| {
        if hours > 0.0 {
            format!("+{text}")
        } else {
            text
        }
    };
    if let Some(key) = normalize_offset_key(trimmed) {
        let hours = js::to_number(&key);
        return Some(signed(hours, key));
    }
    let zone = Zone::parse(trimmed)?;
    let hours = zone.offset_ms(now_ms) as f64 / 3_600_000.0;
    Some(signed(hours, js::number_to_string(hours)))
}

fn locale_separator_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new("[-_]").expect("valid regex"))
}

/// `normalizeLocale`: `"vi-vn"` → `"vi_VN"`.
pub fn normalize_locale(raw: Option<&str>) -> Option<String> {
    let trimmed = non_blank(raw)?;
    let parts: Vec<&str> = locale_separator_re().split(trimmed).collect();
    match parts.as_slice() {
        [language, region, rest @ ..] if !language.is_empty() && !region.is_empty() => {
            let mut out = vec![js::to_lower(language), js::to_upper(region)];
            out.extend(rest.iter().map(|part| js::to_upper(part)));
            Some(out.join("_"))
        }
        _ => Some(js::to_lower(trimmed)),
    }
}

/// `languageFromLocale`: `"vi-VN"` → `"vi"`.
pub fn language_from_locale(locale: Option<&str>) -> Option<String> {
    let normalized = normalize_locale(locale)?;
    let language = locale_separator_re()
        .split(&normalized)
        .next()
        .map(js::to_lower)?;
    (!language.is_empty()).then_some(language)
}

/// `GENDER_LABELS`: `(language, female, male, unknown)`.
const GENDER_LABELS: &[(&str, &str, &str, &str)] = &[
    ("en", "Female", "Male", "Male/Female"),
    ("vi", "Chị", "Anh", "Anh/Chị"),
];

/// `resolveGenderLabel`: the salutation for `gender` in the workspace
/// language (primary subtag; English when there is no label set).
pub fn resolve_gender_label(language: Option<&str>, gender: Option<&str>) -> String {
    let language = js::to_lower(language.filter(|l| !l.is_empty()).unwrap_or("en"));
    let subtag = locale_separator_re().split(&language).next().unwrap_or("");
    let labels = GENDER_LABELS
        .iter()
        .find(|(lang, ..)| *lang == subtag)
        .unwrap_or(&GENDER_LABELS[0]);
    match gender.map(js::to_lower).as_deref() {
        Some("male") => labels.2,
        Some("female") => labels.1,
        _ => labels.3,
    }
    .to_owned()
}

/// `isWorkspaceScheduledForDeletion`.
pub fn is_workspace_scheduled_for_deletion(workspace: &WorkspaceModel) -> bool {
    workspace.scheduled_deletion_at.is_some()
}
