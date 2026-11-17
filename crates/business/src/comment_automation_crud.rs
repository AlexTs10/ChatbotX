//! The CRUD half of `commentAutomationService` (Messenger, Instagram, Threads,
//! TikTok rows of `CommentAutomation`) and `igStoryAutomationService`
//! (`IgStoryAutomation`), plus `assertAutomationAccount`
//! (`comment-automation/account-scope.ts`) and the write normalisers
//! (`withNormalizedReplies`, `withSupportedHideComments`,
//! `withSupportedImageFilter`, `withLiveCapabilities`, the Threads / TikTok
//! builders, `pinStoryMentionFilters`).
//!
//! Rows go in and out as JSON objects with the column names (the public
//! resource is every column but `workspaceId`). The API layer validates the
//! request first (the zod schemas); this module only writes what it gets.

use chatbotx_db::Db;
use chatbotx_db::repo::utils::parse_order_by;
use chatbotx_db::repo::{SortItem, flow, iso, like_contains, new_id, now_ms};
use chatbotx_db::rusqlite::types::{Value as Sql, ValueRef};
use chatbotx_db::rusqlite::{Connection, OptionalExtension as _, params_from_iter};
use serde_json::{Map, Value, json};

use crate::errors::{BusinessError, Result};
use crate::tx;

fn repo_err(error: chatbotx_db::rusqlite::Error) -> BusinessError {
    BusinessError::Repo(error.into())
}

/// One automation table.
#[derive(Debug, Clone, Copy)]
pub struct Table {
    pub name: &'static str,
    /// Every column but `workspaceId`, in resource order.
    pub columns: &'static [&'static str],
    pub json: &'static [&'static str],
}

/// `CommentAutomation`.
pub const COMMENTS: Table = Table {
    name: "CommentAutomation",
    columns: &[
        "id",
        "createdAt",
        "updatedAt",
        "name",
        "folderId",
        "type",
        "inboxId",
        "isActive",
        "startTime",
        "endTime",
        "repliesCount",
        "sentCount",
        "deliveredCount",
        "seenCount",
        "clickedCount",
        "failedCount",
        "missedCount",
        "moderatedCount",
        "post",
        "privateReply",
        "publicReply",
        "includeKeywords",
        "excludeKeywords",
        "excludeKeywordsType",
        "options",
        "hideComments",
        "replyAfter",
    ],
    json: &[
        "post",
        "privateReply",
        "publicReply",
        "includeKeywords",
        "excludeKeywords",
        "options",
        "hideComments",
        "replyAfter",
    ],
};

/// `IgStoryAutomation`.
pub const STORIES: Table = Table {
    name: "IgStoryAutomation",
    columns: &[
        "id",
        "createdAt",
        "updatedAt",
        "name",
        "folderId",
        "type",
        "trigger",
        "inboxId",
        "isActive",
        "startTime",
        "endTime",
        "repliesCount",
        "story",
        "reply",
        "includeKeywords",
    ],
    json: &["story", "reply", "includeKeywords"],
};

const ID_COLUMNS: &[&str] = &["id", "folderId", "inboxId"];
const DATE_COLUMNS: &[&str] = &["createdAt", "updatedAt"];

/// `igCommentAutomationTypes` / `igStoryAutomationTypes`.
pub const IG_TYPES: &[&str] = &["instagram", "instagramFacebook"];

fn select_list(table: Table) -> String {
    table
        .columns
        .iter()
        .map(|c| format!("\"{c}\""))
        .collect::<Vec<_>>()
        .join(", ")
}

fn row_json(
    table: Table,
    row: &chatbotx_db::rusqlite::Row<'_>,
) -> chatbotx_db::rusqlite::Result<Value> {
    let mut out = Map::new();
    for (index, column) in table.columns.iter().enumerate() {
        let value = match row.get_ref(index)? {
            ValueRef::Null => Value::Null,
            ValueRef::Integer(n) if ID_COLUMNS.contains(column) => json!(n.to_string()),
            ValueRef::Integer(n) if DATE_COLUMNS.contains(column) => json!(iso(n)),
            ValueRef::Integer(n) if *column == "isActive" => json!(n != 0),
            ValueRef::Integer(n) => json!(n),
            ValueRef::Real(f) => json!(f),
            ValueRef::Text(bytes) => {
                let text = String::from_utf8_lossy(bytes).into_owned();
                if table.json.contains(column) {
                    serde_json::from_str(&text).unwrap_or(Value::Null)
                } else {
                    json!(text)
                }
            }
            ValueRef::Blob(_) => Value::Null,
        };
        out.insert((*column).to_string(), value);
    }
    Ok(Value::Object(out))
}

