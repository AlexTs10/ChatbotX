//! The public `commentAutomations.*` reads and the missed-comments run:
//! `listAutomationAccounts` (`comment-automation/account-scope.ts`),
//! `commentAutomationService` (`findOrFail`, `resolveMissedCommentsTarget`,
//! `findProcessedCommentIds`, the missed-comments lock and counter),
//! `processMissedComments` with the Messenger scan
//! (`features/shared/comment-automation/lib/missed-comments/*`) and the
//! moderation log (`lib/moderation-logs`, the analytics repository).

use chatbotx_db::Db;
use chatbotx_db::rusqlite::{
    Connection, OptionalExtension as _, params_from_iter, types::Value as Sql,
};
use chatbotx_jobs::{Enqueuer, JobOptions, NewJob, QueueName};
use chatbotx_kv::durable::DurableKv;
use serde_json::{Value, json};

use crate::connection::credentials::{Upstream, UpstreamRequest};
use crate::errors::{BusinessError, Result};
use crate::tx;
use chatbotx_db::repo::now_ms as repo_now;

fn repo_err(error: chatbotx_db::rusqlite::Error) -> BusinessError {
    BusinessError::Repo(error.into())
}

/// `MISSED_COMMENTS_SCAN_DAYS`.
pub const SCAN_DAYS: i64 = 7;
/// `MISSED_COMMENTS_MAX_PAGES`.
const MAX_PAGES: usize = 200;
/// `MISSED_COMMENTS_LOCK_SECONDS` / `..._REMAINING_GRACE_SECONDS`.
const LOCK_SECONDS: u64 = 30 * 60;
const GRACE_SECONDS: u64 = 30 * 60;
/// `MISSED_COMMENTS_REPLAY_SPACING_MS`.
const SPACING_MS: i64 = 1000;
/// `MISSED_COMMENTS_ENQUEUE_CHUNK_SIZE`.
const CHUNK: usize = 500;
/// `LOW_BULK_PRIORITY`.
const LOW_BULK_PRIORITY: u32 = 10;
const GRAPH: &str = "https://graph.facebook.com";

fn lock_key(id: i64) -> String {
    format!("comment-automation:missed-comments:{id}")
}
fn remaining_key(id: i64) -> String {
    format!("comment-automation:missed-comments:remaining:{id}")
}

/// `listAutomationAccounts`: `(inboxId, name)` of the channel's accounts.
pub async fn list_accounts(db: &Db, workspace_id: i64, kind: &str) -> Result<Vec<(i64, String)>> {
    let sql = match kind {
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
    };
    tx::read(db, move |conn| {
        let mut stmt = conn.prepare(sql).map_err(repo_err)?;
        stmt.query_map([workspace_id], |r| Ok((r.get(0)?, r.get(1)?)))
            .map_err(repo_err)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(repo_err)
    })
    .await
}

/// The columns a missed-comments run reads.
#[derive(Debug, Clone)]
pub struct Automation {
    pub id: i64,
    pub kind: String,
    pub inbox_id: Option<i64>,
    pub is_active: bool,
    pub start_time: Option<String>,
    pub end_time: Option<String>,
    pub post: Value,
}

fn load(conn: &Connection, workspace_id: i64, id: i64) -> Result<Option<Automation>> {
    conn.query_row(
        r#"SELECT "id", "type", "inboxId", "isActive", "startTime", "endTime", "post"
           FROM "CommentAutomation" WHERE "id" = ?1 AND "workspaceId" = ?2"#,
        [id, workspace_id],
        |r| {
            Ok(Automation {
                id: r.get(0)?,
                kind: r.get(1)?,
                inbox_id: r.get(2)?,
                is_active: r.get(3)?,
                start_time: r.get(4)?,
                end_time: r.get(5)?,
                post: serde_json::from_str(&r.get::<_, String>(6)?).unwrap_or(Value::Null),
            })
        },
    )
    .optional()
    .map_err(repo_err)
}

/// `findOrFail` ("Comment Automation not found").
pub async fn find_or_fail(db: &Db, workspace_id: i64, id: i64) -> Result<Automation> {
    tx::read(db, move |conn| load(conn, workspace_id, id))
        .await?
        .ok_or_else(|| BusinessError::not_found("Comment Automation not found"))
}

/// `findMissedCommentsInProgress` for one automation of the workspace.
pub async fn missed_comments_in_progress(kv: &DurableKv, id: i64) -> Result<bool> {
    let lock = lock_key(id);
    let remaining = remaining_key(id);
    let values: Vec<Option<f64>> = kv.get_all(&[&lock, &remaining]).await.map_err(kv_err)?;
    Ok(values.iter().any(Option::is_some))
}

