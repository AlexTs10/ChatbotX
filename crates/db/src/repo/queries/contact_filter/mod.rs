//! Port of `queries/contact-filter/` (P3.2b): a saved contact filter (`ContactFilterCriteria`)
//! as one SQL predicate over the `"Contact"` table.
//!
//! The TS built a Drizzle relational `where` object (`ContactWhere`, `{}` = no predicate);
//! here the result is `Option<Sql>`: `None` = `{}`, `Some` = a boolean SQL expression that
//! reads the outer `"Contact"` row (`"Contact"."<column>"`). Relation conditions are
//! correlated `EXISTS (SELECT 1 FROM "<Table>" WHERE "<Table>"."contactId" = "Contact"."id"
//! AND ..)` sub-selects, as the TS `RAW` filters.
//!
//! SQLite forms: `ILIKE` is the app function `ilike(text, pattern)` (`\` escape), `~` is
//! the app `regexp`, timestamps are integer unix ms (a filter value window is computed in
//! Rust and bound as two integers), `NOW()` is the bound `now` (unix ms; P3.2 rule).
//! Ids are bound as integers when they are integer literals, else as text (a non-numeric
//! id then matches no row; Postgres threw on the bigint cast).

use std::sync::LazyLock;

use chrono_tz::Tz;
use regex::Regex;
use rusqlite::types::Value;
use serde::{Deserialize, Serialize};
use serde_json::Value as Json;

use self::predicates as p;
use self::sqlx::{
    contact_col, contact_inbox_exists, conversation_exists, exists, id_value, in_list, join_exists,
    join_raw, non_empty_str, string_values,
};
use super::ad_referral::any_channel_ad_conversation_predicate;
use super::ctwa_retarget::{
    CTWA_RETARGET_SEGMENTS, CtwaSegmentPredicateInput, build_ctwa_segment_contact_exists,
    ctwa_retarget_date_range,
};
use crate::repo::sql::Sql;
use crate::repo::utils::{escape_like_pattern, is_bigint_id, like_contains};

mod continents;
mod field_value;
pub mod permission;
mod predicates;
mod relation_sets;
mod sqlx;
pub mod value_format;

pub use permission::{
    EMAIL_PHONE_FILTER_FIELDS, contact_filter_uses_field, prune_contact_filter_fields,
    prune_email_phone_filter_conditions,
};

/// `ContactFilterCriteriaInput.operator`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CriteriaOperator {
    #[default]
    And,
    Or,
}

/// `ContactFilterCriteriaInput`. `conditions` stay raw JSON (`unknown[]` in the TS); each
/// one is read as a [`FilterCondition`] when the filter is built.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FilterCriteria {
    pub operator: CriteriaOperator,
    #[serde(default)]
    pub conditions: Vec<Json>,
    /// IANA zone of naive date / datetime values (default UTC).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timezone: Option<String>,
    /// IANA zone of `isWithinWorkingHours` (default UTC).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub working_hours_timezone: Option<String>,
}

/// `ContactFilterConditionInput`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct FilterCondition {
    pub field: String,
    pub operator: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<Json>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub custom_field_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub custom_field_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bot_field_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bot_field_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub topic_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub leaderboard_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attribute: Option<String>,
    /// `string | null | undefined`: JSON `null` = any pipeline.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pipeline_id: Option<Json>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub segment: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ad_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub integration_whatsapp_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub channel: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub since: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub until: Option<String>,
}

/// `ContactWhereInput` (+ `now`, unix ms).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ContactWhereInput {
    pub workspace_id: i64,
    pub keyword: Option<String>,
    pub contact_filter: Option<FilterCriteria>,
    pub now: i64,
}

/// `SmartKeywordOptions` (both default to `true`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SmartKeywordOptions {
    pub include_email_and_phone: bool,
    pub include_source_id: bool,
}

impl Default for SmartKeywordOptions {
    fn default() -> Self {
        SmartKeywordOptions {
            include_email_and_phone: true,
            include_source_id: true,
        }
    }
}