fn in_list(column: &str, values: &[&str], args: &mut Vec<Sql>) -> String {
    let marks: Vec<String> = values
        .iter()
        .map(|value| {
            args.push(Sql::Text((*value).to_string()));
            format!("?{}", args.len())
        })
        .collect();
    format!("\"{column}\" IN ({})", marks.join(", "))
}

fn load(
    conn: &Connection,
    table: Table,
    workspace_id: i64,
    id: i64,
    types: &[&str],
) -> Result<Option<Value>> {
    let mut args = vec![Sql::Integer(id), Sql::Integer(workspace_id)];
    let types = in_list("type", types, &mut args);
    let sql = format!(
        r#"SELECT {} FROM "{}" WHERE "id" = ?1 AND "workspaceId" = ?2 AND {types}"#,
        select_list(table),
        table.name
    );
    conn.query_row(&sql, params_from_iter(args), |row| row_json(table, row))
        .optional()
        .map_err(repo_err)
}

/// `find…OrFail`: the row of one of `types` in the workspace, else 404 `message`.
pub async fn find_or_fail(
    db: &Db,
    table: Table,
    workspace_id: i64,
    id: i64,
    types: &'static [&'static str],
    message: &'static str,
) -> Result<Value> {
    tx::read(db, move |conn| load(conn, table, workspace_id, id, types))
        .await?
        .ok_or_else(|| BusinessError::not_found(message))
}

/// The list filters (`list`, `listIgComments`, `listIgStories`, …).
#[derive(Debug, Clone, Default)]
pub struct ListFilter {
    pub name: Option<String>,
    /// `resolveFolderIdFilter(folderId, includeAllFolders = true)`: `"0"` = root.
    pub folder_id: Option<String>,
    pub is_active: Option<bool>,
    pub inbox_id: Option<i64>,
    pub trigger: Option<String>,
    pub sort: Option<Vec<SortItem>>,
    pub page: i64,
    pub per_page: i64,
}

