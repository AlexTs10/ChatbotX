//! `contact/filter-value-labels.ts` (P3.6-D2): `resolveContactFilterValueLabels`
//! (one primary-key lookup per filter value type, workspace-scoped) and
//! `listContactActivityValueOptions` (the bounded pickers of the
//! activity-backed filters).

use std::collections::{BTreeMap, HashSet};

use chatbotx_db::Db;
use chatbotx_db::repo::contact_referral_event::{ListRecentAdsInput, RecentAd, list_recent_ads};
use chatbotx_db::repo::flow::list_current_version_nodes_by_flow_ids;
use chatbotx_db::repo::pipeline as pipeline_repo;
use chatbotx_db::repo::utils::like_contains;
use chatbotx_db::rusqlite::{Connection, params};
use serde_json::Value;

use crate::pipeline::access::{
    PipelineAccessScope, PipelinePermissions, resolve_pipeline_permissions,
};
use crate::pipeline::service::plain_member_id;
use crate::{BusinessError, Result, tx};

/// `IdLabel`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct IdLabel {
    pub id: String,
    pub name: String,
}

/// `FilterValueLabelTypes`, in the original key order.
pub const FILTER_VALUE_LABEL_TYPES: &[&str] = &[
    "tags",
    "sequences",
    "broadcasts",
    "reflinks",
    "inboxes",
    "members",
    "inboxTeams",
    "flows",
    "flowNodes",
    "keywords",
    "whatsappFlows",
    "emailTopics",
    "pipelines",
    "pipelineStages",
    "ads",
];

fn repo_err(error: chatbotx_db::rusqlite::Error) -> BusinessError {
    BusinessError::Repo(error.into())
}

/// The numeric ids among `ids` as a JSON array (a non-numeric id names no row).
fn numeric(ids: &[String]) -> Vec<i64> {
    ids.iter().filter_map(|id| id.parse().ok()).collect()
}

fn id_list(ids: &[i64]) -> String {
    serde_json::to_string(ids).unwrap_or_else(|_| "[]".into())
}

/// `(id, name)` rows of `sql` bound to `(workspaceId, ids json)`.
fn pairs(conn: &Connection, sql: &str, ws: i64, ids: &[i64]) -> Result<Vec<(i64, Option<String>)>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let mut stmt = conn.prepare(sql).map_err(repo_err)?;
    let rows = stmt
        .query_map(params![ws, id_list(ids)], |r| Ok((r.get(0)?, r.get(1)?)))
        .map_err(repo_err)?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(repo_err)?;
    Ok(rows)
}

/// `selectLabelsByIds(table, { where })`.
fn select_labels(
    conn: &Connection,
    table: &str,
    extra: &str,
    ws: i64,
    ids: &[String],
) -> Result<Vec<IdLabel>> {
    let sql = format!(
        r#"SELECT "id", "name" FROM "{table}" WHERE "workspaceId" = ?1 AND "id" IN (SELECT value FROM json_each(?2)){extra} ORDER BY "id""#
    );
    Ok(pairs(conn, &sql, ws, &numeric(ids))?
        .into_iter()
        .map(|(id, name)| IdLabel {
            id: id.to_string(),
            name: name.unwrap_or_default(),
        })
        .collect())
}

/// `pipelineService.viewableIds`.
fn viewable_pipeline_ids(
    conn: &Connection,
    ws: i64,
    scope: &PipelineAccessScope,
) -> Result<HashSet<i64>> {
    let rows = pipeline_repo::list_by_workspace(conn, ws)?;
    let access = match plain_member_id(scope) {
        Some(member_id) => Some(pipeline_repo::list_member_access(conn, ws, member_id)?),
        None => None,
    };
    Ok(rows
        .into_iter()
        .filter(|p| match &access {
            None => true,
            Some(access) => {
                resolve_pipeline_permissions(
                    scope,
                    access
                        .iter()
                        .find(|a| a.pipeline_id == p.id)
                        .map(|a| PipelinePermissions {
                            can_view: a.can_view,
                            can_edit: a.can_edit,
                            can_delete: a.can_delete,
                            can_manage_settings: a.can_manage_settings,
                        }),
                )
                .can_view
            }
        })
        .map(|p| p.id)
        .collect())
}

