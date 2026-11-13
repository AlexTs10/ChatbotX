//! Port of `queries/contact-filter/contact-activity.ts`: the activity-backed conditions
//! (`CONTACT_ACTIVITY_FILTERS` fields, `sentMessage`, `lastDelivered`). A condition that
//! cannot be evaluated (unknown operator, malformed id, no workspace, kill switch off)
//! fails closed: `FALSE` (`matchesNobody`), never `None` (that would widen an AND group).
//! Every sub-select binds the workspace id first (the TS partition-pruning `workspaceId =`).

use chrono_tz::Tz;
use rusqlite::types::Value;
use serde_json::Value as Json;

use super::predicates::timestamp_comparison;
use super::sqlx::exists;
use crate::repo::partials::contact_activity::{
    ContactActivityFilterConfig, ContactActivityReference, UNSCOPED_PARENT_REFERENCE_ID,
    parse_contact_activity_references,
};
use crate::repo::sql::Sql;
use crate::repo::utils::{escape_like_pattern, normalize_for_match};

/// `matchesNobody`.
fn matches_nobody() -> Sql {
    Sql::raw("FALSE")
}

/// `isEmpty` / `isNotEmpty` over `any`, or `None` for another operator.
fn emptiness(operator: &str, any: impl Fn() -> Sql) -> Option<Sql> {
    match operator {
        "isEmpty" => Some(exists(any(), true)),
        "isNotEmpty" => Some(exists(any(), false)),
        _ => None,
    }
}

/// `<any> AND <predicate>` inside `[NOT ]EXISTS`.
fn exists_and(mut any: Sql, predicate: Sql, negate: bool) -> Sql {
    any.push(" AND ");
    any.append(predicate);
    exists(any, negate)
}

/// `referenceInList`.
fn reference_in_list(
    config: &ContactActivityFilterConfig,
    references: &[ContactActivityReference],
) -> Sql {
    if config.parent_scoped {
        let rows = vec!["(?, ?)"; references.len()].join(", ");
        let params = references
            .iter()
            .flat_map(|r| {
                [
                    Value::Integer(r.parent_reference_id),
                    Value::Integer(r.reference_id),
                ]
            })
            .collect();
        Sql::new(
            format!("(a.\"parentReferenceId\", a.\"referenceId\") IN (VALUES {rows})"),
            params,
        )
    } else {
        let marks = vec!["?"; references.len()].join(", ");
        let mut params = vec![Value::Integer(UNSCOPED_PARENT_REFERENCE_ID)];
        params.extend(references.iter().map(|r| Value::Integer(r.reference_id)));
        Sql::new(
            format!("a.\"parentReferenceId\" = ? AND a.\"referenceId\" IN ({marks})"),
            params,
        )
    }
}

/// `buildContactActivityWhere`: an existence test on the contact's activity rows; the
/// negatives are the exact `NOT EXISTS` complement.
pub(super) fn contact_activity_where(
    config: &ContactActivityFilterConfig,
    operator: &str,
    value: Option<&Json>,
    workspace_id: Option<i64>,
    enabled: bool,
) -> Sql {
    let Some(workspace_id) = workspace_id.filter(|_| enabled) else {
        return matches_nobody();
    };
    let any = || {
        Sql::new(
            "SELECT 1 FROM \"ContactActivity\" a WHERE a.\"workspaceId\" = ? AND a.\"contactId\" = \"Contact\".\"id\" AND a.\"activityType\" = ? AND a.\"referenceType\" = ?",
            vec![
                Value::Integer(workspace_id),
                Value::Text(config.activity_type.to_owned()),
                Value::Text(config.reference_type.to_owned()),
            ],
        )
    };
    if let Some(q) = emptiness(operator, any) {
        return q;
    }
    let negate = match operator {
        "in" | "eq" => false,
        "notIn" | "ne" => true,
        _ => return matches_nobody(),
    };
    let Some(references) = parse_contact_activity_references(config, value) else {
        return matches_nobody();
    };
    exists_and(any(), reference_in_list(config, &references), negate)
}

/// `MAX_SENT_MESSAGE_LENGTH` (UTF-16 code units, as `String.length`).
const MAX_SENT_MESSAGE_LENGTH: usize = 1000;

/// JS `String.prototype.trim` (Unicode white space + U+FEFF).
fn js_trim(value: &str) -> &str {
    value.trim_matches(|c: char| c.is_whitespace() || c == '\u{feff}')
}

/// `buildSentMessageWhere`: the contact's keyword-matched inbound messages, compared
/// against the stored `normalizedText` (`normalizeForMatch`).
pub(super) fn sent_message_where(
    operator: &str,
    value: Option<&Json>,
    workspace_id: Option<i64>,
    enabled: bool,
) -> Sql {
    let Some(workspace_id) = workspace_id.filter(|_| enabled) else {
        return matches_nobody();
    };
    let any = || {
        Sql::bind(
            "SELECT 1 FROM \"ContactKeywordMessage\" m WHERE m.\"workspaceId\" = ",
            workspace_id,
        )
        .closed(" AND m.\"contactId\" = \"Contact\".\"id\"")
    };
    if let Some(q) = emptiness(operator, any) {
        return q;
    }
    let (positive, negate) = match operator {
        "ne" => ("eq", true),
        "notContains" => ("contains", true),
        other => (other, false),
    };
    if !matches!(positive, "eq" | "startsWith" | "contains" | "endsWith") {
        return matches_nobody();
    }
    let Some(Json::String(raw)) = value else {
        return matches_nobody();
    };
    if raw.encode_utf16().count() > MAX_SENT_MESSAGE_LENGTH {
        return matches_nobody();
    }
    // Judged after normalizing: text made only of combining marks is blank once folded.
    let normalized = normalize_for_match(raw);
    let needle = js_trim(&normalized);
    if needle.is_empty() {
        return matches_nobody();
    }
    let text = "m.\"normalizedText\"";
    let predicate = if positive == "eq" {
        Sql::bind(format!("{text} = "), needle.to_owned())
    } else {
        let escaped = escape_like_pattern(needle);
        let pattern = match positive {
            "startsWith" => format!("{escaped}%"),
            "contains" => format!("%{escaped}%"),
            _ => format!("%{escaped}"),
        };
        Sql::bind(format!("{text} LIKE "), pattern).closed(" ESCAPE '\\'")
    };
    exists_and(any(), predicate, negate)
}

/// `buildLastDeliveredWhere`: the contact's latest confirmed delivery (`ContactState`);
/// the null-safe negatives (`ne`, `notBetween`) are `NOT EXISTS` of the positive window.
pub(super) fn last_delivered_where(
    operator: &str,
    value: Option<&Json>,
    workspace_id: Option<i64>,
    zone: Tz,
    enabled: bool,
) -> Sql {
    let Some(workspace_id) = workspace_id.filter(|_| enabled) else {
        return matches_nobody();
    };
    let any = || {
        Sql::bind(
            "SELECT 1 FROM \"ContactState\" s WHERE s.\"workspaceId\" = ",
            workspace_id,
        )
        .closed(" AND s.\"contactId\" = \"Contact\".\"id\" AND s.\"lastDeliveredAt\" IS NOT NULL")
    };
    if let Some(q) = emptiness(operator, any) {
        return q;
    }
    let (positive, negate) = match operator {
        "ne" => ("eq", true),
        "notBetween" => ("isBetween", true),
        other => (other, false),
    };
    let Some(cmp) = timestamp_comparison(positive, value, zone) else {
        return matches_nobody();
    };
    exists_and(any(), cmp.render("s.\"lastDeliveredAt\""), negate)
}
