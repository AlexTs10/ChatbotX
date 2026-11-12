//! Port of `repositories/pipeline/item-filter.ts`: the shared WHERE of every pipeline
//! board / list query. Every dynamic value is a bind value; field ids are bound as JSON
//! paths, never interpolated. Money amounts (`value`) are the stored x 10,000 integers;
//! dates are unix ms.

use std::collections::BTreeMap;
use std::sync::LazyLock;

use super::pipeline_field_condition::{
    CustomFieldType, PipelineFieldCondition, field_condition_sql,
};
use super::pipeline_item_cursor::bind1;
use super::sql::Sql;
use super::utils::{is_bigint_id, like_contains};
use super::{id_list, text_list};
use regex::Regex;

static NUMERIC_SEARCH_PATTERN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^#?\d+$").expect("numeric search"));
static PHONE_SEARCH_PATTERN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[+\d\s\-()]+$").expect("phone search"));
const MIN_PHONE_SEARCH_DIGITS: usize = 6;

/// `PIPELINE_AUTOMATION_ACTOR_KINDS`.
pub const PIPELINE_AUTOMATION_ACTOR_KINDS: [&str; 4] =
    ["workspaceToken", "flow", "trigger", "system"];

/// `pipelineActivityKinds` (in declaration order).
pub const PIPELINE_ACTIVITY_KINDS: [&str; 16] = [
    "created",
    "stageChanged",
    "statusChanged",
    "valueChanged",
    "priorityChanged",
    "dueDateChanged",
    "titleChanged",
    "descriptionChanged",
    "fieldValuesChanged",
    "assigneesChanged",
    "attachmentAdded",
    "attachmentRemoved",
    "commentAdded",
    "commentDeleted",
    "transferred",
    "deleted",
];

/// `DateRange` (unix ms, both bounds inclusive).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DateRange {
    pub from: Option<i64>,
    pub to: Option<i64>,
}

/// `PipelineValueOperator`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PipelineValueOperator {
    Eq,
    Gt,
    Lt,
    Ne,
    Between,
}

/// The `value` filter: without `operator` the `min`/`max` interval.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PipelineValueFilter {
    pub min: Option<i64>,
    pub max: Option<i64>,
    pub operator: Option<PipelineValueOperator>,
    pub amount: Option<i64>,
}

/// `PipelineActivityMatch`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PipelineActivityMatch {
    pub kinds: Vec<String>,
    pub actor_user_ids: Vec<i64>,
    pub automation: bool,
    pub occurred_at: Option<DateRange>,
}

/// `closedAs`: won / lost closure ranges.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ClosedAs {
    pub won: Option<DateRange>,
    pub lost: Option<DateRange>,
}

/// `PipelineItemFilter`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PipelineItemFilter {
    pub stage_ids: Vec<i64>,
    pub statuses: Vec<String>,
    pub priorities: Vec<String>,
    pub contact_id: Option<i64>,
    pub assignee_member_ids: Vec<i64>,
    pub assignee_team_ids: Vec<i64>,
    pub search: Option<String>,
    /// `Some(false)` keeps contact email and phone out of `search`.
    pub search_contact_email_and_phone: Option<bool>,
    pub value: Option<PipelineValueFilter>,
    pub due_date: Option<DateRange>,
    pub created_at: Option<DateRange>,
    pub updated_at: Option<DateRange>,
    /// Field id -> condition (a `BTreeMap`: iteration is sorted by id).
    pub field_conditions: BTreeMap<String, PipelineFieldCondition>,
    pub timezone: Option<String>,
    pub field_types: BTreeMap<String, CustomFieldType>,
    pub last_comment_at: Option<DateRange>,
    pub status_changed_at: Option<DateRange>,
    pub stage_entered_at: Option<DateRange>,
    /// Activity kind -> range.
    pub activity: BTreeMap<String, DateRange>,
    pub activity_match: Option<PipelineActivityMatch>,
    pub closed_at: Option<DateRange>,
    pub closed_as: Option<ClosedAs>,
}

const ITEM: &str = "\"PipelineItem\"";

fn col(name: &str) -> String {
    format!("{ITEM}.\"{name}\"")
}

fn in_ids(column: &str, ids: &[i64]) -> Sql {
    bind1(
        format!("{column} IN (SELECT value FROM json_each(?))"),
        id_list(ids),
    )
}

fn in_texts<S: AsRef<str>>(column: &str, items: &[S]) -> Sql {
    bind1(
        format!("{column} IN (SELECT value FROM json_each(?))"),
        text_list(items),
    )
}

fn range_conditions(column: &str, range: Option<&DateRange>) -> Vec<Option<Sql>> {
    let Some(range) = range else {
        return Vec::new();
    };
    vec![
        range.from.map(|from| bind1(format!("{column} >= ?"), from)),
        range.to.map(|to| bind1(format!("{column} <= ?"), to)),
    ]
}