/// `formatAdLabel`.
pub fn format_ad_label(ad: &RecentAd) -> String {
    match ad.title.as_deref() {
        Some(title) if !title.is_empty() => format!("{title} ({})", ad.ad_id),
        _ => ad.ad_id.clone(),
    }
}

fn read_node_label(node: &Value) -> String {
    match node["data"]["name"].as_str() {
        Some(name) if !name.trim().is_empty() => name.to_owned(),
        _ => match &node["id"] {
            Value::String(text) => text.clone(),
            Value::Null => String::new(),
            other => other.to_string(),
        },
    }
}

fn resolve(
    conn: &Connection,
    ws: i64,
    kind: &str,
    ids: &[String],
    scope: &PipelineAccessScope,
) -> Result<Vec<IdLabel>> {
    Ok(match kind {
        "tags" => select_labels(conn, "Tag", r#" AND "deletedAt" IS NULL"#, ws, ids)?,
        "sequences" => select_labels(conn, "Sequence", "", ws, ids)?,
        "broadcasts" => select_labels(conn, "Broadcast", r#" AND "deletedAt" IS NULL"#, ws, ids)?,
        "reflinks" => select_labels(conn, "Reflink", r#" AND "type" = 'refLink'"#, ws, ids)?,
        "inboxes" => select_labels(conn, "Inbox", "", ws, ids)?,
        "inboxTeams" => select_labels(conn, "InboxTeam", "", ws, ids)?,
        "flows" => select_labels(conn, "Flow", "", ws, ids)?,
        "emailTopics" => select_labels(conn, "EmailTopic", "", ws, ids)?,
        // `listLabelsByUserIds`: members by user id, the user's name or `--`.
        "members" => pairs(
            conn,
            r#"SELECT wm."userId", u."name" FROM "WorkspaceMember" wm LEFT JOIN "User" u ON u."id" = wm."userId"
               WHERE wm."workspaceId" = ?1 AND wm."userId" IN (SELECT value FROM json_each(?2)) ORDER BY wm."userId""#,
            ws,
            &numeric(ids),
        )?
        .into_iter()
        .map(|(id, name)| IdLabel {
            id: id.to_string(),
            name: name.unwrap_or_else(|| "--".into()),
        })
        .collect(),
        // `listKeywordLabelsByIds`: inbound rules, labelled by their keywords.
        "keywords" => pairs(
            conn,
            r#"SELECT "id", "keywords" FROM "AutomatedResponse" WHERE "workspaceId" = ?1 AND "type" = 'inbound'
               AND "id" IN (SELECT value FROM json_each(?2)) ORDER BY "id""#,
            ws,
            &numeric(ids),
        )?
        .into_iter()
        .map(|(id, keywords)| IdLabel {
            id: id.to_string(),
            name: keyword_label(id, keywords.as_deref()),
        })
        .collect(),
        "whatsappFlows" => pairs(
            conn,
            r#"SELECT wf."id", wf."name" FROM "WhatsappFlow" wf JOIN "IntegrationWhatsapp" iw ON iw."id" = wf."integrationWhatsappId"
               WHERE iw."workspaceId" = ?1 AND wf."id" IN (SELECT value FROM json_each(?2)) ORDER BY wf."id""#,
            ws,
            &numeric(ids),
        )?
        .into_iter()
        .map(|(id, name)| IdLabel {
            id: id.to_string(),
            name: name.unwrap_or_default(),
        })
        .collect(),
        // `listNodeLabelsByIds`: `Flow name · Node name` from the published version.
        "flowNodes" => {
            let refs: Vec<(&String, i64, &str)> = ids
                .iter()
                .filter_map(|id| {
                    let mut parts = id.split(':');
                    let (flow, node) = (parts.next()?, parts.next()?);
                    (!flow.is_empty() && !node.is_empty()).then_some(())?;
                    Some((id, flow.parse().ok()?, node))
                })
                .collect();
            let mut flow_ids: Vec<i64> = refs.iter().map(|r| r.1).collect();
            flow_ids.sort_unstable();
            flow_ids.dedup();
            let flows = list_current_version_nodes_by_flow_ids(conn, ws, &flow_ids)?;
            refs.into_iter()
                .filter_map(|(id, flow_id, node_id)| {
                    let flow = flows.iter().find(|f| f.flow_id == flow_id)?;
                    let node = flow.nodes.iter().find(|n| match &n["id"] {
                        Value::String(text) => text == node_id,
                        other => other.as_i64().map(|n| n.to_string()).as_deref() == Some(node_id),
                    })?;
                    Some(IdLabel {
                        id: id.clone(),
                        name: format!("{} · {}", flow.flow_name, read_node_label(node)),
                    })
                })
                .collect()
        }
        "pipelines" => {
            if ids.is_empty() {
                return Ok(Vec::new());
            }
            let viewable = viewable_pipeline_ids(conn, ws, scope)?;
            select_labels(conn, "Pipeline", "", ws, ids)?
                .into_iter()
                .filter(|l| l.id.parse().is_ok_and(|id| viewable.contains(&id)))
                .collect()
        }
        "pipelineStages" => {
            if ids.is_empty() {
                return Ok(Vec::new());
            }
            let viewable = viewable_pipeline_ids(conn, ws, scope)?;
            let sql = r#"SELECT "id", "name", "pipelineId" FROM "PipelineStage" WHERE "workspaceId" = ?1 AND "id" IN (SELECT value FROM json_each(?2)) ORDER BY "id""#;
            let mut stmt = conn.prepare(sql).map_err(repo_err)?;
            stmt.query_map(params![ws, id_list(&numeric(ids))], |r| {
                Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)?))
            })
            .map_err(repo_err)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(repo_err)?
            .into_iter()
            .filter(|(_, _, pipeline)| viewable.contains(pipeline))
            .map(|(id, name, _)| IdLabel {
                id: id.to_string(),
                name,
            })
            .collect()
        }
        // `contactReferralEventService.listLabelsByIds`: every id, named by
        // its newest headline when one is known.
        "ads" => {
            if ids.is_empty() {
                return Ok(Vec::new());
            }
            let recent = list_recent_ads(
                conn,
                &ListRecentAdsInput {
                    workspace_id: ws,
                    limit: i64::try_from(ids.len()).unwrap_or(i64::MAX),
                    search: None,
                    ad_ids: Some(ids.to_vec()),
                },
            )?;
            ids.iter()
                .map(|id| {
                    let title = recent.iter().find(|ad| &ad.ad_id == id).and_then(|ad| ad.title.clone());
                    IdLabel {
                        id: id.clone(),
                        name: format_ad_label(&RecentAd {
                            ad_id: id.clone(),
                            title,
                        }),
                    }
                })
                .collect()
        }
        _ => Vec::new(),
    })
}