/// `ConversationAssigneeSelection`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ConversationAssigneeSelection {
    pub has_unassigned: bool,
    pub user_ids: Vec<String>,
    pub inbox_team_ids: Vec<String>,
}

/// `parseConversationAssigneeValues`: `unassigned`, `u_<id>`, `t_<id>` (ids must be
/// bigint literals).
pub fn parse_conversation_assignee_values(values: &[&str]) -> ConversationAssigneeSelection {
    let mut out = ConversationAssigneeSelection::default();
    for v in values {
        if *v == "unassigned" {
            out.has_unassigned = true;
        } else if let Some(id) = v.strip_prefix("u_") {
            if is_bigint_id(id) {
                out.user_ids.push(id.to_owned());
            }
        } else if let Some(id) = v.strip_prefix("t_")
            && is_bigint_id(id)
        {
            out.inbox_team_ids.push(id.to_owned());
        }
    }
    out
}

/// The context of one filter build.
struct Ctx {
    zone: Tz,
    working_hours_zone: Tz,
    workspace_id: Option<i64>,
    now: i64,
}

/// `applyContactFilter`: the AND / OR of the conditions that compile; `None` = `{}`.
pub fn apply_contact_filter(
    criteria: &FilterCriteria,
    workspace_id: Option<i64>,
    now: i64,
) -> Option<Sql> {
    if criteria.conditions.is_empty() {
        return None;
    }
    let ctx = Ctx {
        zone: value_format::resolve_zone(criteria.timezone.as_deref()),
        working_hours_zone: value_format::resolve_zone(criteria.working_hours_timezone.as_deref()),
        workspace_id,
        now,
    };
    let parts: Vec<Sql> = criteria
        .conditions
        .iter()
        .filter_map(|c| serde_json::from_value::<FilterCondition>(c.clone()).ok())
        .filter_map(|c| condition_where(&c, &ctx))
        .collect();
    if parts.is_empty() {
        return None;
    }
    let parts = parts.into_iter().map(Some);
    Some(match criteria.operator {
        CriteriaOperator::Or => Sql::or(parts),
        CriteriaOperator::And => Sql::and(parts),
    })
}

/// `contactFilterHasPredicate`.
pub fn contact_filter_has_predicate(criteria: &FilterCriteria, workspace_id: Option<i64>) -> bool {
    apply_contact_filter(criteria, workspace_id, 0).is_some()
}

static PHONE_KEYWORD: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[+0-9\s\-()]+$").expect("regex"));

/// `buildSmartKeywordWhere`; `None` for a blank keyword.
pub fn build_smart_keyword_where(keyword: &str, options: SmartKeywordOptions) -> Option<Sql> {
    let trimmed = keyword.trim();
    if trimmed.is_empty() {
        return None;
    }
    let like = like_contains(&trimmed.to_lowercase());
    let ilike = |column: &str, pattern: &str| {
        Some(
            Sql::bind(
                format!("ilike({}, ", contact_col(column)),
                pattern.to_owned(),
            )
            .closed(")"),
        )
    };
    let source_id = || {
        options.include_source_id.then(|| {
            contact_inbox_exists(
                Some(Sql::bind(
                    "\"ContactInbox\".\"sourceId\" = ",
                    trimmed.to_owned(),
                )),
                false,
            )
        })
    };
    let names_and_source_id = || {
        Sql::or([
            ilike("firstName", &like),
            ilike("lastName", &like),
            source_id(),
        ])
    };
    if trimmed.contains('@') {
        return Some(if options.include_email_and_phone {
            ilike("email", &like).unwrap_or_default()
        } else {
            names_and_source_id()
        });
    }
    if trimmed.bytes().all(|b| b.is_ascii_digit()) {
        let id =
            is_bigint_id(trimmed).then(|| Sql::bind("\"Contact\".\"id\" = ", id_value(trimmed)));
        let email_phone = options.include_email_and_phone;
        return Some(Sql::or([
            id,
            ilike("firstName", &like),
            ilike("lastName", &like),
            if email_phone {
                ilike("email", &like)
            } else {
                None
            },
            if email_phone {
                ilike("phoneNumber", &like)
            } else {
                None
            },
            source_id(),
        ]));
    }
    let digits = trimmed.chars().filter(char::is_ascii_digit).count();
    if PHONE_KEYWORD.is_match(trimmed) && digits >= 6 {
        if !options.include_email_and_phone {
            return Some(names_and_source_id());
        }
        let phone: String = trimmed
            .chars()
            .filter(|c| !(c.is_whitespace() || matches!(c, '-' | '(' | ')')))
            .collect();
        return ilike("phoneNumber", &like_contains(&phone));
    }
    Some(names_and_source_id())
}