fn closed_as_condition(closed_as: Option<&ClosedAs>) -> Option<Sql> {
    let closed_as = closed_as?;
    let branches: Vec<Option<Sql>> = [("won", closed_as.won), ("lost", closed_as.lost)]
        .into_iter()
        .filter_map(|(status, range)| {
            range.map(|range| {
                let mut parts = vec![Some(bind1(
                    format!("{} = ?", col("status")),
                    status.to_owned(),
                ))];
                parts.extend(range_conditions(&col("closedAt"), Some(&range)));
                Some(Sql::and(parts))
            })
        })
        .collect();
    match branches.len() {
        0 => None,
        1 => branches.into_iter().next().flatten(),
        _ => Some(Sql::or(branches)),
    }
}

fn value_conditions(value: Option<&PipelineValueFilter>) -> Vec<Option<Sql>> {
    let Some(value) = value else {
        return Vec::new();
    };
    let v = col("value");
    let single = match value.operator {
        Some(PipelineValueOperator::Eq) => Some("="),
        Some(PipelineValueOperator::Gt) => Some(">"),
        Some(PipelineValueOperator::Lt) => Some("<"),
        Some(PipelineValueOperator::Ne) => Some("is distinct from"),
        _ => None,
    };
    if let Some(op) = single {
        return match value.amount {
            None => Vec::new(),
            Some(amount) if op == "is distinct from" => {
                vec![Some(bind1(format!("{v} IS NOT ?"), amount))]
            }
            Some(amount) => vec![Some(bind1(format!("{v} {op} ?"), amount))],
        };
    }
    vec![
        value.min.map(|min| bind1(format!("{v} >= ?"), min)),
        value.max.map(|max| bind1(format!("{v} <= ?"), max)),
    ]
}

const ACTIVITY: &str = "\"PipelineItemActivity\"";

fn activity_scope() -> Vec<Option<Sql>> {
    vec![
        Some(Sql::raw(format!(
            "{ACTIVITY}.\"workspaceId\" = {}",
            col("workspaceId")
        ))),
        Some(Sql::raw(format!("{ACTIVITY}.\"itemId\" = {}", col("id")))),
        // Only history recorded in the item's current pipeline.
        Some(Sql::raw(format!(
            "{ACTIVITY}.\"pipelineId\" = {}",
            col("pipelineId")
        ))),
    ]
}

fn exists(table: &str, cond: Sql) -> Sql {
    let mut out = Sql::raw(format!("EXISTS (SELECT 1 FROM {table} WHERE "));
    out.append(cond);
    out.push(")");
    out
}

fn activity_exists(kind: &str, range: &DateRange) -> Sql {
    let mut parts = activity_scope();
    parts.push(Some(bind1(
        format!("{ACTIVITY}.\"kind\" = ?"),
        kind.to_owned(),
    )));
    parts.extend(range_conditions(
        &format!("{ACTIVITY}.\"occurredAt\""),
        Some(range),
    ));
    exists(ACTIVITY, Sql::and(parts))
}

fn activity_actor_condition(m: &PipelineActivityMatch) -> Option<Sql> {
    let mut branches = Vec::new();
    if !m.actor_user_ids.is_empty() {
        branches.push(Some(Sql::and([
            Some(Sql::raw(format!("{ACTIVITY}.\"actorKind\" = 'user'"))),
            Some(in_ids(
                &format!("{ACTIVITY}.\"actorUserId\""),
                &m.actor_user_ids,
            )),
        ])));
    }
    if m.automation {
        branches.push(Some(in_texts(
            &format!("{ACTIVITY}.\"actorKind\""),
            &PIPELINE_AUTOMATION_ACTOR_KINDS[..],
        )));
    }
    (!branches.is_empty()).then(|| Sql::or(branches))
}

fn activity_match_exists(m: &PipelineActivityMatch) -> Sql {
    let mut parts = activity_scope();
    if !m.kinds.is_empty() {
        parts.push(Some(in_texts(&format!("{ACTIVITY}.\"kind\""), &m.kinds)));
    }
    parts.push(activity_actor_condition(m));
    parts.extend(range_conditions(
        &format!("{ACTIVITY}.\"occurredAt\""),
        m.occurred_at.as_ref(),
    ));
    exists(ACTIVITY, Sql::and(parts))
}

fn has_activity_match(m: &PipelineActivityMatch) -> bool {
    !m.kinds.is_empty()
        || !m.actor_user_ids.is_empty()
        || m.automation
        || m.occurred_at
            .is_some_and(|r| r.from.is_some() || r.to.is_some())
}

/// Contact columns a board search looks at, by what the input looks like.
fn contact_search_conditions(search: &str, include_email_and_phone: bool) -> Vec<Option<Sql>> {
    let mut out = vec![Some(bind1(
        "ilike(\"Contact\".\"fullName\", ?)",
        like_contains(search),
    ))];
    if include_email_and_phone && search.contains('@') {
        out.push(Some(bind1(
            "ilike(\"Contact\".\"email\", ?)",
            like_contains(search),
        )));
    }
    let digits: String = search.chars().filter(char::is_ascii_digit).collect();
    if include_email_and_phone
        && PHONE_SEARCH_PATTERN.is_match(search)
        && digits.len() >= MIN_PHONE_SEARCH_DIGITS
    {
        out.push(Some(bind1(
            "ilike(\"Contact\".\"phoneNumber\", ?)",
            like_contains(&digits),
        )));
    }
    if is_bigint_id(search) {
        out.push(Some(bind1(
            "CAST(\"Contact\".\"id\" AS TEXT) = ?",
            search.to_owned(),
        )));
    }
    out
}

