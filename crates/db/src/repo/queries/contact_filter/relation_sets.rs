//! Port of `queries/contact-filter/relation-sets.ts`: "has one of these" relation filters.

use rusqlite::types::Value;
use serde_json::Value as Json;

use super::sqlx::{exists, id_value, in_list, join_exists, json_scalar};
use crate::repo::sql::Sql;

struct RelationSet {
    table: &'static str,
    contact_column: &'static str,
    column: &'static str,
    /// Bind values as ids (integer when they are integer literals).
    ids: bool,
    has_value: Option<&'static str>,
    extra: Option<&'static str>,
}

const fn set(table: &'static str, column: &'static str, ids: bool) -> RelationSet {
    RelationSet {
        table,
        contact_column: "contactId",
        column,
        ids,
        has_value: None,
        extra: None,
    }
}

const fn broadcast(extra: &'static str) -> RelationSet {
    RelationSet {
        extra: Some(extra),
        ..set("ContactOnBroadcast", "broadcastId", true)
    }
}

fn filter_of(field: &str) -> Option<RelationSet> {
    Some(match field {
        "source" => set("ContactInbox", "source", false),
        "currentChannel" => set("ContactInbox", "channel", false),
        "inbox" => set("ContactInbox", "inboxId", true),
        "language" => RelationSet {
            has_value: Some(
                "\"ContactInbox\".\"language\" IS NOT NULL AND \"ContactInbox\".\"language\" <> ''",
            ),
            ..set("ContactInbox", "language", false)
        },
        "tags" => set("ContactToTag", "tagId", true),
        "broadcastSent" => broadcast("\"ContactOnBroadcast\".\"sent\" = 1"),
        "broadcastDelivered" => broadcast("\"ContactOnBroadcast\".\"deliveredAt\" IS NOT NULL"),
        "broadcastSeen" => broadcast("\"ContactOnBroadcast\".\"seenAt\" IS NOT NULL"),
        "broadcastClicked" => broadcast("\"ContactOnBroadcast\".\"clickedAt\" IS NOT NULL"),
        "broadcastFailed" => broadcast("\"ContactOnBroadcast\".\"failedAt\" IS NOT NULL"),
        "subscribedToDripCampaign" => set("ContactOnSequence", "sequenceId", true),
        "entryPointsLinks" => set("RefLinkStat", "linkId", true),
        "ctwaConversion" => set("AdsConversionEvent", "eventType", false),
        _ => return None,
    })
}

fn relation_exists(f: &RelationSet, predicate: Option<Sql>, negate: bool) -> Sql {
    if f.table != "AdsConversionEvent" {
        return join_exists(f.table, f.contact_column, predicate, negate);
    }
    let mut inner = Sql::raw(
        "SELECT 1 FROM \"AdsConversionEvent\" INNER JOIN \"ContactInbox\" ON \"ContactInbox\".\"id\" = \"AdsConversionEvent\".\"contactInboxId\" WHERE \"ContactInbox\".\"contactId\" = \"Contact\".\"id\"",
    );
    if let Some(p) = predicate {
        inner.push(" AND ");
        inner.append(p);
    }
    exists(inner, negate)
}

fn combine(parts: Vec<Option<Sql>>) -> Option<Sql> {
    super::sqlx::join_raw(parts.into_iter().flatten().collect(), " AND ")
}

/// `buildRelationSetWhere`.
pub(crate) fn relation_set_where(field: &str, operator: &str, value: Option<&Json>) -> Option<Sql> {
    let f = filter_of(field)?;
    if !matches!(operator, "in" | "notIn" | "eq" | "ne" | "isEmpty") {
        return None;
    }
    let extra = f.extra.map(Sql::raw);
    if operator == "isEmpty" {
        let p = combine(vec![f.has_value.map(Sql::raw), extra]);
        return Some(relation_exists(&f, p, true));
    }
    let raw: Vec<&Json> = match value {
        Some(Json::Array(a)) => a.iter().collect(),
        Some(v) => vec![v],
        None => Vec::new(),
    };
    let values: Vec<Value> = raw
        .into_iter()
        .filter_map(|v| match v {
            Json::String(s) if f.ids => Some(id_value(s)),
            other => json_scalar(other),
        })
        .collect();
    let col = format!("\"{}\".\"{}\"", f.table, f.column);
    let p = combine(vec![Some(in_list(&col, values, false)), extra]);
    Some(relation_exists(&f, p, !matches!(operator, "in" | "eq")))
}