/// `buildContactWhere`: the workspace scope AND the keyword AND the filter.
pub fn build_contact_where(input: &ContactWhereInput) -> Sql {
    let workspace = Sql::bind("\"Contact\".\"workspaceId\" = ", input.workspace_id);
    let filters: Vec<Sql> = [
        input
            .keyword
            .as_deref()
            .filter(|k| !k.is_empty())
            .and_then(|k| build_smart_keyword_where(k, SmartKeywordOptions::default())),
        input
            .contact_filter
            .as_ref()
            .and_then(|f| apply_contact_filter(f, Some(input.workspace_id), input.now)),
    ]
    .into_iter()
    .flatten()
    .collect();
    match filters.len() {
        0 => Sql::and([Some(workspace)]),
        1 => Sql::and([Some(workspace), filters.into_iter().next()]),
        _ => Sql::and([
            Some(workspace),
            Some(Sql::and(filters.into_iter().map(Some))),
        ]),
    }
}

/// `resolveContactFilterPredicate`: `Err(true)` = every contact, `Err(false)` = nobody.
fn resolve_predicate(workspace_id: i64, criteria: &FilterCriteria, now: i64) -> Result<Sql, bool> {
    if criteria.conditions.is_empty() {
        return Err(true);
    }
    if !contact_filter_has_predicate(criteria, Some(workspace_id)) {
        return Err(false);
    }
    Ok(build_contact_where(&ContactWhereInput {
        workspace_id,
        keyword: None,
        contact_filter: Some(criteria.clone()),
        now,
    }))
}

fn constant(b: bool) -> Sql {
    Sql::raw(if b { "TRUE" } else { "FALSE" })
}

/// `buildContactFilterMatchSQL`: `TRUE` (no conditions), `FALSE` (nothing compiles) or
/// the predicate (with the workspace scope).
pub fn build_contact_filter_match_sql(
    workspace_id: i64,
    criteria: &FilterCriteria,
    now: i64,
) -> Sql {
    resolve_predicate(workspace_id, criteria, now).unwrap_or_else(constant)
}

/// `buildContactInboxContactFilterSQL`: `<contact_id_column> IN (SELECT "Contact"."id"
/// FROM "Contact" WHERE ..)`, or `TRUE` / `FALSE`.
pub fn build_contact_inbox_contact_filter_sql(
    contact_id_column: &str,
    workspace_id: i64,
    criteria: &FilterCriteria,
    now: i64,
) -> Sql {
    match resolve_predicate(workspace_id, criteria, now) {
        Err(b) => constant(b),
        Ok(p) => {
            let mut out = Sql::raw(format!(
                "{contact_id_column} IN (SELECT \"Contact\".\"id\" FROM \"Contact\" WHERE "
            ));
            out.append(p);
            out.closed(")")
        }
    }
}

/// `contactInboxHasDmSQL`: the `"ContactInbox"` row has a DM conversation (no `sourceId`).
pub fn contact_inbox_has_dm_sql() -> Sql {
    Sql::raw(
        "EXISTS (SELECT 1 FROM \"Conversation\" WHERE \"Conversation\".\"contactId\" = \"ContactInbox\".\"contactId\" AND \"Conversation\".\"sourceId\" IS NULL)",
    )
}