fn search_condition(search: &str, include_email_and_phone: bool) -> Sql {
    // "#12", "12" and "0012" all mean item 12, compared as a number.
    let digits = if NUMERIC_SEARCH_PATTERN.is_match(search) {
        search
            .replacen('#', "", 1)
            .trim_start_matches('0')
            .to_owned()
    } else {
        String::new()
    };
    let item_number = is_bigint_id(&digits).then(|| digits.parse::<i64>().unwrap_or(0));
    let contact = Sql::and([
        Some(Sql::raw(format!(
            "\"Contact\".\"id\" = {}",
            col("contactId")
        ))),
        Some(Sql::raw(format!(
            "\"Contact\".\"workspaceId\" = {}",
            col("workspaceId")
        ))),
        Some(Sql::or(contact_search_conditions(
            search,
            include_email_and_phone,
        ))),
    ]);
    Sql::or([
        Some(bind1(
            format!("ilike({}, ?)", col("title")),
            like_contains(search),
        )),
        item_number.map(|n| bind1(format!("{} = ?", col("number")), n)),
        Some(exists("\"Contact\"", contact)),
    ])
}

/// `buildPipelineItemWhere`: the shared WHERE of every board / list query (over
/// `"PipelineItem"`).
pub fn build_pipeline_item_where(
    workspace_id: i64,
    pipeline_id: Option<i64>,
    filter: &PipelineItemFilter,
) -> Sql {
    let mut conditions: Vec<Option<Sql>> = vec![
        Some(bind1(format!("{} = ?", col("workspaceId")), workspace_id)),
        pipeline_id.map(|p| bind1(format!("{} = ?", col("pipelineId")), p)),
        (!filter.stage_ids.is_empty()).then(|| in_ids(&col("stageId"), &filter.stage_ids)),
        (!filter.statuses.is_empty()).then(|| in_texts(&col("status"), &filter.statuses)),
        (!filter.priorities.is_empty()).then(|| in_texts(&col("priority"), &filter.priorities)),
        filter
            .contact_id
            .map(|c| bind1(format!("{} = ?", col("contactId")), c)),
    ];
    conditions.extend(value_conditions(filter.value.as_ref()));
    conditions.extend(range_conditions(&col("dueDate"), filter.due_date.as_ref()));
    conditions.extend(range_conditions(
        &col("createdAt"),
        filter.created_at.as_ref(),
    ));
    conditions.extend(range_conditions(
        &col("updatedAt"),
        filter.updated_at.as_ref(),
    ));
    conditions.extend(range_conditions(
        &col("closedAt"),
        filter.closed_at.as_ref(),
    ));
    conditions.push(closed_as_condition(filter.closed_as.as_ref()));
    conditions.extend(range_conditions(
        &col("lastCommentAt"),
        filter.last_comment_at.as_ref(),
    ));
    conditions.extend(range_conditions(
        &col("statusChangedAt"),
        filter.status_changed_at.as_ref(),
    ));
    conditions.extend(range_conditions(
        &col("stageEnteredAt"),
        filter.stage_entered_at.as_ref(),
    ));

    let mut assignee = Vec::new();
    if !filter.assignee_member_ids.is_empty() {
        assignee.push(Some(in_ids(
            "\"PipelineItemAssignee\".\"workspaceMemberId\"",
            &filter.assignee_member_ids,
        )));
    }
    if !filter.assignee_team_ids.is_empty() {
        assignee.push(Some(in_ids(
            "\"PipelineItemAssignee\".\"inboxTeamId\"",
            &filter.assignee_team_ids,
        )));
    }
    if !assignee.is_empty() {
        conditions.push(Some(exists(
            "\"PipelineItemAssignee\"",
            Sql::and([
                Some(Sql::raw(format!(
                    "\"PipelineItemAssignee\".\"itemId\" = {}",
                    col("id")
                ))),
                Some(Sql::or(assignee)),
            ]),
        )));
    }

    if let Some(search) = filter
        .search
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        conditions.push(Some(search_condition(
            search,
            filter.search_contact_email_and_phone != Some(false),
        )));
    }

    for (field_id, condition) in &filter.field_conditions {
        if let Some(field_type) = filter.field_types.get(field_id) {
            conditions.push(field_condition_sql(
                field_id,
                *field_type,
                condition,
                filter.timezone.as_deref(),
            ));
        }
    }
    if let Some(m) = filter
        .activity_match
        .as_ref()
        .filter(|m| has_activity_match(m))
    {
        conditions.push(Some(activity_match_exists(m)));
    }
    for kind in PIPELINE_ACTIVITY_KINDS {
        if let Some(range) = filter.activity.get(kind) {
            conditions.push(Some(activity_exists(kind, range)));
        }
    }
    Sql::and(conditions)
}
