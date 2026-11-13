//! Port of `repositories/flow/repository.ts` (`flowRepository`, P3.2b).

use rusqlite::Connection;
use rusqlite::types::Value;

use super::models::{Flow, FlowVersion};
use super::sql::Sql;
use super::utils::{
    PaginationInput, SortItem, like_contains, order_by_sql, parse_order_by, parse_pagination,
};
use super::{Model, Result, find_one, id_list};

/// `rootFolderId` (`partials/folder.ts`).
pub const ROOT_FOLDER_ID: &str = "0";

/// `FlowListInput` (`folder_id` is the API string: `"0"` = root).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FlowListInput {
    pub workspace_id: i64,
    pub name: Option<String>,
    pub folder_id: Option<String>,
    pub active: Option<bool>,
    pub enable_in_inbox: Option<bool>,
    pub page: Option<i64>,
    pub per_page: Option<i64>,
    pub sort: Option<Vec<SortItem>>,
}

/// A flow with its `flowVersions` (by id).
#[derive(Debug, Clone, PartialEq)]
pub struct FlowWithVersions {
    pub flow: Flow,
    pub flow_versions: Vec<FlowVersion>,
}

/// `{ id, name }` of `listPublishedOptions`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlowOption {
    pub id: i64,
    pub name: String,
}

/// One row of `listCurrentVersionNodesByFlowIds`.
#[derive(Debug, Clone, PartialEq)]
pub struct FlowCurrentVersionNodes {
    pub flow_id: i64,
    pub flow_name: String,
    pub nodes: Vec<serde_json::Value>,
}

/// `buildWhere` over the unaliased `"Flow"` table.
fn build_where(input: &FlowListInput) -> Sql {
    let folder = input
        .folder_id
        .as_deref()
        .filter(|f| !f.is_empty())
        .map(|f| {
            if f == ROOT_FOLDER_ID {
                Sql::raw("\"folderId\" IS NULL")
            } else {
                // A non-numeric id matches nothing (Postgres would reject the bigint cast).
                match f.parse::<i64>() {
                    Ok(id) => Sql::bind("\"folderId\" = ", id),
                    Err(_) => Sql::raw("1 = 0"),
                }
            }
        });
    Sql::and([
        Some(Sql::bind("\"workspaceId\" = ", input.workspace_id)),
        folder,
        input
            .name
            .as_deref()
            .filter(|n| !n.is_empty())
            .map(|n| Sql::bind("ilike(\"name\", ", like_contains(n)).closed(")")),
        input
            .active
            .map(|a| Sql::bind("\"active\" = ", i64::from(a))),
        input
            .enable_in_inbox
            .map(|a| Sql::bind("\"enableInInbox\" = ", i64::from(a))),
    ])
}

/// The versions of `flow_ids` (by id), narrowed by the SQL tail `extra`.
fn versions_of(conn: &Connection, flow_ids: &[i64], extra: &str) -> Result<Vec<FlowVersion>> {
    if flow_ids.is_empty() {
        return Ok(Vec::new());
    }
    Sql::new(
        format!(
            "{} WHERE \"flowId\" IN (SELECT value FROM json_each(?)) {extra} ORDER BY \"id\"",
            FlowVersion::select_sql()
        ),
        vec![Value::Text(id_list(flow_ids))],
    )
    .models(conn)
}

fn attach(flows: Vec<Flow>, versions: Vec<FlowVersion>) -> Vec<FlowWithVersions> {
    flows
        .into_iter()
        .map(|flow| {
            let flow_versions = versions
                .iter()
                .filter(|v| v.flow_id == flow.id)
                .cloned()
                .collect();
            FlowWithVersions {
                flow,
                flow_versions,
            }
        })
        .collect()
}

/// The elements of a `nodes` JSON array (anything else = none).
fn node_list(nodes: super::Json) -> Vec<serde_json::Value> {
    match nodes.0 {
        serde_json::Value::Array(items) => items,
        _ => Vec::new(),
    }
}