/// `row.keywords.join(", ") || "#<id>"`.
fn keyword_label(id: i64, keywords: Option<&str>) -> String {
    let words: Vec<String> = keywords
        .and_then(|k| serde_json::from_str::<Vec<Value>>(k).ok())
        .unwrap_or_default()
        .iter()
        .map(|w| match w {
            Value::String(text) => text.clone(),
            Value::Null => String::new(),
            other => other.to_string(),
        })
        .collect();
    let joined = words.join(", ");
    if joined.is_empty() {
        format!("#{id}")
    } else {
        joined
    }
}

/// `resolveContactFilterValueLabels`: display names for the ids a filter
/// references, per type (ids de-duplicated, first-seen order); an id missing
/// from the result no longer exists. Every type is present in the result.
pub async fn resolve_contact_filter_value_labels(
    db: &Db,
    workspace_id: i64,
    ids: BTreeMap<String, Vec<String>>,
    pipeline_scope: PipelineAccessScope,
) -> Result<BTreeMap<String, Vec<IdLabel>>> {
    tx::read(db, move |conn| {
        let mut out = BTreeMap::new();
        for kind in FILTER_VALUE_LABEL_TYPES {
            let mut seen = HashSet::new();
            let unique: Vec<String> = ids
                .get(*kind)
                .map(|list| {
                    list.iter()
                        .filter(|id| seen.insert((*id).clone()))
                        .cloned()
                        .collect()
                })
                .unwrap_or_default();
            out.insert(
                (*kind).to_owned(),
                resolve(conn, workspace_id, kind, &unique, &pipeline_scope)?,
            );
        }
        Ok(out)
    })
    .await
}

