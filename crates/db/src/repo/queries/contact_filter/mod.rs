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

use serde::{Deserialize, Serialize};
use serde_json::Value as Json;

use crate::repo::sql::Sql;

pub mod permission;
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
    let _ = values;
    todo!("P3.2b")
}

/// `applyContactFilter`: the AND / OR of the conditions that compile; `None` = `{}`.
pub fn apply_contact_filter(
    criteria: &FilterCriteria,
    workspace_id: Option<i64>,
    now: i64,
) -> Option<Sql> {
    let _ = (criteria, workspace_id, now);
    todo!("P3.2b")
}

/// `contactFilterHasPredicate`.
pub fn contact_filter_has_predicate(criteria: &FilterCriteria, workspace_id: Option<i64>) -> bool {
    let _ = (criteria, workspace_id);
    todo!("P3.2b")
}

/// `buildSmartKeywordWhere`; `None` for a blank keyword.
pub fn build_smart_keyword_where(keyword: &str, options: SmartKeywordOptions) -> Option<Sql> {
    let _ = (keyword, options);
    todo!("P3.2b")
}

/// `buildContactWhere`: the workspace scope AND the keyword AND the filter.
pub fn build_contact_where(input: &ContactWhereInput) -> Sql {
    let _ = input;
    todo!("P3.2b")
}

/// `buildContactFilterMatchSQL`: `TRUE` (no conditions), `FALSE` (nothing compiles) or
/// the predicate (with the workspace scope).
pub fn build_contact_filter_match_sql(
    workspace_id: i64,
    criteria: &FilterCriteria,
    now: i64,
) -> Sql {
    let _ = (workspace_id, criteria, now);
    todo!("P3.2b")
}

/// `buildContactInboxContactFilterSQL`: `<contact_id_column> IN (SELECT "Contact"."id"
/// FROM "Contact" WHERE ..)`, or `TRUE` / `FALSE`.
pub fn build_contact_inbox_contact_filter_sql(
    contact_id_column: &str,
    workspace_id: i64,
    criteria: &FilterCriteria,
    now: i64,
) -> Sql {
    let _ = (contact_id_column, workspace_id, criteria, now);
    todo!("P3.2b")
}

/// `contactInboxHasDmSQL`: the `"ContactInbox"` row has a DM conversation (no `sourceId`).
pub fn contact_inbox_has_dm_sql() -> Sql {
    todo!("P3.2b")
}