fn questionnaire_exists(predicate: Option<Sql>, negate: bool) -> Sql {
    join_exists("QuestionnaireSubmission", "contactId", predicate, negate)
}

fn questionnaire_predicate(status: Option<&str>) -> Sql {
    let mut text = String::new();
    if let Some(s) = status {
        text.push_str(&format!(
            "\"QuestionnaireSubmission\".\"status\" = '{s}' AND "
        ));
    }
    text.push_str("\"QuestionnaireSubmission\".\"questionnaireId\" IN (SELECT \"Questionnaire\".\"id\" FROM \"Questionnaire\" WHERE \"Questionnaire\".\"workspaceId\" = \"QuestionnaireSubmission\".\"workspaceId\")");
    Sql::raw(text)
}

fn conversation_assigned_where(operator: &str, value: Option<&Json>) -> Option<Sql> {
    if operator == "isEmpty" {
        return Some(conversation_exists(
            Some(Sql::raw(
                "(\"Conversation\".\"assignedUserId\" IS NOT NULL OR \"Conversation\".\"assignedInboxTeamId\" IS NOT NULL)",
            )),
            true,
        ));
    }
    let negative = matches!(operator, "ne" | "notIn");
    if !(negative || matches!(operator, "eq" | "in")) {
        return None;
    }
    let values = string_values(value);
    let refs: Vec<&str> = values.iter().map(String::as_str).collect();
    let sel = parse_conversation_assignee_values(&refs);
    let ids = |v: &[String]| v.iter().map(|i| id_value(i)).collect::<Vec<Value>>();
    let mut parts = Vec::new();
    if !sel.user_ids.is_empty() {
        parts.push(in_list(
            "\"Conversation\".\"assignedUserId\"",
            ids(&sel.user_ids),
            false,
        ));
    }
    if !sel.inbox_team_ids.is_empty() {
        parts.push(in_list(
            "\"Conversation\".\"assignedInboxTeamId\"",
            ids(&sel.inbox_team_ids),
            false,
        ));
    }
    if sel.has_unassigned {
        parts.push(Sql::raw(
            "\"Conversation\".\"assignedUserId\" IS NULL AND \"Conversation\".\"assignedInboxTeamId\" IS NULL",
        ));
    }
    join_raw(parts, " OR ").map(|p| conversation_exists(Some(p), negative))
}

fn coupon_topic_where(topic_id: Option<&str>, operator: &str, value: Option<&Json>) -> Option<Sql> {
    let topic = topic_id.filter(|t| !t.is_empty())?;
    let mut predicate = Sql::bind(
        "\"Coupon\".\"workspaceId\" = \"Contact\".\"workspaceId\" AND \"Coupon\".\"topicId\" = ",
        id_value(topic),
    );
    match operator {
        "isNotEmpty" => {}
        "used" => {
            predicate.push(" AND \"Coupon\".\"usedAt\" IS NOT NULL");
        }
        "eq" => {
            let code = non_empty_str(value)?;
            predicate.push(" AND ilike(\"Coupon\".\"code\", ");
            predicate.push_bind(escape_like_pattern(code));
            predicate.push(")");
        }
        _ => return None,
    }
    Some(join_exists(
        "Coupon",
        "issuedContactId",
        Some(predicate),
        false,
    ))
}