/// `listNamesWithPrefix`: names of the workspace's flows that start with `prefix`
/// (plain text, case-sensitive).
pub fn list_names_with_prefix(
    conn: &Connection,
    workspace_id: i64,
    prefix: &str,
) -> Result<Vec<String>> {
    Sql::new(
        "SELECT \"name\" FROM \"Flow\" WHERE \"workspaceId\" = ? \
         AND substr(\"name\", 1, length(?2)) = ?2 ORDER BY \"id\"",
        vec![Value::Integer(workspace_id), Value::Text(prefix.to_owned())],
    )
    .query_map(conn, |r| r.get(0))
}

/// `existsInWorkspace`.
pub fn exists_in_workspace(conn: &Connection, workspace_id: i64, id: i64) -> Result<bool> {
    let row: Option<i64> = Sql::new(
        "SELECT \"id\" FROM \"Flow\" WHERE \"id\" = ? AND \"workspaceId\" = ? LIMIT 1",
        vec![Value::Integer(id), Value::Integer(workspace_id)],
    )
    .query_map(conn, |r| r.get(0))?
    .into_iter()
    .next();
    Ok(row.is_some())
}

/// `listPublishedOptions`: active flows with a published version (by id).
pub fn list_published_options(conn: &Connection, workspace_id: i64) -> Result<Vec<FlowOption>> {
    Sql::new(
        "SELECT \"id\", \"name\" FROM \"Flow\" WHERE \"workspaceId\" = ? AND \"active\" = 1 \
         AND \"currentVersionId\" IS NOT NULL ORDER BY \"id\"",
        vec![Value::Integer(workspace_id)],
    )
    .query_map(conn, |r| {
        Ok(FlowOption {
            id: r.get(0)?,
            name: r.get(1)?,
        })
    })
}

/// `listWithVersions`: the page of flows, each with its draft and latest versions.
pub fn list_with_versions(
    conn: &Connection,
    input: &FlowListInput,
) -> Result<Vec<FlowWithVersions>> {
    let mut q = Sql::raw(format!("{} WHERE ", Flow::select_sql()));
    q.append(build_where(input));
    let order = parse_order_by(Flow::COLUMNS, input.sort.as_deref(), None);
    q.push(" ");
    if order.is_empty() {
        q.push("ORDER BY \"id\"");
    } else {
        q.push(&order_by_sql(&order, None));
    }
    if let Some(p) = parse_pagination(PaginationInput {
        page: input.page,
        per_page: input.per_page,
    }) {
        q.push(" LIMIT ");
        q.push_bind(p.limit);
        q.push(" OFFSET ");
        q.push_bind(p.offset);
    }
    let flows: Vec<Flow> = q.models(conn)?;
    let ids: Vec<i64> = flows.iter().map(|f| f.id).collect();
    let versions = versions_of(conn, &ids, "AND (\"isDraft\" = 1 OR \"isLatest\" = 1)")?;
    Ok(attach(flows, versions))
}

/// `count` (same filter as `listWithVersions`).
pub fn count(conn: &Connection, input: &FlowListInput) -> Result<i64> {
    let mut q = Sql::raw("SELECT count(*) FROM \"Flow\" WHERE ");
    q.append(build_where(input));
    Ok(q.query_map(conn, |r| r.get::<_, i64>(0))?
        .into_iter()
        .next()
        .unwrap_or(0))
}

/// `findWithVersions`: the flow with all its versions.
pub fn find_with_versions(
    conn: &Connection,
    workspace_id: i64,
    id: i64,
) -> Result<Option<FlowWithVersions>> {
    let Some(flow) = find_one::<Flow>(
        conn,
        "\"workspaceId\" = ?1 AND \"id\" = ?2",
        &[&workspace_id, &id],
    )?
    else {
        return Ok(None);
    };
    let versions = versions_of(conn, &[flow.id], "")?;
    Ok(attach(vec![flow], versions).pop())
}