/// The outcome of `processMissedComments`.
#[derive(Debug, Clone, PartialEq)]
pub enum RunResult {
    Done {
        scanned: usize,
        skipped: usize,
        queued: usize,
        failed: usize,
    },
    Failed(&'static str),
}

impl RunResult {
    pub fn to_json(&self) -> Value {
        match self {
            RunResult::Done {
                scanned,
                skipped,
                queued,
                failed,
            } => {
                json!({ "status": "done", "scanned": scanned, "skipped": skipped, "queued": queued, "failed": failed })
            }
            RunResult::Failed(reason) => json!({ "status": "failed", "reason": reason }),
        }
    }
}

struct Target {
    automation: Automation,
    post_id: String,
    workspace_active: bool,
}

/// `resolveMissedCommentsTarget` plus the workspace's `isActiveNow`.
fn resolve_target(
    conn: &Connection,
    workspace_id: i64,
    id: i64,
    now: i64,
) -> Result<std::result::Result<Target, &'static str>> {
    let automation = load(conn, workspace_id, id)?
        .ok_or_else(|| BusinessError::not_found("Comment automation not found"))?;
    let values = automation.post["value"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    // `canProcessMissedComments`: `postIds` with exactly one id.
    let post_id = match (automation.post["type"].as_str(), values.as_slice()) {
        (Some("postIds"), [Value::String(id)]) if !id.is_empty() => id.clone(),
        _ => return Ok(Err("notSinglePost")),
    };
    if !automation.is_active {
        return Ok(Err("inactive"));
    }
    let workspace: Option<(String, bool, Option<String>, Option<String>)> = conn
        .query_row(
            r#"SELECT "timezone", "isActive", "startTime", "endTime" FROM "Workspace" WHERE "id" = ?1"#,
            [workspace_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .optional()
        .map_err(repo_err)?;
    let (timezone, active, start, end) =
        workspace.ok_or_else(|| BusinessError::not_found("Workspace not found"))?;
    // `isWithinSchedule`: the same window rule as the workspace's.
    if !crate::workspace::is_active_now(
        true,
        automation.start_time.as_deref(),
        automation.end_time.as_deref(),
        &timezone,
        now,
    ) {
        return Ok(Err("outsideSchedule"));
    }
    let workspace_active =
        crate::workspace::is_active_now(active, start.as_deref(), end.as_deref(), &timezone, now);
    Ok(Ok(Target {
        automation,
        post_id,
        workspace_active,
    }))
}

/// One comment shaped like the channel's webhook enqueues it.
#[derive(Debug, Clone)]
struct Scanned {
    integration_identifier: String,
    comment: Value,
    comment_id: String,
    created_time: i64,
}

enum ScanError {
    IntegrationNotFound,
    Fetch(String),
}

/// `parseGraphTime`: `2025-10-17T01:37:25+0000` as epoch ms.
fn parse_graph_time(value: &str) -> Option<i64> {
    let fixed = match value.len() {
        n if n > 5
            && matches!(value.as_bytes()[n - 5], b'+' | b'-')
            && !value[n - 5..].contains(':') =>
        {
            format!("{}:{}", &value[..n - 2], &value[n - 2..])
        }
        _ => value.to_string(),
    };
    chrono::DateTime::parse_from_rfc3339(&fixed)
        .ok()
        .map(|t| t.timestamp_millis())
}

/// `scanMessengerComments` (`listPostComments`, newest first, until the
/// first comment older than `since`).
async fn scan_messenger(
    db: &Db,
    upstream: &impl Upstream,
    workspace_id: i64,
    post_id: &str,
    since: i64,
    inbox_id: Option<i64>,
) -> std::result::Result<Vec<Scanned>, ScanError> {
    let pages: Vec<(String, i64, String)> = tx::read(db, move |conn| {
        let mut stmt = conn
            .prepare(r#"SELECT "pageId", "inboxId", "auth" FROM "IntegrationMessenger" WHERE "workspaceId" = ?1 ORDER BY "id""#)
            .map_err(repo_err)?;
        stmt.query_map([workspace_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .map_err(repo_err)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(repo_err)
    })
    .await
    .map_err(|e| ScanError::Fetch(e.to_string()))?;
    let page_in_post = post_id.split_once('_').map(|(page, _)| page.to_string());
    let candidates: Vec<_> = pages
        .into_iter()
        .filter(|(_, inbox, _)| inbox_id.is_none_or(|id| id == *inbox))
        .filter(|(page, _, _)| page_in_post.as_ref().is_none_or(|p| p == page))
        .collect();
    if candidates.is_empty() {
        return Err(ScanError::IntegrationNotFound);
    }
    let story = post_id.rsplit('_').next().unwrap_or(post_id).to_string();
    let mut last_error = String::new();
    'pages: for (page_id, _, auth) in candidates {
        let auth: Value = serde_json::from_str(&auth).unwrap_or(Value::Null);
        let version = auth["version"].as_str().unwrap_or("v23.0").to_string();
        let token = auth["tokens"]["accessToken"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        let composite = format!("{page_id}_{story}");
        let mut collected = Vec::new();
        let mut after: Option<String> = None;
        for _ in 0..MAX_PAGES {
            let mut url = url::Url::parse(&format!("{GRAPH}/{version}/{composite}/comments"))
                .expect("graph url");
            url.query_pairs_mut()
                .append_pair(
                    "fields",
                    "id,message,created_time,from{id,name},parent{id},message_tags",
                )
                .append_pair("order", "reverse_chronological")
                .append_pair("filter", "stream")
                .append_pair("limit", "100");
            if let Some(after) = &after {
                url.query_pairs_mut().append_pair("after", after);
            }
            let reply = upstream
                .send(UpstreamRequest {
                    method: "GET",
                    url: url.to_string(),
                    headers: vec![("Authorization".into(), format!("Bearer {token}"))],
                    body: None,
                })
                .await;
            let body: Value = match reply {
                Ok((status, body)) if (200..300).contains(&status) => {
                    serde_json::from_str(&body).unwrap_or(Value::Null)
                }
                Ok((status, body)) => {
                    last_error = format!("Graph {status}: {body}");
                    continue 'pages;
                }
                Err(error) => {
                    last_error = error;
                    continue 'pages;
                }
            };
            let mut older = false;
            for comment in body["data"].as_array().into_iter().flatten() {
                let created = comment["created_time"]
                    .as_str()
                    .and_then(parse_graph_time)
                    .unwrap_or(i64::MIN);
                if created < since {
                    older = true;
                    break;
                }
                collected.push((comment.clone(), created));
            }
            let next = body["paging"]["next"]
                .is_string()
                .then(|| {
                    body["paging"]["cursors"]["after"]
                        .as_str()
                        .map(str::to_string)
                })
                .flatten();
            if older || next.is_none() {
                break;
            }
            after = next;
        }
        return Ok(collected
            .into_iter()
            .filter_map(|(comment, created)| {
                let from_id = comment["from"]["id"]
                    .as_str()
                    .filter(|id| !id.is_empty() && *id != page_id)?
                    .to_string();
                let comment_id = comment["id"].as_str().unwrap_or_default().to_string();
                let tags: Vec<Value> = comment["message_tags"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter(|t| t["id"].is_string())
                    .map(|t| json!({ "id": t["id"], "name": t["name"] }))
                    .collect();
                let mut data = json!({
                    "commentId": comment_id,
                    "postId": composite,
                    "parentId": comment["parent"]["id"].as_str().unwrap_or(&composite),
                    "fromId": from_id,
                    "createdTime": created.div_euclid(1000),
                });
                if let Some(name) = comment["from"]["name"].as_str() {
                    data["fromName"] = json!(name);
                }
                if let Some(message) = comment["message"].as_str() {
                    data["message"] = json!(message);
                }
                if comment["message_tags"].is_array() {
                    data["tags"] = json!(tags);
                }
                Some(Scanned {
                    integration_identifier: page_id.clone(),
                    comment: data,
                    comment_id,
                    created_time: created,
                })
            })
            .collect());
    }
    Err(ScanError::Fetch(last_error))
}

/// `findProcessedCommentIds`: an event or a miss row of this automation.
fn processed_ids(
    conn: &Connection,
    automation_id: i64,
    ids: &[String],
) -> Result<std::collections::HashSet<String>> {
    let mut out = std::collections::HashSet::new();
    for table in ["CommentAutomationEvent", "CommentAutomationMiss"] {
        for chunk in ids.chunks(500) {
            let marks = vec!["?"; chunk.len()].join(", ");
            let sql = format!(
                r#"SELECT "commentId" FROM "{table}" WHERE "automationId" = ? AND "commentId" IN ({marks})"#
            );
            let mut args = vec![Sql::Integer(automation_id)];
            args.extend(chunk.iter().map(|id| Sql::Text(id.clone())));
            let mut stmt = conn.prepare(&sql).map_err(repo_err)?;
            let rows = stmt
                .query_map(params_from_iter(args), |r| r.get::<_, String>(0))
                .map_err(repo_err)?;
            for row in rows {
                out.insert(row.map_err(repo_err)?);
            }
        }
    }
    Ok(out)
}

fn kv_err(error: impl std::fmt::Display) -> BusinessError {
    BusinessError::Repo(chatbotx_db::repo::Error::msg(error.to_string()))
}

/// `processMissedComments`. Only the Messenger scan is ported; another
/// channel's run answers `fetchFailed`.
pub async fn process_missed_comments(
    db: &Db,
    kv: &DurableKv,
    jobs: &dyn Enqueuer,
    upstream: &impl Upstream,
    workspace_id: i64,
    id: i64,
) -> Result<RunResult> {
    let now = repo_now();
    let target = match tx::read(db, move |conn| resolve_target(conn, workspace_id, id, now)).await?
    {
        Ok(target) => target,
        Err(reason) => return Ok(RunResult::Failed(reason)),
    };
    if !target.workspace_active {
        return Ok(RunResult::Failed("workspaceInactive"));
    }
    let automation = target.automation;
    // `claimMissedCommentsRun`.
    if kv
        .exists(&remaining_key(automation.id))
        .await
        .map_err(kv_err)?
        || !kv
            .set_number_if_not_exists(&lock_key(automation.id), 1.0, LOCK_SECONDS)
            .await
            .map_err(kv_err)?
    {
        return Ok(RunResult::Failed("alreadyRunning"));
    }
    let result = run_claimed(
        db,
        kv,
        jobs,
        upstream,
        workspace_id,
        &automation,
        &target.post_id,
        now,
    )
    .await;
    // `releaseMissedCommentsRun` (finally).
    let _ = kv.delete(&[&lock_key(automation.id)]).await;
    result
}

#[allow(clippy::too_many_arguments)]
async fn run_claimed(
    db: &Db,
    kv: &DurableKv,
    jobs: &dyn Enqueuer,
    upstream: &impl Upstream,
    workspace_id: i64,
    automation: &Automation,
    post_id: &str,
    now: i64,
) -> Result<RunResult> {
    let since = now - SCAN_DAYS * 86_400_000;
    let scanned = if automation.kind == "messenger" {
        scan_messenger(
            db,
            upstream,
            workspace_id,
            post_id,
            since,
            automation.inbox_id,
        )
        .await
    } else {
        // ponytail: Instagram / Threads / TikTok scans are not ported yet.
        Err(ScanError::Fetch(format!(
            "{} scan not ported",
            automation.kind
        )))
    };
    let comments = match scanned {
        Ok(comments) => comments,
        Err(ScanError::IntegrationNotFound) => return Ok(RunResult::Failed("integrationNotFound")),
        Err(ScanError::Fetch(error)) => {
            tracing::warn!(
                automation_id = automation.id,
                post_id,
                error,
                "Process missed comments: fetching the post's comments failed"
            );
            return Ok(RunResult::Failed("fetchFailed"));
        }
    };
    let ids: Vec<String> = comments.iter().map(|c| c.comment_id.clone()).collect();
    let automation_id = automation.id;
    let processed = tx::read(db, move |conn| processed_ids(conn, automation_id, &ids)).await?;
    let mut pending: Vec<&Scanned> = comments
        .iter()
        .filter(|c| !processed.contains(&c.comment_id))
        .collect();
    pending.sort_by_key(|c| c.created_time);
    let mut failed = 0;
    if !pending.is_empty() {
        // `startMissedCommentsReplay`.
        kv.set_number(
            &remaining_key(automation_id),
            pending.len() as f64,
            LOCK_SECONDS,
        )
        .await
        .map_err(kv_err)?;
        let mut last_delay = 0i64;
        let mut groups: Vec<(String, Vec<&Scanned>)> = Vec::new();
        for comment in &pending {
            match groups
                .iter_mut()
                .find(|(key, _)| *key == comment.integration_identifier)
            {
                Some((_, group)) => group.push(comment),
                None => groups.push((comment.integration_identifier.clone(), vec![comment])),
            }
        }
        for (identifier, group) in groups {
            // `reserveMissedCommentsReplayWindow`.
            let start = kv
                .reserve_time_window(
                    &format!(
                        "comment-automation:missed-comments:pace:{}:{identifier}",
                        automation.kind
                    ),
                    (group.len() as i64 * SPACING_MS) as f64,
                )
                .await
                .map_err(kv_err)?;
            let first = (start - repo_now()).max(0);
            let batch: Vec<NewJob> = group
                .iter()
                .enumerate()
                .map(|(index, comment)| {
                    let delay = first + index as i64 * SPACING_MS;
                    last_delay = last_delay.max(delay);
                    NewJob {
                        name: "replayMissedComment".into(),
                        data: json!({ "type": "replayMissedComment", "data": {
                            "workspaceId": workspace_id.to_string(),
                            "integrationType": automation.kind,
                            "integrationIdentifier": identifier,
                            "commentData": comment.comment,
                            "replay": { "automationId": automation_id.to_string() },
                        }}),
                        opts: JobOptions {
                            job_id: Some(format!(
                                "missed-comment-{automation_id}-{}",
                                comment.comment_id
                            )),
                            delay_ms: Some(delay as u64),
                            priority: Some(LOW_BULK_PRIORITY),
                            attempts: Some(1),
                            ..JobOptions::default()
                        },
                    }
                })
                .collect();
            for chunk in batch.chunks(CHUNK) {
                if let Err(error) = jobs.add_bulk(QueueName::Low, chunk.to_vec()).await {
                    failed += chunk.len();
                    tracing::error!(%error, automation_id, "Process missed comments: enqueueing a chunk of comments failed");
                }
            }
        }
        // `settleMissedCommentsEnqueue`.
        let key = remaining_key(automation_id);
        let mut settled = false;
        if failed > 0 {
            let remaining = kv
                .increment_counter(&key, -(failed as i64), None)
                .await
                .map_err(kv_err)?;
            if remaining.is_some_and(|r| r <= 0) {
                kv.delete(&[&key]).await.map_err(kv_err)?;
                settled = true;
            }
        }
        if !settled {
            let ttl = (last_delay as u64).div_ceil(1000) + GRACE_SECONDS;
            kv.expire(&key, ttl).await.map_err(kv_err)?;
        }
    }
    Ok(RunResult::Done {
        scanned: comments.len(),
        skipped: comments.len() - pending.len(),
        queued: pending.len() - failed,
        failed,
    })
}

/// The moderation-log filters (`moderationLogFilterSchema`).
#[derive(Debug, Clone, Default)]
pub struct LogFilter {
    pub from: Option<i64>,
    pub to: Option<i64>,
    pub action: Option<String>,
    pub source: Option<String>,
}

fn where_clause(workspace_id: i64, automation_id: i64, filter: &LogFilter) -> (String, Vec<Sql>) {
    let mut sql = String::from(r#"l."workspaceId" = ? AND l."automationId" = ?"#);
    let mut args = vec![Sql::Integer(workspace_id), Sql::Integer(automation_id)];
    if let Some(from) = filter.from {
        sql.push_str(r#" AND l."occurredAt" >= ?"#);
        args.push(Sql::Integer(from));
    }
    if let Some(to) = filter.to {
        sql.push_str(r#" AND l."occurredAt" <= ?"#);
        args.push(Sql::Integer(to));
    }
    if let Some(action) = &filter.action {
        sql.push_str(r#" AND l."action" = ?"#);
        args.push(Sql::Text(action.clone()));
    }
    if let Some(source) = &filter.source {
        sql.push_str(r#" AND l."source" = ?"#);
        args.push(Sql::Text(source.clone()));
    }
    (sql, args)
}

/// `listModerationLogs` (public resource: no `fullName` / `avatar`):
/// `(rows, total)`, newest comment first.
pub async fn list_moderation_logs(
    db: &Db,
    workspace_id: i64,
    automation_id: i64,
    filter: LogFilter,
    page: i64,
    per_page: i64,
) -> Result<(Vec<Value>, i64)> {
    find_or_fail(db, workspace_id, automation_id).await?;
    tx::read(db, move |conn| {
        let (clause, args) = where_clause(workspace_id, automation_id, &filter);
        let total: i64 = conn
            .query_row(
                &format!(r#"SELECT COUNT(*) FROM "CommentModerationLog" l WHERE {clause}"#),
                params_from_iter(args.clone()),
                |r| r.get(0),
            )
            .map_err(repo_err)?;
        let sql = format!(
            r#"SELECT l."id", l."commentId", l."postId", l."commentText", l."source", l."rule", l."matchedKeyword",
                      l."categories", l."reason", l."action", l."status", l."errorDetail", l."occurredAt",
                      l."contactId", l."contactInboxId",
                      (SELECT c."id" FROM "Conversation" c WHERE c."contactId" = ci."contactId" AND c."sourceId" IS NULL),
                      ci."channel", ci."sourceId", ci."id"
               FROM "CommentModerationLog" l
               LEFT JOIN "ContactInbox" ci ON ci."id" = l."contactInboxId"
                 AND EXISTS (SELECT 1 FROM "Contact" k WHERE k."id" = ci."contactId" AND k."workspaceId" = ?)
               WHERE {clause}
               ORDER BY l."occurredAt" DESC, l."id" DESC LIMIT ? OFFSET ?"#
        );
        let mut all = vec![Sql::Integer(workspace_id)];
        all.extend(args);
        all.push(Sql::Integer(per_page));
        all.push(Sql::Integer((page - 1) * per_page));
        let mut stmt = conn.prepare(&sql).map_err(repo_err)?;
        let id = |v: Option<i64>| v.map(|v| v.to_string());
        let rows = stmt
            .query_map(params_from_iter(all), |r| {
                let categories: String = r.get(7)?;
                let hydrated: Option<i64> = r.get(18)?;
                Ok(json!({
                    "id": r.get::<_, i64>(0)?.to_string(),
                    "commentId": r.get::<_, String>(1)?,
                    "postId": r.get::<_, String>(2)?,
                    "commentText": r.get::<_, Option<String>>(3)?,
                    "source": r.get::<_, String>(4)?,
                    "rule": r.get::<_, Option<String>>(5)?,
                    "matchedKeyword": r.get::<_, Option<String>>(6)?,
                    "categories": serde_json::from_str::<Value>(&categories).unwrap_or(json!([])),
                    "reason": r.get::<_, Option<String>>(8)?,
                    "action": r.get::<_, String>(9)?,
                    "status": r.get::<_, String>(10)?,
                    "errorDetail": r.get::<_, Option<String>>(11)?,
                    "occurredAt": chatbotx_db::repo::iso(r.get::<_, i64>(12)?),
                    "contactId": id(r.get(13)?),
                    "contactInboxId": id(r.get(14)?),
                    "conversationId": hydrated.and(id(r.get(15)?)),
                    "channel": hydrated.and(r.get::<_, Option<String>>(16)?),
                    "sourceId": hydrated.and(r.get::<_, Option<String>>(17)?),
                }))
            })
            .map_err(repo_err)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(repo_err)?;
        Ok((rows, total))
    })
    .await
}

/// `getModerationSummary`.
pub async fn moderation_summary(
    db: &Db,
    workspace_id: i64,
    automation_id: i64,
    filter: LogFilter,
) -> Result<Value> {
    find_or_fail(db, workspace_id, automation_id).await?;
    tx::read(db, move |conn| {
        let (clause, args) = where_clause(workspace_id, automation_id, &filter);
        let sql = format!(
            r#"SELECT l."action", l."source", l."status", COUNT(*) FROM "CommentModerationLog" l
               WHERE {clause} GROUP BY l."action", l."source", l."status""#
        );
        let mut stmt = conn.prepare(&sql).map_err(repo_err)?;
        let rows = stmt
            .query_map(params_from_iter(args), |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, i64>(3)?))
            })
            .map_err(repo_err)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(repo_err)?;
        let (mut total, mut hidden, mut deleted, mut by_rule, mut by_ai, mut failed, mut pending) = (0, 0, 0, 0, 0, 0, 0);
        for (action, source, status, count) in rows {
            total += count;
            if action == "delete" { deleted += count } else { hidden += count }
            if source == "ai" { by_ai += count } else { by_rule += count }
            match status.as_str() {
                "failed" => failed += count,
                "pending" => pending += count,
                _ => {}
            }
        }
        Ok(json!({ "total": total, "hidden": hidden, "deleted": deleted, "byRule": by_rule, "byAI": by_ai, "failed": failed, "pending": pending }))
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::parse_graph_time;

    #[test]
    fn graph_time() {
        assert_eq!(
            parse_graph_time("2025-10-17T01:37:25+0000"),
            Some(1_760_665_045_000)
        );
        assert_eq!(
            parse_graph_time("2025-10-17T01:37:25Z"),
            Some(1_760_665_045_000)
        );
        assert_eq!(parse_graph_time("nope"), None);
    }
}