fn ctwa_retarget_where(c: &FilterCondition, ctx: &Ctx) -> Option<Sql> {
    let segment = c
        .segment
        .as_deref()
        .filter(|s| CTWA_RETARGET_SEGMENTS.contains(s))?;
    let (since, until) = (c.since.as_deref()?, c.until.as_deref()?);
    if let Some(ch) = c.channel.as_deref()
        && !matches!(ch, "whatsapp" | "messenger" | "instagram")
    {
        return None;
    }
    let (since, until) = ctwa_retarget_date_range(since, until)?;
    let integration_whatsapp_id = match c.integration_whatsapp_id.as_deref() {
        None => None,
        Some(id) => Some(id.parse::<i64>().ok()?),
    };
    let input = CtwaSegmentPredicateInput {
        segment: segment.to_owned(),
        ad_id: c.ad_id.clone(),
        since,
        until,
        workspace_id: ctx.workspace_id,
        integration_whatsapp_id,
        channel: c.channel.clone(),
        ..CtwaSegmentPredicateInput::default()
    };
    Some(exists(
        build_ctwa_segment_contact_exists(&input, "\"Contact\".\"id\""),
        false,
    ))
}

fn commented_on_post_where(
    operator: &str,
    value: Option<&Json>,
    workspace_id: Option<i64>,
) -> Option<Sql> {
    let _ = (operator, value, workspace_id);
    todo!("P3.2b")
}

fn leaderboard_score_where(
    leaderboard_id: Option<&str>,
    operator: &str,
    value: Option<&Json>,
) -> Option<Sql> {
    let _ = (leaderboard_id, operator, value);
    todo!("P3.2b")
}

fn working_hours_where(operator: &str, value: Option<&Json>, ctx: &Ctx) -> Option<Sql> {
    let _ = (operator, value, ctx.working_hours_zone);
    todo!("P3.2b")
}

const GOOGLE_CLICK: &str = "(\"ContactInbox\".\"referral\"->>'gclid' IS NOT NULL OR \"ContactInbox\".\"referral\"->>'gbraid' IS NOT NULL)";

