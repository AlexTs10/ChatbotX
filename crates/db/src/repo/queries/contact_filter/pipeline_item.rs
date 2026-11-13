//! Port of `queries/contact-filter/pipeline-item.ts`: the "pipeline item" condition. Every
//! shape is a workspace-scoped correlated `EXISTS` over `"PipelineItem"` (positive operators)
//! or its `NOT EXISTS` twin (`isEmpty`, `notIn`), so a contact without any matching item
//! satisfies the negative operators; a condition that cannot be evaluated fails closed
//! (`FALSE`). Every user-supplied value is a bound parameter.

use rusqlite::types::Value;
use serde_json::Value as Json;

use super::FilterCondition;
use super::sqlx::exists;
use crate::repo::sql::Sql;
use crate::repo::utils::is_bigint_id;

/// `pipelineItemStatuses`.
const STATUSES: &[&str] = &["open", "won", "lost"];
/// `pipelineItemPriorities`.
const PRIORITIES: &[&str] = &["low", "medium", "high"];
/// `MAX_PIPELINE_ITEM_FILTER_STAGE_IDS`.
const MAX_STAGE_IDS: usize = 200;

/// `matchesNobody`.
fn matches_nobody() -> Sql {
    Sql::raw("FALSE")
}

/// `isCanonicalBigintId`: a positive bigint with no leading zero.
fn canonical_id(value: &str) -> Option<i64> {
    if value.starts_with('0') || !is_bigint_id(value) {
        return None;
    }
    value.parse().ok()
}

/// The unique strings of a non-empty JSON array (first occurrence order), or `None` when
/// the value is not one or an item is not a string.
fn unique_strings(value: Option<&Json>) -> Option<Vec<&str>> {
    let Some(Json::Array(items)) = value else {
        return None;
    };
    if items.is_empty() {
        return None;
    }
    let mut out: Vec<&str> = Vec::new();
    for item in items {
        let s = item.as_str()?;
        if !out.contains(&s) {
            out.push(s);
        }
    }
    Some(out)
}

/// `parseEnumList`.
fn enum_list<'a>(value: Option<&'a Json>, allowed: &[&str]) -> Option<Vec<&'a str>> {
    let values = unique_strings(value)?;
    values.iter().all(|v| allowed.contains(v)).then_some(values)
}

/// `parseStageIds`.
fn stage_ids(value: Option<&Json>) -> Option<Vec<i64>> {
    if matches!(value, Some(Json::Array(items)) if items.len() > MAX_STAGE_IDS) {
        return None;
    }
    unique_strings(value)?
        .into_iter()
        .map(canonical_id)
        .collect()
}

/// `"<column>" IN (?, ..)`.
fn column_in(column: &str, values: Vec<Value>) -> Sql {
    let marks = vec!["?"; values.len()].join(", ");
    Sql::new(
        format!("\"PipelineItem\".\"{column}\" IN ({marks})"),
        values,
    )
}

/// `buildPipelineItemConditionWhere`.
pub(super) fn pipeline_item_where(c: &FilterCondition, workspace_id: Option<i64>) -> Sql {
    let attribute = c.attribute.as_deref().unwrap_or_default();
    let Some(workspace_id) = workspace_id else {
        return matches_nobody();
    };
    if !matches!(attribute, "exists" | "status" | "stage" | "priority") {
        return matches_nobody();
    }
    // `pipelineId ?? null`; a present id must be canonical.
    let pipeline_id = match c.pipeline_id.as_ref() {
        None | Some(Json::Null) => None,
        Some(Json::String(s)) => match canonical_id(s) {
            Some(id) => Some(id),
            None => return matches_nobody(),
        },
        Some(_) => return matches_nobody(),
    };
    let item_where = |extra: Option<Sql>| {
        let mut q = Sql::bind(
            "SELECT 1 FROM \"PipelineItem\" WHERE \"PipelineItem\".\"workspaceId\" = ",
            workspace_id,
        );
        q.push(" AND \"PipelineItem\".\"contactId\" = \"Contact\".\"id\"");
        if let Some(id) = pipeline_id {
            q.push(" AND \"PipelineItem\".\"pipelineId\" = ");
            q.push_bind(id);
        }
        if let Some(extra) = extra {
            q.push(" AND ");
            q.append(extra);
        }
        q
    };
    let value = c.value.as_ref();
    if attribute == "exists" {
        let negate = match c.operator.as_str() {
            "isNotEmpty" => false,
            "isEmpty" => true,
            _ => return matches_nobody(),
        };
        let extra = match value {
            Some(Json::String(s)) if s == "any" => None,
            Some(Json::String(s)) if STATUSES.contains(&s.as_str()) => {
                Some(Sql::bind("\"PipelineItem\".\"status\" = ", s.clone()))
            }
            _ => return matches_nobody(),
        };
        return exists(item_where(extra), negate);
    }
    let negate = match c.operator.as_str() {
        "in" => false,
        "notIn" => true,
        _ => return matches_nobody(),
    };
    let texts = |values: Vec<&str>| {
        values
            .into_iter()
            .map(|v| Value::Text(v.to_owned()))
            .collect()
    };
    let test = match attribute {
        "status" => enum_list(value, STATUSES).map(|v| column_in("status", texts(v))),
        "priority" => enum_list(value, PRIORITIES).map(|v| column_in("priority", texts(v))),
        // Stage ids are only meaningful inside one pipeline.
        _ => stage_ids(value)
            .filter(|_| pipeline_id.is_some())
            .map(|ids| column_in("stageId", ids.into_iter().map(Value::Integer).collect())),
    };
    match test {
        Some(test) => exists(item_where(Some(test)), negate),
        None => matches_nobody(),
    }
}