/// `ContactActivityValueOption`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct ContactActivityValueOption {
    pub value: String,
    pub label: String,
}

/// `ACTIVITY_VALUE_OPTIONS_LIMIT`.
pub const ACTIVITY_VALUE_OPTIONS_LIMIT: i64 = 50;

/// `listContactActivityValueOptions` for `source` (`keywords`,
/// `whatsappFlows`, `emailTopics`, `ads`); the search is trimmed.
pub async fn list_contact_activity_value_options(
    db: &Db,
    workspace_id: i64,
    source: &str,
    search: Option<&str>,
) -> Result<Vec<ContactActivityValueOption>> {
    let source = source.to_owned();
    let search = chatbotx_core::js::trim(search.unwrap_or_default()).to_owned();
    tx::read(db, move |conn| {
        let ws = workspace_id;
        let pattern = (!search.is_empty()).then(|| like_contains(&search));
        let option = |value: i64, label: String| ContactActivityValueOption {
            value: value.to_string(),
            label,
        };
        let query = |sql: &str| -> Result<Vec<(i64, Option<String>)>> {
            let mut stmt = conn.prepare(sql).map_err(repo_err)?;
            let rows = stmt
                .query_map(
                    params![ws, pattern, ACTIVITY_VALUE_OPTIONS_LIMIT],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .map_err(repo_err)?
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(repo_err)?;
            Ok(rows)
        };
        Ok(match source.as_str() {
            // `listKeywordOptions`: `array_to_string(keywords, ' ') ILIKE`.
            "keywords" => query(
                r#"SELECT "id", "keywords" FROM "AutomatedResponse" WHERE "workspaceId" = ?1 AND "type" = 'inbound'
                   AND (?2 IS NULL OR ilike((SELECT group_concat(value, ' ') FROM json_each("keywords")), ?2))
                   ORDER BY "createdAt" DESC, "id" DESC LIMIT ?3"#,
            )?
            .into_iter()
            .map(|(id, keywords)| option(id, keyword_label(id, keywords.as_deref())))
            .collect(),
            "whatsappFlows" => query(
                r#"SELECT wf."id", wf."name" FROM "WhatsappFlow" wf JOIN "IntegrationWhatsapp" iw ON iw."id" = wf."integrationWhatsappId"
                   WHERE iw."workspaceId" = ?1 AND (?2 IS NULL OR ilike(wf."name", ?2))
                   ORDER BY wf."createdAt" ASC, wf."id" ASC LIMIT ?3"#,
            )?
            .into_iter()
            .map(|(id, name)| option(id, name.unwrap_or_default()))
            .collect(),
            "emailTopics" => query(
                r#"SELECT "id", "name" FROM "EmailTopic" WHERE "workspaceId" = ?1 AND (?2 IS NULL OR ilike("name", ?2))
                   ORDER BY "createdAt" DESC, "id" DESC LIMIT ?3"#,
            )?
            .into_iter()
            .map(|(id, name)| option(id, name.unwrap_or_default()))
            .collect(),
            "ads" => list_recent_ads(
                conn,
                &ListRecentAdsInput {
                    workspace_id: ws,
                    limit: ACTIVITY_VALUE_OPTIONS_LIMIT,
                    search: (!search.is_empty()).then(|| search.clone()),
                    ad_ids: None,
                },
            )?
            .iter()
            .map(|ad| ContactActivityValueOption {
                value: ad.ad_id.clone(),
                label: format_ad_label(ad),
            })
            .collect(),
            _ => Vec::new(),
        })
    })
    .await
}