/// `buildConditionWhere`. Fields of the later slices (Instagram comments, activity,
/// commerce, pipeline items, leaderboards, working hours) compile to `None` until ported.
fn condition_where(c: &FilterCondition, ctx: &Ctx) -> Option<Sql> {
    let op = c.operator.as_str();
    let v = c.value.as_ref();
    let zone = ctx.zone;
    let now = ctx.now;
    let ci = contact_inbox_exists;
    match c.field.as_str() {
        "fullName" | "email" => p::column_where(&c.field, op, v, true),
        "continent" => p::continent_where(op, v),
        "phone" => p::column_where("phoneNumber", op, v, true),
        "lastComment" => p::last_comment_where(op, v),
        "gender" | "country" | "locale" | "timezone" => p::column_where(&c.field, op, v, false),
        "contactCreatedAt" => p::date_column_where("createdAt", op, v, zone),
        "contactCreatedDateMinutesAgo" => p::minutes_ago_where("createdAt", op, v, now),
        "lastSeen" => p::latest_date_where("contactLastReadAt", op, v, zone),
        "lastSeenMinutesAgo" => p::latest_minutes_ago_where("contactLastReadAt", op, v, now),
        "lastSent" => p::latest_date_where("lastOutboundMessageAt", op, v, zone),
        "lastInteraction" => p::latest_date_where("lastIncomingMessageAt", op, v, zone),
        "lastInteractionMinutesAgo" => {
            p::latest_minutes_ago_where("lastIncomingMessageAt", op, v, now)
        }
        "lastUserInput" => p::latest_text_where("lastUserInput", op, v),
        "lastUserInputType" => p::latest_text_where("lastUserInputType", op, v),
        "consecutiveAiFailures" => p::latest_number_where("consecutiveFailedReply", op, v),
        "followsBusinessOnInstagram" => p::contact_inbox_tri_state_where("followsBusiness", op, v),
        "businessFollowsUserOnInstagram" => {
            p::contact_inbox_tri_state_where("businessFollowsContact", op, v)
        }
        "verifiedAccountOnInstagram" => p::contact_inbox_tri_state_where("accountVerified", op, v),
        "followerCountOnInstagram" => p::latest_number_where("followerCount", op, v),
        "emailWasVerified" => p::boolean_column("emailVerified", op, v),
        "optedInForEmail" => p::boolean_column("emailOptIn", op, v),
        "existingContact" => p::existing_contact_where(op, v),
        "hasContactInfo" => p::has_contact_info_where(op, v),
        "subscribedToBroadcast" => p::boolean_from_timestamp("broadcastSubscribedAt", op, v),
        "blocked" => p::boolean_from_timestamp("blockedAt", op, v),
        "interactedInLast24h" => p::exists_boolean_where(
            ci,
            Sql::bind("\"ContactInbox\".\"lastIncomingMessageAt\" >= ", now - 86_400_000),
            op,
            v,
        ),
        "fromCtwaAd" => p::exists_boolean_where(
            ci,
            any_channel_ad_conversation_predicate("\"ContactInbox\""),
            op,
            v,
        ),
        "fromGoogleAd" => p::exists_boolean_where(ci, Sql::raw(GOOGLE_CLICK), op, v),
        "ctwaRetarget" => ctwa_retarget_where(c, ctx),
        "tags" | "source" | "currentChannel" | "inbox" | "language" | "broadcastSent"
        | "broadcastDelivered" | "broadcastSeen" | "broadcastClicked" | "broadcastFailed"
        | "subscribedToDripCampaign" | "entryPointsLinks" | "ctwaConversion" => {
            relation_sets::relation_set_where(&c.field, op, v)
        }
        "questionnaireStarted" => p::exists_boolean_where(
            questionnaire_exists,
            questionnaire_predicate(None),
            op,
            v,
        ),
        "questionnaireInProgress" => p::exists_boolean_where(
            questionnaire_exists,
            questionnaire_predicate(Some("inProgress")),
            op,
            v,
        ),
        "questionnaireFinished" => p::exists_boolean_where(
            questionnaire_exists,
            questionnaire_predicate(Some("completed")),
            op,
            v,
        ),
        "couponTopic" => coupon_topic_where(c.topic_id.as_deref(), op, v),
        "conversationAssigned" => conversation_assigned_where(op, v),
        "customField" => field_value::custom_field_where(c, zone),
        "botField" => field_value::bot_field_where(c, zone, ctx.workspace_id),
        "archived" => p::exists_boolean_where(
            conversation_exists,
            Sql::raw("\"Conversation\".\"archivedAt\" IS NOT NULL"),
            op,
            v,
        ),
        "followUp" => p::exists_boolean_where(
            conversation_exists,
            Sql::raw("\"Conversation\".\"followed\" = 1"),
            op,
            v,
        ),
        "conversationTransferredToHuman" => p::exists_boolean_where(
            conversation_exists,
            Sql::bind(
                "\"Conversation\".\"botEnabled\" = 0 AND (\"Conversation\".\"botResumeAt\" IS NULL OR \"Conversation\".\"botResumeAt\" > ",
                now,
            )
            .closed(")"),
            op,
            v,
        ),
        "unreplied" => p::exists_boolean_where(
            ci,
            Sql::raw(
                "\"ContactInbox\".\"lastIncomingMessageAt\" IS NOT NULL AND (\"ContactInbox\".\"lastOutboundMessageAt\" IS NULL OR \"ContactInbox\".\"lastIncomingMessageAt\" > \"ContactInbox\".\"lastOutboundMessageAt\")",
            ),
            op,
            v,
        ),
        "unread" => p::exists_boolean_where(
            conversation_exists,
            Sql::raw(
                "\"Conversation\".\"lastActivityAt\" IS NOT NULL AND (\"Conversation\".\"agentLastReadAt\" IS NULL OR \"Conversation\".\"lastActivityAt\" > \"Conversation\".\"agentLastReadAt\")",
            ),
            op,
            v,
        ),
        "commentedOnPost" => commented_on_post_where(op, v, ctx.workspace_id),
        "leaderboardScore" => leaderboard_score_where(c.leaderboard_id.as_deref(), op, v),
        "isWithinWorkingHours" => working_hours_where(op, v, ctx),
        _ => None,
    }
}