/// `findCurrentVersionNodes`: `None` when the flow is not in the workspace, an empty
/// list when it has no published version.
pub fn find_current_version_nodes(
    conn: &Connection,
    workspace_id: i64,
    flow_id: i64,
) -> Result<Option<Vec<serde_json::Value>>> {
    let current: Option<Option<i64>> = Sql::new(
        "SELECT \"currentVersionId\" FROM \"Flow\" WHERE \"workspaceId\" = ? AND \"id\" = ? LIMIT 1",
        vec![Value::Integer(workspace_id), Value::Integer(flow_id)],
    )
    .query_map(conn, |r| r.get(0))?
    .into_iter()
    .next();
    let Some(current) = current else {
        return Ok(None);
    };
    let Some(version_id) = current else {
        return Ok(Some(Vec::new()));
    };
    let nodes: Option<super::Json> = Sql::new(
        "SELECT \"nodes\" FROM \"FlowVersion\" WHERE \"id\" = ? AND \"flowId\" = ? \
         AND \"workspaceId\" = ? LIMIT 1",
        vec![
            Value::Integer(version_id),
            Value::Integer(flow_id),
            Value::Integer(workspace_id),
        ],
    )
    .query_map(conn, |r| r.get(0))?
    .into_iter()
    .next();
    Ok(Some(nodes.map(node_list).unwrap_or_default()))
}

/// `listCurrentVersionNodesByFlowIds` (flows of another workspace are absent).
pub fn list_current_version_nodes_by_flow_ids(
    conn: &Connection,
    workspace_id: i64,
    flow_ids: &[i64],
) -> Result<Vec<FlowCurrentVersionNodes>> {
    if flow_ids.is_empty() {
        return Ok(Vec::new());
    }
    let flows: Vec<(i64, String, Option<i64>)> = Sql::new(
        "SELECT \"id\", \"name\", \"currentVersionId\" FROM \"Flow\" WHERE \"workspaceId\" = ? \
         AND \"id\" IN (SELECT value FROM json_each(?)) ORDER BY \"id\"",
        vec![Value::Integer(workspace_id), Value::Text(id_list(flow_ids))],
    )
    .query_map(conn, |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
    let version_ids: Vec<i64> = flows.iter().filter_map(|f| f.2).collect();
    let versions: Vec<(i64, super::Json)> = if version_ids.is_empty() {
        Vec::new()
    } else {
        Sql::new(
            "SELECT \"id\", \"nodes\" FROM \"FlowVersion\" WHERE \"workspaceId\" = ? \
             AND \"id\" IN (SELECT value FROM json_each(?))",
            vec![
                Value::Integer(workspace_id),
                Value::Text(id_list(&version_ids)),
            ],
        )
        .query_map(conn, |r| Ok((r.get(0)?, r.get(1)?)))?
    };
    Ok(flows
        .into_iter()
        .map(|(flow_id, flow_name, current)| {
            let nodes = current
                .and_then(|v| versions.iter().find(|(id, _)| *id == v))
                .map(|(_, nodes)| node_list(nodes.clone()))
                .unwrap_or_default();
            FlowCurrentVersionNodes {
                flow_id,
                flow_name,
                nodes,
            }
        })
        .collect())
}

/// `listIdsByIds`: the ids of `ids` that exist in the workspace.
pub fn list_ids_by_ids(conn: &Connection, workspace_id: i64, ids: &[i64]) -> Result<Vec<i64>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    Sql::new(
        "SELECT \"id\" FROM \"Flow\" WHERE \"workspaceId\" = ? \
         AND \"id\" IN (SELECT value FROM json_each(?)) ORDER BY \"id\"",
        vec![Value::Integer(workspace_id), Value::Text(id_list(ids))],
    )
    .query_map(conn, |r| r.get(0))
}