/// `{ data, pageCount }` of one channel's automations.
pub async fn list(
    db: &Db,
    table: Table,
    workspace_id: i64,
    types: &'static [&'static str],
    filter: ListFilter,
) -> Result<(Vec<Value>, i64)> {
    tx::read(db, move |conn| {
        let mut args = vec![Sql::Integer(workspace_id)];
        let mut clauses = vec![
            r#""workspaceId" = ?1"#.to_string(),
            in_list("type", types, &mut args),
        ];
        match filter.folder_id.as_deref() {
            Some(crate::folder::ROOT_FOLDER_ID) => clauses.push(r#""folderId" IS NULL"#.into()),
            Some(folder) if !folder.is_empty() => {
                args.push(Sql::Integer(folder.parse().unwrap_or(-1)));
                clauses.push(format!(r#""folderId" = ?{}"#, args.len()));
            }
            _ => {}
        }
        if let Some(name) = filter.name.as_deref().filter(|n| !n.is_empty()) {
            args.push(Sql::Text(like_contains(name)));
            clauses.push(format!(r#""name" LIKE ?{} ESCAPE '\'"#, args.len()));
        }
        if let Some(active) = filter.is_active {
            args.push(Sql::Integer(i64::from(active)));
            clauses.push(format!(r#""isActive" = ?{}"#, args.len()));
        }
        if let Some(inbox) = filter.inbox_id {
            args.push(Sql::Integer(inbox));
            clauses.push(format!(r#""inboxId" = ?{}"#, args.len()));
        }
        if let Some(trigger) = filter.trigger {
            args.push(Sql::Text(trigger));
            clauses.push(format!(r#""trigger" = ?{}"#, args.len()));
        }
        let filter_sql = clauses.join(" AND ");
        let total: i64 = conn
            .query_row(
                &format!(
                    r#"SELECT count(*) FROM "{}" WHERE {filter_sql}"#,
                    table.name
                ),
                params_from_iter(args.clone()),
                |r| r.get(0),
            )
            .map_err(repo_err)?;
        let order = parse_order_by(table.columns, filter.sort.as_deref(), None);
        let order_sql = if order.is_empty() {
            String::new()
        } else {
            let items: Vec<String> = order
                .iter()
                .map(|(column, dir)| format!("\"{column}\" {}", dir.sql()))
                .collect();
            format!(" ORDER BY {}", items.join(", "))
        };
        let limit = filter.per_page.max(1);
        let offset = (filter.page.max(1) - 1) * limit;
        let sql = format!(
            r#"SELECT {} FROM "{}" WHERE {filter_sql}{order_sql} LIMIT {limit} OFFSET {offset}"#,
            select_list(table),
            table.name
        );
        let mut stmt = conn.prepare(&sql).map_err(repo_err)?;
        let data = stmt
            .query_map(params_from_iter(args), |row| row_json(table, row))
            .map_err(repo_err)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(repo_err)?;
        Ok((data, (total + limit - 1) / limit))
    })
    .await
}

/// `deleteMany`: own rows of `types` only; returns the deleted ids.
pub async fn delete_many(
    db: &Db,
    table: Table,
    workspace_id: i64,
    ids: Vec<i64>,
    types: &'static [&'static str],
) -> Result<Vec<i64>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    tx::write(db, move |conn| {
        let mut args = vec![Sql::Integer(workspace_id)];
        let marks: Vec<String> = ids
            .iter()
            .map(|id| {
                args.push(Sql::Integer(*id));
                format!("?{}", args.len())
            })
            .collect();
        let types = in_list("type", types, &mut args);
        let sql = format!(
            r#"DELETE FROM "{}" WHERE "workspaceId" = ?1 AND "id" IN ({}) AND {types} RETURNING "id""#,
            table.name,
            marks.join(", ")
        );
        let mut stmt = conn.prepare(&sql).map_err(repo_err)?;
        stmt.query_map(params_from_iter(args), |r| r.get(0))
            .map_err(repo_err)?
            .collect::<std::result::Result<Vec<i64>, _>>()
            .map_err(repo_err)
    })
    .await
}

/// `delete…`: 404 first, then the delete.
pub async fn delete_one(
    db: &Db,
    table: Table,
    workspace_id: i64,
    id: i64,
    types: &'static [&'static str],
    message: &'static str,
) -> Result<()> {
    find_or_fail(db, table, workspace_id, id, types, message).await?;
    delete_many(db, table, workspace_id, vec![id], types).await?;
    Ok(())
}

/// `listAutomationAccounts` SQL per automation type.
pub fn accounts_sql(kind: &str) -> &'static str {
    match kind {
        "messenger" => {
            r#"SELECT "inboxId", "name" FROM "IntegrationMessenger" WHERE "workspaceId" = ?1"#
        }
        "instagram" => {
            r#"SELECT "inboxId", "name" FROM "IntegrationInstagram" WHERE "workspaceId" = ?1 AND "type" = 'instagram'"#
        }
        "instagramFacebook" => {
            r#"SELECT "inboxId", "name" FROM "IntegrationInstagram" WHERE "workspaceId" = ?1 AND "type" = 'facebook'"#
        }
        "threads" => {
            r#"SELECT "inboxId", "name" FROM "IntegrationThreads" WHERE "workspaceId" = ?1"#
        }
        _ => r#"SELECT "inboxId", "name" FROM "IntegrationTiktok" WHERE "workspaceId" = ?1"#,
    }
}

/// `assertAutomationAccount`: `None` / `null` / the stored scope pass.
fn assert_account(
    conn: &Connection,
    workspace_id: i64,
    kind: &str,
    inbox_id: Option<&Value>,
    current: Option<&Value>,
) -> Result<()> {
    let Some(Value::String(inbox)) = inbox_id else {
        return Ok(());
    };
    if current.and_then(Value::as_str) == Some(inbox.as_str()) {
        return Ok(());
    }
    let mut stmt = conn.prepare(accounts_sql(kind)).map_err(repo_err)?;
    let ids = stmt
        .query_map([workspace_id], |r| r.get::<_, Option<i64>>(0))
        .map_err(repo_err)?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(repo_err)?;
    if ids.iter().flatten().any(|id| id.to_string() == *inbox) {
        return Ok(());
    }
    Err(BusinessError::validation(
        "inboxId",
        "The account is not connected to this workspace for this channel",
    ))
}

/// The flow ids the replies point at (`replyFlowParents`).
fn reply_flows(data: &Map<String, Value>) -> Vec<(&'static str, String)> {
    ["privateReply", "publicReply"]
        .into_iter()
        .filter_map(|field| {
            let reply = data.get(field)?;
            (reply.get("type")? == "flow")
                .then(|| reply.get("value")?.as_str().map(|v| (field, v.to_string())))
                .flatten()
                .filter(|(_, v)| !v.is_empty())
        })
        .collect()
}

fn flow_exists(conn: &Connection, workspace_id: i64, id: &str) -> Result<bool> {
    let Ok(id) = id.parse::<i64>() else {
        return Ok(false);
    };
    Ok(flow::exists_in_workspace(conn, workspace_id, id)?)
}

/// `assertReplyFlowsExist` (`lock` = the create path's `assertReferencable`:
/// 404 instead of the 422).
fn assert_flows(
    conn: &Connection,
    workspace_id: i64,
    data: &Map<String, Value>,
    lock: bool,
) -> Result<()> {
    for (field, id) in reply_flows(data) {
        if !flow_exists(conn, workspace_id, &id)? {
            return Err(if lock {
                BusinessError::not_found("Flow not found")
            } else {
                BusinessError::validation(field, "Flow not found")
            });
        }
    }
    Ok(())
}

/// `assertModerationAgentExists`.
fn assert_moderation_agent(
    conn: &Connection,
    workspace_id: i64,
    data: &Map<String, Value>,
) -> Result<()> {
    let Some(moderation) = data.get("hideComments").and_then(|h| h.get("aiModeration")) else {
        return Ok(());
    };
    if moderation.get("enabled") != Some(&Value::Bool(true)) {
        return Ok(());
    }
    let Some(agent) = moderation.get("aiAgentId").and_then(Value::as_str) else {
        return Err(BusinessError::validation(
            "hideComments.aiModeration.aiAgentId",
            "AI agent is required",
        ));
    };
    if moderation
        .get("categories")
        .and_then(Value::as_array)
        .is_none_or(Vec::is_empty)
    {
        return Err(BusinessError::validation(
            "hideComments.aiModeration.categories",
            "Choose at least one category",
        ));
    }
    let found = agent.parse::<i64>().ok().is_some_and(|id| {
        conn.query_row(
            r#"SELECT 1 FROM "AIAgent" WHERE "id" = ?1 AND "workspaceId" = ?2"#,
            [id, workspace_id],
            |_| Ok(()),
        )
        .optional()
        .ok()
        .flatten()
        .is_some()
    });
    if !found {
        return Err(BusinessError::validation(
            "hideComments.aiModeration.aiAgentId",
            "AI agent not found",
        ));
    }
    Ok(())
}

fn sql_value(column: &str, value: &Value) -> Sql {
    match value {
        Value::Null => Sql::Null,
        Value::Bool(flag) => Sql::Integer(i64::from(*flag)),
        Value::Number(n) => n
            .as_i64()
            .map_or(Sql::Real(n.as_f64().unwrap_or(0.0)), Sql::Integer),
        Value::String(text) if ID_COLUMNS.contains(&column) => {
            Sql::Integer(text.parse().unwrap_or(-1))
        }
        Value::String(text) => Sql::Text(text.clone()),
        other => Sql::Text(other.to_string()),
    }
}

fn insert(
    conn: &Connection,
    table: Table,
    workspace_id: i64,
    kind: &str,
    data: &Map<String, Value>,
) -> Result<i64> {
    let id = new_id();
    let now = now_ms();
    let mut columns = vec!["id", "createdAt", "updatedAt", "workspaceId", "type"];
    let mut args = vec![
        Sql::Integer(id),
        Sql::Integer(now),
        Sql::Integer(now),
        Sql::Integer(workspace_id),
        Sql::Text(kind.to_string()),
    ];
    for column in table.columns {
        if columns.contains(column) {
            continue;
        }
        if let Some(value) = data.get(*column) {
            columns.push(column);
            args.push(sql_value(column, value));
        }
    }
    let marks: Vec<String> = (1..=args.len()).map(|i| format!("?{i}")).collect();
    let sql = format!(
        r#"INSERT INTO "{}" ({}) VALUES ({})"#,
        table.name,
        columns
            .iter()
            .map(|c| format!("\"{c}\""))
            .collect::<Vec<_>>()
            .join(", "),
        marks.join(", ")
    );
    conn.execute(&sql, params_from_iter(args))
        .map_err(repo_err)?;
    Ok(id)
}

/// The writable columns of an update (never `type`, `trigger`, counters, ids).
const WRITABLE: &[&str] = &[
    "name",
    "folderId",
    "inboxId",
    "isActive",
    "startTime",
    "endTime",
    "post",
    "privateReply",
    "publicReply",
    "includeKeywords",
    "excludeKeywords",
    "excludeKeywordsType",
    "options",
    "hideComments",
    "replyAfter",
    "story",
    "reply",
];

fn update_row(
    conn: &Connection,
    table: Table,
    workspace_id: i64,
    id: i64,
    data: &Map<String, Value>,
) -> Result<()> {
    let mut sets = vec![r#""updatedAt" = ?1"#.to_string()];
    let mut args = vec![Sql::Integer(now_ms())];
    for column in WRITABLE {
        if !table.columns.contains(column) {
            continue;
        }
        if let Some(value) = data.get(*column) {
            args.push(sql_value(column, value));
            sets.push(format!("\"{column}\" = ?{}", args.len()));
        }
    }
    args.push(Sql::Integer(id));
    args.push(Sql::Integer(workspace_id));
    let n = args.len();
    let sql = format!(
        r#"UPDATE "{}" SET {} WHERE "id" = ?{} AND "workspaceId" = ?{n}"#,
        table.name,
        sets.join(", "),
        n - 1
    );
    conn.execute(&sql, params_from_iter(args))
        .map_err(repo_err)?;
    Ok(())
}

// --- write normalisers ----------------------------------------------------

/// `normalizeReplyTexts` on `publicReply` (`withNormalizedReplies`).
pub fn normalize_replies(data: &mut Map<String, Value>) {
    let Some(Value::Object(reply)) = data.get_mut("publicReply") else {
        return;
    };
    if reply.get("type").and_then(Value::as_str) != Some("text") {
        return;
    }
    match reply.get("values").and_then(Value::as_array).cloned() {
        Some(values) => {
            let first = values
                .first()
                .and_then(|v| v.get("value"))
                .cloned()
                .unwrap_or_else(|| json!(""));
            reply.insert("value".into(), first);
        }
        None => {
            let value = match reply.get("value") {
                Some(Value::String(text)) => text.clone(),
                _ => String::new(),
            };
            reply.insert("values".into(), json!([{ "value": value }]));
        }
    }
}

fn supports_delete(kind: &str) -> bool {
    kind != "threads"
}
fn supports_gif(kind: &str) -> bool {
    matches!(kind, "messenger" | "threads")
}
fn supports_image_filter(kind: &str) -> bool {
    matches!(kind, "messenger" | "threads")
}

/// `withSupportedAIModeration`.
fn supported_moderation(kind: &str, hide: &mut Map<String, Value>) {
    if supports_delete(kind) {
        return;
    }
    if let Some(Value::Object(moderation)) = hide.get_mut("aiModeration")
        && moderation.get("action").and_then(Value::as_str) == Some("delete")
    {
        moderation.insert("action".into(), json!("hide"));
    }
}

/// `withSupportedHideComments` + `withSupportedImageFilter` +
/// `withLiveCapabilities` (Instagram), `withNormalizedReplies` (all).
/// `existing` = the stored row on an update.
pub fn normalize_comment_write(
    kind: &str,
    data: &mut Map<String, Value>,
    existing: Option<&Value>,
) {
    normalize_replies(data);
    if kind == "messenger" {
        return;
    }
    if let Some(Value::Object(hide)) = data.get_mut("hideComments") {
        supported_moderation(kind, hide);
        if !supports_gif(kind) {
            hide.insert("hasGif".into(), json!(false));
        }
    }
    if !supports_image_filter(kind)
        && let Some(Value::Object(options)) = data.get_mut("options")
        && options.get("onlyCommentsWithImage") == Some(&Value::Bool(true))
    {
        options.insert("onlyCommentsWithImage".into(), json!(false));
    }
    if IG_TYPES.contains(&kind) {
        live_capabilities(data, existing);
    }
}

/// `withLiveCapabilities` for Instagram (every capability off on a Live post).
fn live_capabilities(data: &mut Map<String, Value>, existing: Option<&Value>) {
    let from = |data: &Map<String, Value>, key: &str| {
        data.get(key)
            .cloned()
            .or_else(|| existing.and_then(|e| e.get(key)).cloned())
    };
    let post = from(data, "post");
    if post
        .as_ref()
        .and_then(|p| p.get("type"))
        .and_then(Value::as_str)
        != Some("live")
    {
        return;
    }
    data.insert(
        "publicReply".into(),
        json!({ "type": "none", "value": null }),
    );
    if let Some(Value::Object(mut options)) = from(data, "options") {
        options.insert("likeUserComment".into(), json!(false));
        options.insert("ignoreCommentReplies".into(), json!(false));
        data.insert("options".into(), Value::Object(options));
    }
    if let Some(Value::Object(mut hide)) = from(data, "hideComments") {
        for key in [
            "all",
            "hasPhoneNumber",
            "hasImage",
            "hasVideo",
            "hasLink",
            "hasKeywords",
            "hasGif",
            "hasEmoji",
        ] {
            hide.insert(key.into(), json!(false));
        }
        hide.insert("showCommentsAfter".into(), json!("none"));
        if let Some(Value::Object(moderation)) = hide.get_mut("aiModeration") {
            moderation.insert("enabled".into(), json!(false));
        }
        data.insert("hideComments".into(), Value::Object(hide));
    }
    data.insert(
        "replyAfter".into(),
        json!({ "type": "immediately", "value": 0 }),
    );
}

/// `buildThreadsOptions` / `buildThreadsHideComments` / the TikTok builders
/// and the create defaults of `createThreadsAutomation` /
/// `createTiktokAutomation` (`privateReply`, `excludeKeywordsType`,
/// `hideComments`, `replyAfter`). Runs before [`normalize_comment_write`].
pub fn build_channel_write(kind: &str, data: &mut Map<String, Value>, create: bool) {
    let tiktok = kind == "tiktok";
    if let Some(Value::Object(input)) = data.get("options").cloned() {
        let flag =
            |key: &str, default: bool| input.get(key).and_then(Value::as_bool).unwrap_or(default);
        let mut out = json!({
            "replyToNewContactsOnly": flag("replyToNewContactsOnly", false),
            "replyOncePerUserPerPost": flag("replyOncePerUserPerPost", false),
            "likeUserComment": tiktok && flag("likeUserComment", false),
            "replyToUsersWhoCommentedOnOtherPosts": flag("replyToUsersWhoCommentedOnOtherPosts", true),
            "ignoreCommentReplies": flag("ignoreCommentReplies", true),
            "trackUserTags": flag("trackUserTags", false),
        });
        if !tiktok {
            out["onlyCommentsWithImage"] = json!(flag("onlyCommentsWithImage", false));
        }
        data.insert("options".into(), out);
    }
    let default_hide = json!({
        "all": false, "hasPhoneNumber": false, "hasImage": false, "hasVideo": false,
        "hasLink": false, "hasKeywords": false, "hasGif": false, "hasEmoji": false,
        "keywords": [], "showCommentsAfter": "none",
    });
    match data.get("hideComments").cloned() {
        Some(Value::Object(input)) => {
            let flag = |key: &str| input.get(key).and_then(Value::as_bool).unwrap_or(false);
            let mut out = json!({
                "all": flag("all"),
                "hasPhoneNumber": flag("hasPhoneNumber"),
                "hasImage": false,
                "hasVideo": false,
                "hasLink": flag("hasLink"),
                "hasKeywords": flag("hasKeywords"),
                "hasGif": !tiktok && flag("hasGif"),
                "hasEmoji": flag("hasEmoji"),
                "keywords": input.get("keywords").cloned().unwrap_or_else(|| json!([])),
                "showCommentsAfter": input.get("showCommentsAfter").cloned().unwrap_or_else(|| json!("none")),
            });
            if let Some(moderation) = input.get("aiModeration") {
                out["aiModeration"] = moderation.clone();
            }
            data.insert("hideComments".into(), out);
        }
        _ if create => {
            data.insert("hideComments".into(), default_hide);
        }
        _ => {}
    }
    // TikTok: only `text` / `AIAgent` DMs (`buildTiktokPrivateReply`); Threads: none.
    let private = data.get("privateReply").cloned();
    let keep = tiktok
        && private
            .as_ref()
            .and_then(|r| r["type"].as_str())
            .is_some_and(|t| t == "text" || t == "AIAgent");
    if create || private.is_some() {
        let value = if keep {
            private.unwrap_or(Value::Null)
        } else {
            json!({ "type": "none", "value": null })
        };
        data.insert("privateReply".into(), value);
    }
    if create {
        data.entry("excludeKeywordsType")
            .or_insert_with(|| json!("contain"));
        data.entry("replyAfter")
            .or_insert_with(|| json!({ "type": "immediately", "value": 0 }));
    }
}

/// `toThreadsResource` / `toTiktokResource`: the stored row narrowed to what
/// the channel supports. `post` is `postIds` or `all` (no ids); `privateReply`
/// is `none` (Threads) or `text` / `AIAgent` / `none` (TikTok); a text public
/// reply keeps its `values`, a flow / AI one drops them.
pub fn channel_resource(kind: &str, mut row: Value) -> Value {
    let post_ids = row["post"]["type"] == "postIds";
    row["post"] = json!({
        "type": if post_ids { "postIds" } else { "all" },
        "value": if post_ids { row["post"]["value"].clone() } else { json!([]) },
    });
    let private_type = row["privateReply"]["type"].as_str().unwrap_or("none");
    row["privateReply"] = if kind == "tiktok" && matches!(private_type, "text" | "AIAgent") {
        json!({
            "type": private_type,
            "value": row["privateReply"]["value"].as_str().unwrap_or_default(),
        })
    } else {
        json!({ "type": "none", "value": null })
    };
    let public = row["publicReply"].clone();
    row["publicReply"] = match public["type"].as_str().unwrap_or("none") {
        "none" => json!({ "type": "none", "value": null }),
        kind => {
            let mut out =
                json!({ "type": kind, "value": public["value"].as_str().unwrap_or_default() });
            if kind == "text" && !public["values"].is_null() {
                out["values"] = public["values"].clone();
            }
            out
        }
    };
    row
}

/// `pinStoryMentionFilters`: only the filters present are pinned.
pub fn pin_story_mention(trigger: &str, data: &mut Map<String, Value>) {
    if trigger != "storyMention" {
        return;
    }
    if data.contains_key("story") {
        data.insert("story".into(), json!({ "type": "all", "value": [] }));
    }
    if data.contains_key("includeKeywords") {
        data.insert(
            "includeKeywords".into(),
            json!({ "type": "all", "value": [] }),
        );
    }
}

// --- create / update ------------------------------------------------------

/// `createMessenger` / `createInstagram` / `createThreadsAutomation` /
/// `createTiktokAutomation` (the data already passed the request schema and,
/// for Threads / TikTok, their builders).
pub async fn create_comment(
    db: &Db,
    workspace_id: i64,
    kind: &'static str,
    mut data: Map<String, Value>,
) -> Result<Value> {
    tx::write(db, move |conn| {
        // `withTemplateLockedTransaction` + `assertReferencable`: 404.
        assert_flows(conn, workspace_id, &data, true)?;
        assert_moderation_agent(conn, workspace_id, &data)?;
        assert_account(conn, workspace_id, kind, data.get("inboxId"), None)?;
        normalize_comment_write(kind, &mut data, None);
        let id = insert(conn, COMMENTS, workspace_id, kind, &data)?;
        load(conn, COMMENTS, workspace_id, id, &[kind])?
            .ok_or_else(|| BusinessError::not_found("Comment Automation not found"))
    })
    .await
}

/// `updateMessenger` / `updateInstagram` / … : 404 on another type.
pub async fn update_comment(
    db: &Db,
    workspace_id: i64,
    id: i64,
    types: &'static [&'static str],
    message: &'static str,
    mut data: Map<String, Value>,
) -> Result<Value> {
    tx::write(db, move |conn| {
        let existing = load(conn, COMMENTS, workspace_id, id, types)?
            .ok_or_else(|| BusinessError::not_found(message))?;
        let kind = existing["type"].as_str().unwrap_or("messenger").to_string();
        assert_flows(conn, workspace_id, &data, false)?;
        assert_moderation_agent(conn, workspace_id, &data)?;
        assert_account(
            conn,
            workspace_id,
            &kind,
            data.get("inboxId"),
            existing.get("inboxId"),
        )?;
        normalize_comment_write(&kind, &mut data, Some(&existing));
        update_row(conn, COMMENTS, workspace_id, id, &data)?;
        load(conn, COMMENTS, workspace_id, id, types)?
            .ok_or_else(|| BusinessError::not_found(message))
    })
    .await
}

/// `igStoryAutomationService.create`.
pub async fn create_story(
    db: &Db,
    workspace_id: i64,
    kind: &'static str,
    trigger: String,
    data: Map<String, Value>,
) -> Result<Value> {
    tx::write(db, move |conn| {
        assert_account(conn, workspace_id, kind, data.get("inboxId"), None)?;
        let mut row = Map::new();
        row.insert("story".into(), json!({ "type": "all", "value": [] }));
        row.insert(
            "includeKeywords".into(),
            json!({ "type": "all", "value": [] }),
        );
        row.extend(data);
        pin_story_mention(&trigger, &mut row);
        row.insert("trigger".into(), json!(trigger));
        let id = insert(conn, STORIES, workspace_id, kind, &row)?;
        load(conn, STORIES, workspace_id, id, IG_TYPES)?
            .ok_or_else(|| BusinessError::not_found(STORY_NOT_FOUND))
    })
    .await
}

/// "Instagram Story Automation not found".
pub const STORY_NOT_FOUND: &str = "Instagram Story Automation not found";

/// `igStoryAutomationService.update`.
pub async fn update_story(
    db: &Db,
    workspace_id: i64,
    id: i64,
    mut data: Map<String, Value>,
) -> Result<Value> {
    tx::write(db, move |conn| {
        let existing = load(conn, STORIES, workspace_id, id, IG_TYPES)?
            .ok_or_else(|| BusinessError::not_found(STORY_NOT_FOUND))?;
        let kind = existing["type"].as_str().unwrap_or("instagram");
        assert_account(
            conn,
            workspace_id,
            kind,
            data.get("inboxId"),
            existing.get("inboxId"),
        )?;
        pin_story_mention(existing["trigger"].as_str().unwrap_or_default(), &mut data);
        update_row(conn, STORIES, workspace_id, id, &data)?;
        load(conn, STORIES, workspace_id, id, IG_TYPES)?
            .ok_or_else(|| BusinessError::not_found(STORY_NOT_FOUND))
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn obj(value: Value) -> Map<String, Value> {
        value.as_object().cloned().unwrap_or_default()
    }

    #[test]
    fn public_text_reply_is_mirrored_both_ways() {
        let mut data = obj(json!({ "publicReply": { "type": "text", "value": "Hi" } }));
        normalize_replies(&mut data);
        assert_eq!(data["publicReply"]["values"], json!([{ "value": "Hi" }]));
        let mut data = obj(
            json!({ "publicReply": { "type": "text", "value": "x", "values": [{ "value": "A" }, { "value": "B" }] } }),
        );
        normalize_replies(&mut data);
        assert_eq!(data["publicReply"]["value"], json!("A"));
    }

    #[test]
    fn instagram_pins_gif_image_and_live() {
        let mut data = obj(json!({
            "post": { "type": "live", "value": [] },
            "publicReply": { "type": "text", "value": "P" },
            "options": { "likeUserComment": true, "ignoreCommentReplies": true, "onlyCommentsWithImage": true },
            "hideComments": { "all": true, "hasGif": true, "showCommentsAfter": "1d", "keywords": [] },
            "replyAfter": { "type": "seconds", "value": 30 }
        }));
        normalize_comment_write("instagram", &mut data, None);
        assert_eq!(
            data["publicReply"],
            json!({ "type": "none", "value": null })
        );
        assert_eq!(data["options"]["likeUserComment"], json!(false));
        assert_eq!(data["options"]["onlyCommentsWithImage"], json!(false));
        assert_eq!(data["hideComments"]["all"], json!(false));
        assert_eq!(data["hideComments"]["hasGif"], json!(false));
        assert_eq!(data["hideComments"]["showCommentsAfter"], json!("none"));
        assert_eq!(
            data["replyAfter"],
            json!({ "type": "immediately", "value": 0 })
        );
    }

    #[test]
    fn messenger_keeps_gif_and_threads_moderation_delete_becomes_hide() {
        let mut data = obj(json!({ "hideComments": { "hasGif": true } }));
        normalize_comment_write("messenger", &mut data, None);
        assert_eq!(data["hideComments"]["hasGif"], json!(true));
        let mut data = obj(
            json!({ "hideComments": { "hasGif": true, "aiModeration": { "action": "delete" } } }),
        );
        normalize_comment_write("threads", &mut data, None);
        assert_eq!(
            data["hideComments"]["aiModeration"]["action"],
            json!("hide")
        );
        assert_eq!(data["hideComments"]["hasGif"], json!(true));
    }

    #[test]
    fn threads_and_tiktok_builders_pin_what_the_channel_cannot_do() {
        let mut data = obj(json!({
            "options": { "likeUserComment": true, "replyToNewContactsOnly": true },
            "privateReply": { "type": "flow", "value": "1" }
        }));
        build_channel_write("threads", &mut data, true);
        assert_eq!(data["options"]["likeUserComment"], json!(false));
        assert_eq!(data["options"]["replyToNewContactsOnly"], json!(true));
        assert_eq!(
            data["privateReply"],
            json!({ "type": "none", "value": null })
        );
        assert_eq!(data["hideComments"]["hasGif"], json!(false));
        assert_eq!(data["excludeKeywordsType"], json!("contain"));
        let mut data = obj(json!({
            "options": { "likeUserComment": true },
            "hideComments": { "hasGif": true, "hasImage": true },
            "privateReply": { "type": "text", "value": "Hi" }
        }));
        build_channel_write("tiktok", &mut data, false);
        assert_eq!(data["options"]["likeUserComment"], json!(true));
        assert_eq!(data["hideComments"]["hasGif"], json!(false));
        assert_eq!(data["hideComments"]["hasImage"], json!(false));
        assert_eq!(data["privateReply"]["type"], json!("text"));
        assert!(!data.contains_key("replyAfter"));
    }

    #[test]
    fn story_mention_pins_present_filters_only() {
        let mut data = obj(json!({ "story": { "type": "storyIds", "value": ["s"] } }));
        pin_story_mention("storyMention", &mut data);
        assert_eq!(data["story"], json!({ "type": "all", "value": [] }));
        assert!(!data.contains_key("includeKeywords"));
    }

    #[test]
    fn channel_resources_narrow_the_stored_row() {
        let row = json!({
            "id": "1",
            "post": { "type": "published", "value": ["x"] },
            "privateReply": { "type": "flow", "value": "9" },
            "publicReply": { "type": "flow", "value": "9", "values": [{ "value": "a" }] },
        });
        let threads = channel_resource("threads", row.clone());
        assert_eq!(threads["post"], json!({ "type": "all", "value": [] }));
        assert_eq!(
            threads["privateReply"],
            json!({ "type": "none", "value": null })
        );
        assert_eq!(
            threads["publicReply"],
            json!({ "type": "flow", "value": "9" })
        );
        let row = json!({
            "post": { "type": "postIds", "value": ["p"] },
            "privateReply": { "type": "text", "value": null },
            "publicReply": { "type": "text", "value": "hi", "values": [{ "value": "hi" }] },
        });
        let tiktok = channel_resource("tiktok", row);
        assert_eq!(tiktok["post"], json!({ "type": "postIds", "value": ["p"] }));
        assert_eq!(
            tiktok["privateReply"],
            json!({ "type": "text", "value": "" })
        );
        assert_eq!(tiktok["publicReply"]["values"], json!([{ "value": "hi" }]));
    }
}
