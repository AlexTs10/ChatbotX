//! Automatic Customer Scan: the `scanContactScans` cron
//! (`schedule/handlers/scan-contact-scans.ts`) and the `contactScan`
//! integration job (`integration/handlers/contact-scan/engine.ts`) over
//! `CoexistSyncRun` rows of `type = 'contact_scan'`, with the Messenger
//! adapter (`adapters/messenger.ts`).
//!
//! ponytail: the Instagram adapter is not ported (such a run fails
//! `channelUnsupported`); the Meta usage-header throttle, quota counters and
//! profile snapshots are left out (no Rust consumer).

use chatbotx_business::connection::credentials::{HttpUpstream, Upstream, UpstreamRequest};
use chatbotx_db::repo::now_ms;
use chatbotx_db::rusqlite::{OptionalExtension as _, params};
use chatbotx_jobs::{JobError, JobOptions, QueueName};
use serde_json::{Value, json};

use crate::channel::{InboundInbox, IncomingContact};
use crate::{Deps, db_fail, id_of, str_of};

/// `IntegrationJobAction.contactScan`.
pub const CONTACT_SCAN: &str = "contactScan";
/// `CONTACT_SCAN_MAX_ATTEMPTS`.
const MAX_ATTEMPTS: i64 = 5;
/// `CONTACT_SCAN_MAX_PAGES`.
const MAX_PAGES: i64 = 5000;
/// `CHUNK_BUDGET_MS`.
const CHUNK_BUDGET_MS: i64 = 4 * 60 * 1000;
const BATCH: i64 = 500;
pub(crate) const GRAPH: &str = "https://graph.facebook.com";
/// Messenger `listConversations` page size.
const PAGE_LIMIT: &str = "499";
/// `MAX_INLINE_RETRIES`.
const MAX_INLINE_RETRIES: u32 = 4;

/// `scanContactScans`: fail runs past the attempt cap, then dispatch every
/// due run (`pickDueRuns`: idle `init` rows and stale `running` ones).
pub async fn sweep(deps: &Deps) -> Result<(), JobError> {
    let now = now_ms();
    let picked: Vec<(i64, i64, i64)> = deps
        .db
        .write(move |tx| {
            tx.execute(
                r#"UPDATE "CoexistSyncRun" SET "status" = 'failed',
                       "currentError" = 'Max scheduler retries exceeded', "finishedAt" = ?1,
                       "resumeCursor" = NULL, "updatedAt" = ?1
                   WHERE "type" = 'contact_scan' AND "attempts" >= ?2
                     AND "status" IN ('init', 'running')
                     AND ("lastHeartbeatAt" IS NULL OR "lastHeartbeatAt" < ?1 - 600000)"#,
                params![now, MAX_ATTEMPTS],
            )?;
            let mut stmt = tx.prepare(
                r#"UPDATE "CoexistSyncRun" SET "attempts" = "attempts" + 1, "status" = 'init', "updatedAt" = ?1
                   WHERE "id" IN (
                     SELECT "id" FROM "CoexistSyncRun"
                     WHERE (("status" = 'init' AND "updatedAt" < ?1 - 10000)
                            OR ("status" = 'running' AND "lastHeartbeatAt" < ?1 - 3600000))
                       AND "type" = 'contact_scan' AND "attempts" < ?2
                     ORDER BY "createdAt" ASC LIMIT ?3)
                   RETURNING "id", "attempts", "workspaceId""#,
            )?;
            let rows = stmt
                .query_map(params![now, MAX_ATTEMPTS, BATCH], |r| {
                    Ok((r.get(0)?, r.get(1)?, r.get(2)?))
                })?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        })
        .await
        .map_err(db_fail)?;
    for (run_id, attempts, workspace_id) in picked {
        let added = deps
            .jobs
            .add(
                QueueName::Integration,
                CONTACT_SCAN,
                json!({ "type": CONTACT_SCAN, "data": {
                    "runId": run_id.to_string(), "workspaceId": workspace_id.to_string() } }),
                JobOptions {
                    job_id: Some(format!("contact-scan-{run_id}-{attempts}")),
                    attempts: Some(1),
                    ..JobOptions::default()
                },
            )
            .await;
        if let Err(err) = added {
            tracing::error!(%err, run_id, "scanContactScans: enqueue failed");
        }
    }
    Ok(())
}

/// The claimed run (`claimContactScanRun`).
struct Claimed {
    workspace_id: i64,
    integration_id: i64,
    channel: String,
    attempts: i64,
    claim_token: String,
    scan_from_at: Option<i64>,
    last_synced_at: Option<i64>,
    resume_cursor: Option<String>,
    page_number: i64,
}

/// A guarded write (`runWriteFilter` with `{ status: 'running', claimToken }`):
/// `SET` clause and its arguments after `?1` (now), `?2` (run), `?3` (token).
async fn guarded(
    deps: &Deps,
    run_id: i64,
    token: &str,
    set: &'static str,
    args: Vec<chatbotx_db::rusqlite::types::Value>,
) -> Result<usize, JobError> {
    let token = token.to_owned();
    deps.db
        .write(move |tx| {
            let sql = format!(
                r#"UPDATE "CoexistSyncRun" SET {set}, "updatedAt" = ?1
                   WHERE "id" = ?2 AND "status" = 'running' AND "claimToken" = ?3"#
            );
            let mut all: Vec<chatbotx_db::rusqlite::types::Value> =
                vec![now_ms().into(), run_id.into(), token.into()];
            all.extend(args);
            Ok(tx.execute(&sql, chatbotx_db::rusqlite::params_from_iter(all))?)
        })
        .await
        .map_err(db_fail)
}

/// `finish`: a terminal status, the error and the cleared cursor.
async fn finish(
    deps: &Deps,
    run_id: i64,
    token: &str,
    status: &'static str,
    error: Option<String>,
) -> Result<usize, JobError> {
    let error = match (status, error) {
        ("failed", None) => Some("Contact scan failed".to_owned()),
        (_, error) => error,
    };
    guarded(
        deps,
        run_id,
        token,
        r#""status" = ?4, "currentError" = ?5, "finishedAt" = ?1, "resumeCursor" = NULL"#,
        vec![status.to_owned().into(), error.into()],
    )
    .await
}

/// `resetForRetry`: back to `init` for the sweeper.
async fn reset_for_retry(
    deps: &Deps,
    run_id: i64,
    token: &str,
    error: &str,
) -> Result<usize, JobError> {
    guarded(
        deps,
        run_id,
        token,
        r#""status" = 'init', "currentError" = ?4, "lastHeartbeatAt" = ?1"#,
        vec![error.to_owned().into()],
    )
    .await
}

/// The Messenger adapter context (`loadContext`).
pub(crate) struct Context {
    pub(crate) inbox: InboundInbox,
    pub(crate) token: String,
    pub(crate) version: String,
    pub(crate) page_id: String,
}

async fn load_context(
    deps: &Deps,
    workspace_id: i64,
    integration_id: i64,
) -> Result<Option<Context>, JobError> {
    deps.db
        .read(move |conn| {
            let row: Option<(String, i64, String)> = conn
                .query_row(
                    r#"SELECT "pageId", "inboxId", "auth" FROM "IntegrationMessenger"
                       WHERE "id" = ?1 AND "workspaceId" = ?2"#,
                    params![integration_id, workspace_id],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )
                .optional()?;
            let Some((page_id, inbox_id, auth)) = row else {
                return Ok(None);
            };
            let auth: Value = serde_json::from_str(&auth).unwrap_or(Value::Null);
            let Some(token) = auth["tokens"]["accessToken"].as_str() else {
                return Ok(None);
            };
            let channel: Option<String> = conn
                .query_row(
                    r#"SELECT "channel" FROM "Inbox" WHERE "id" = ?1 AND "workspaceId" = ?2"#,
                    params![inbox_id, workspace_id],
                    |r| r.get(0),
                )
                .optional()?;
            Ok(channel.map(|channel| Context {
                inbox: InboundInbox {
                    inbox_id,
                    workspace_id,
                    channel,
                },
                token: token.to_owned(),
                version: auth["metadata"]["version"]
                    .as_str()
                    .unwrap_or(crate::channels::whatsapp::DEFAULT_API_VERSION)
                    .to_owned(),
                page_id,
            }))
        })
        .await
        .map_err(db_fail)
}

/// A Graph failure: the HTTP status (`None` = no answer) and the error body.
#[derive(Debug)]
pub(crate) struct GraphError {
    status: Option<u16>,
    body: Value,
    pub(crate) message: String,
}

/// `classifyGraphSdkError`.
fn classify(error: &GraphError) -> &'static str {
    let code = error.body["error"]["code"].as_i64();
    let kind = error.body["error"]["type"].as_str();
    let retryable_status = error.status.is_none_or(|s| s == 429 || s >= 500);
    if retryable_status || code.is_some_and(|c| matches!(c, 4 | 17 | 613)) {
        return "retryable";
    }
    if kind == Some("OAuthException")
        || code.is_some_and(|c| matches!(c, 102 | 190 | 458 | 459 | 460 | 463 | 464 | 492 | 467))
    {
        return "tokenInvalid";
    }
    if code.is_some_and(|c| matches!(c, 3 | 10 | 24 | 25 | 341 | 368) || (200..=299).contains(&c)) {
        return "graphPermission";
    }
    "unknown"
}

/// One page of the walk: (conversation id, participant contact, updated
/// time) per conversation and the next cursor.
pub(crate) type Page = (
    Vec<(String, Option<IncomingContact>, Option<i64>)>,
    Option<String>,
);

/// `splitName`.
fn split_name(raw: Option<&str>) -> (Option<String>, Option<String>) {
    let Some(trimmed) = raw.map(str::trim).filter(|t| !t.is_empty()) else {
        return (None, None);
    };
    match trimmed.find(char::is_whitespace) {
        None => (Some(trimmed.to_owned()), None),
        Some(i) => {
            let last = trimmed[i..].trim();
            (
                Some(trimmed[..i].to_owned()),
                (!last.is_empty()).then(|| last.to_owned()),
            )
        }
    }
}

/// `listConversations` with `withInlineRetry`, mapped by `listPage`.
pub(crate) async fn list_page(
    upstream: &impl Upstream,
    ctx: &Context,
    cursor: Option<&str>,
) -> Result<Page, GraphError> {
    list_page_with_headers(upstream, ctx, cursor)
        .await
        .map(|(page, _)| page)
}

/// `list_page` with the response headers (the coexist BUC reading).
pub(crate) async fn list_page_with_headers(
    upstream: &impl Upstream,
    ctx: &Context,
    cursor: Option<&str>,
) -> Result<(Page, Vec<(String, String)>), GraphError> {
    let mut url = url::Url::parse(&format!(
        "{GRAPH}/{}/{}/conversations",
        ctx.version, ctx.page_id
    ))
    .map_err(|e| GraphError {
        status: Some(400),
        body: Value::Null,
        message: e.to_string(),
    })?;
    url.query_pairs_mut()
        .append_pair("fields", "id,participants,updated_time")
        .append_pair("platform", "MESSENGER")
        .append_pair("limit", PAGE_LIMIT);
    if let Some(after) = cursor {
        url.query_pairs_mut().append_pair("after", after);
    }
    let (body, headers) = graph_get_with_headers(upstream, &ctx.token, url.as_str()).await?;
    let mut entries = Vec::new();
    for conv in body["data"].as_array().into_iter().flatten() {
        let participant = conv["participants"]["data"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|p| p["id"].as_str().is_some_and(|id| id != ctx.page_id));
        let contact = participant.and_then(|p| p["id"].as_str()).map(|source_id| {
            let (first_name, last_name) = split_name(participant.and_then(|p| p["name"].as_str()));
            IncomingContact {
                source_id: source_id.to_owned(),
                first_name,
                last_name,
                ..IncomingContact::default()
            }
        });
        let updated = str_of(conv, "updated_time").and_then(chatbotx_core::js::parse_date);
        let id = str_of(conv, "id").unwrap_or_default().to_owned();
        entries.push((id, contact, updated));
    }
    Ok(((entries, next_cursor(&body)), headers))
}

/// `nextCursor`: the `after` cursor only while a `next` page exists.
pub(crate) fn next_cursor(body: &Value) -> Option<String> {
    body["paging"]["next"]
        .as_str()
        .and(body["paging"]["cursors"]["after"].as_str())
        .map(str::to_owned)
}

/// A Graph GET with `withInlineRetry` (retryable errors: 1 s, 2 s, 4 s) and
/// the response headers of the answer.
pub(crate) async fn graph_get_with_headers(
    upstream: &impl Upstream,
    token: &str,
    url: &str,
) -> Result<(Value, Vec<(String, String)>), GraphError> {
    let mut attempt = 0;
    let body = loop {
        let reply = upstream
            .send_with_headers(UpstreamRequest {
                method: "GET",
                url: url.to_owned(),
                headers: vec![("Authorization".into(), format!("Bearer {token}"))],
                body: None,
            })
            .await;
        let error = match reply {
            Ok((status, text, headers)) if (200..300).contains(&status) => {
                break (
                    serde_json::from_str::<Value>(&text).unwrap_or(Value::Null),
                    headers,
                );
            }
            Ok((status, text, _)) => {
                let body: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
                let message = body["error"]["message"]
                    .as_str()
                    .unwrap_or(&text)
                    .to_owned();
                GraphError {
                    status: Some(status),
                    body,
                    message,
                }
            }
            Err(message) => GraphError {
                status: None,
                body: Value::Null,
                message,
            },
        };
        attempt += 1;
        if classify(&error) != "retryable" || attempt >= MAX_INLINE_RETRIES {
            return Err(error);
        }
        let delay = (1000u64 << (attempt - 1)).min(30_000);
        tracing::warn!(
            attempt,
            delay,
            "[coexist] Messenger Graph rate-limited — retrying after delay"
        );
        tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
    };
    Ok(body)
}

/// The `contactScan` integration job.
pub async fn run(deps: &Deps, data: &Value) -> Result<(), JobError> {
    run_with(deps, &HttpUpstream, data).await
}

pub(crate) async fn run_with(
    deps: &Deps,
    upstream: &impl Upstream,
    data: &Value,
) -> Result<(), JobError> {
    let (Some(run_id), Some(workspace_id)) = (id_of(&data["runId"]), id_of(&data["workspaceId"]))
    else {
        return Err(JobError::unrecoverable("Invalid contactScan data"));
    };
    let job_start = now_ms();
    let token = chatbotx_db::repo::new_id().to_string();
    let claim_token = token.clone();
    let claimed = deps
        .db
        .write(move |tx| {
            let now = now_ms();
            Ok(tx
                .query_row(
                    r#"UPDATE "CoexistSyncRun" SET "status" = 'running', "claimToken" = ?2,
                           "startedAt" = COALESCE("startedAt", ?3), "lastHeartbeatAt" = ?3, "updatedAt" = ?3
                       WHERE "id" = ?1 AND "type" = 'contact_scan' AND "status" IN ('init', 'running')
                         AND ("status" != 'running' OR "claimToken" IS NULL OR "lastHeartbeatAt" < ?3 - 600000)
                       RETURNING "workspaceId", "integrationId", "channel", "attempts", "scanFromAt",
                                 "lastSyncedAt", "resumeCursor", COALESCE("currentPageNumber", 0)"#,
                    params![run_id, claim_token, now],
                    |r| {
                        Ok(Claimed {
                            workspace_id: r.get(0)?,
                            integration_id: r.get(1)?,
                            channel: r.get(2)?,
                            attempts: r.get(3)?,
                            claim_token: claim_token.clone(),
                            scan_from_at: r.get(4)?,
                            last_synced_at: r.get(5)?,
                            resume_cursor: r.get(6)?,
                            page_number: r.get(7)?,
                        })
                    },
                )
                .optional()?)
        })
        .await
        .map_err(db_fail)?;
    let Some(run) = claimed else {
        tracing::warn!(run_id, "[contact-scan] claim lost — abandoning");
        return Ok(());
    };
    if run.workspace_id != workspace_id {
        tracing::warn!(
            run_id,
            workspace_id,
            "[contact-scan] workspaceId mismatch between payload and run — refusing"
        );
        return Ok(());
    }
    let token = run.claim_token.as_str();
    if run.channel != "messenger" {
        finish(
            deps,
            run_id,
            token,
            "failed",
            Some("channelUnsupported".into()),
        )
        .await?;
        return Ok(());
    }
    let context = match load_context(deps, workspace_id, run.integration_id).await {
        Ok(Some(context)) => context,
        Ok(None) => {
            finish(
                deps,
                run_id,
                token,
                "failed",
                Some("integrationUnavailable".into()),
            )
            .await?;
            return Ok(());
        }
        Err(err) => {
            tracing::error!(err = %err, run_id, "[contact-scan] setup threw — retrying");
            reset_for_retry(deps, run_id, token, "providerRetryable").await?;
            return Ok(());
        }
    };

    let ceiling = run.scan_from_at;
    let frontier = run.last_synced_at;
    let mut cursor = run.resume_cursor.clone();
    let mut page_number = run.page_number;
    let mut oldest = frontier;
    let mut recovery = cursor.is_some();
    loop {
        if page_number >= MAX_PAGES {
            tracing::warn!(
                run_id,
                page_number,
                "[contact-scan] hit CONTACT_SCAN_MAX_PAGES — stopping as partial"
            );
            finish(
                deps,
                run_id,
                token,
                "partial",
                Some("scanLimitReached".into()),
            )
            .await?;
            return Ok(());
        }
        if now_ms() - job_start >= CHUNK_BUDGET_MS {
            // `yieldForContinuation` + the next chunk's job.
            let rows = guarded(
                deps,
                run_id,
                token,
                r#""claimToken" = NULL, "lastHeartbeatAt" = ?1"#,
                vec![],
            )
            .await?;
            if rows == 0 {
                tracing::warn!(run_id, "[contact-scan] yield lost the claim — abandoning");
                return Ok(());
            }
            let added = deps
                .jobs
                .add(
                    QueueName::Integration,
                    CONTACT_SCAN,
                    json!({ "type": CONTACT_SCAN, "data": {
                        "runId": run_id.to_string(), "workspaceId": workspace_id.to_string() } }),
                    JobOptions {
                        job_id: Some(format!(
                            "contact-scan-{run_id}-{}-page-{}",
                            run.attempts,
                            page_number + 1
                        )),
                        attempts: Some(1),
                        ..JobOptions::default()
                    },
                )
                .await;
            if let Err(err) = added {
                tracing::error!(%err, run_id, "[contact-scan] continuation enqueue failed — reopening for the sweeper");
                deps.db
                    .write(move |tx| {
                        Ok(tx.execute(
                            r#"UPDATE "CoexistSyncRun" SET "status" = 'init', "updatedAt" = ?2
                               WHERE "id" = ?1 AND "type" = 'contact_scan' AND "status" = 'running'
                                 AND "claimToken" IS NULL"#,
                            params![run_id, now_ms()],
                        )?)
                    })
                    .await
                    .map_err(db_fail)?;
            }
            return Ok(());
        }
        page_number += 1;
        let page = match list_page(upstream, &context, cursor.as_deref()).await {
            Ok(page) => page,
            Err(error) if recovery && classify(&error) != "retryable" => {
                recovery = false;
                let rows = guarded(deps, run_id, token, r#""resumeCursor" = NULL"#, vec![]).await?;
                if rows == 0 {
                    tracing::warn!(
                        run_id,
                        "[contact-scan] claim taken over mid-chunk — abandoning"
                    );
                    return Ok(());
                }
                tracing::warn!(
                    run_id,
                    error = error.message,
                    "[contact-scan] persisted resumeCursor rejected — restarting from page 1"
                );
                cursor = None;
                page_number = 0;
                continue;
            }
            Err(error) => {
                tracing::error!(run_id, error = error.message, "[contact-scan] fatal error");
                let (status, current) = match classify(&error) {
                    "retryable" => {
                        reset_for_retry(deps, run_id, token, "providerRetryable").await?;
                        return Ok(());
                    }
                    "tokenInvalid" => ("failed", "tokenInvalid".to_owned()),
                    "graphPermission" => ("failed", "graphPermission".to_owned()),
                    _ => (
                        "failed",
                        chatbotx_business::errors::sanitize_public_text(&error.message),
                    ),
                };
                finish(deps, run_id, token, status, Some(current)).await?;
                return Ok(());
            }
        };
        recovery = false;
        let (entries, after) = page;

        // `filterConversationWindow`.
        let mut stop_all = false;
        let mut items = Vec::new();
        for (_, contact, updated) in entries {
            let (Some(contact), Some(updated)) = (contact, updated) else {
                continue;
            };
            if ceiling.is_some_and(|c| updated <= c) {
                stop_all = true;
                break;
            }
            if frontier.is_some_and(|f| updated > f) {
                continue;
            }
            items.push(contact);
            if oldest.is_none_or(|o| updated < o) {
                oldest = Some(updated);
            }
        }

        // `bulkImportChannelContacts` (deduplicated by source id).
        let mut imported = 0i64;
        let mut seen = std::collections::HashSet::new();
        for contact in items.iter().filter(|c| seen.insert(c.source_id.clone())) {
            match crate::inbound::detect_contact_and_conversation(
                deps,
                &context.inbox,
                contact,
                "inboundMessage",
            )
            .await
            {
                Ok((_, _, true)) => imported += 1,
                Ok(_) => {}
                Err(err) => {
                    tracing::error!(err = %err, run_id, page_number, "[contact-scan] bulk import threw — retrying this page");
                    if reset_for_retry(deps, run_id, token, "pageImportFailed").await? == 0 {
                        tracing::warn!(
                            run_id,
                            "[contact-scan] retry reset lost the claim — abandoning"
                        );
                    }
                    return Ok(());
                }
            }
        }

        let step = format!("page {page_number} processed ({} in window)", items.len());
        let rows = guarded(
            deps,
            run_id,
            token,
            r#""currentScan" = "currentScan" + ?4, "importedContactCount" = "importedContactCount" + ?5,
               "lastSyncedAt" = ?6, "resumeCursor" = ?7, "currentPageNumber" = ?8, "currentStep" = ?9,
               "currentError" = NULL"#,
            vec![
                (items.len() as i64).into(),
                imported.into(),
                oldest.into(),
                after.clone().into(),
                page_number.into(),
                step.into(),
            ],
        )
        .await?;
        if rows == 0 {
            tracing::warn!(
                run_id,
                "[contact-scan] claim taken over mid-chunk — abandoning"
            );
            return Ok(());
        }
        cursor = after;
        if cursor.is_none() || stop_all {
            break;
        }
    }
    finish(deps, run_id, token, "succeeded", None).await?;
    tracing::info!(run_id, status = "succeeded", "[contact-scan] run complete");
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests {
    use std::sync::{Arc, Mutex};

    use chatbotx_business::connection::credentials::UpstreamReply;
    use chatbotx_db::Db;
    use chatbotx_jobs::{AddResult, JobsError, NewJob};

    use super::*;

    #[derive(Default)]
    pub(crate) struct Jobs(pub Mutex<Vec<(String, JobOptions)>>);

    impl chatbotx_jobs::Enqueuer for Jobs {
        fn add(
            &self,
            _: QueueName,
            name: &str,
            _: Value,
            opts: JobOptions,
        ) -> chatbotx_jobs::BoxFuture<'_, Result<AddResult, JobsError>> {
            self.0.lock().unwrap().push((name.to_owned(), opts));
            Box::pin(async {
                Ok(AddResult {
                    job_id: "1".into(),
                    created: true,
                })
            })
        }

        fn add_bulk(
            &self,
            _: QueueName,
            _: Vec<NewJob>,
        ) -> chatbotx_jobs::BoxFuture<'_, Result<Vec<AddResult>, JobsError>> {
            Box::pin(async { Ok(Vec::new()) })
        }
    }

    /// Answers every Graph call with one body; records `METHOD url body`.
    pub(crate) struct Graph(pub Mutex<Vec<String>>, pub Value);

    impl Upstream for Graph {
        fn send(
            &self,
            request: UpstreamRequest,
        ) -> impl std::future::Future<Output = UpstreamReply> + Send {
            self.0.lock().unwrap().push(format!(
                "{} {} {}",
                request.method,
                request.url,
                request.body.unwrap_or_default()
            ));
            let body = self.1.to_string();
            async move { Ok((200, body)) }
        }
    }

    pub(crate) async fn setup() -> (Deps, Arc<Jobs>) {
        let dir = std::env::temp_dir().join(format!(
            "chatbotx-worker-scan-{}-{}",
            std::process::id(),
            chatbotx_db::repo::new_id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let db = Arc::new(Db::open(dir.join("t.db")).unwrap());
        db.migrate().await.unwrap();
        let now = now_ms();
        db.write(move |tx| {
            for sql in [
                r#"INSERT INTO "User" ("id", "email") VALUES (1, 'u@example.test')"#,
                r#"INSERT INTO "Workspace" ("id", "name", "ownerId") VALUES (2, 'WS', 1)"#,
                r#"INSERT INTO "Inbox" ("id", "workspaceId", "name", "channel", "sourceId") VALUES (4, 2, 'Page', 'messenger', 'p1')"#,
                r#"INSERT INTO "IntegrationMessenger" ("id", "auth", "pageId", "name", "conversationStarters", "persistentMenus", "personas", "workspaceId", "inboxId")
                   VALUES (5, '{"tokens":{"accessToken":"tok"}}', 'p1', 'Page', '[]', '[]', '[]', 2, 4)"#,
            ] {
                tx.execute(sql, [])?;
            }
            tx.execute(
                r#"INSERT INTO "CoexistSyncRun" ("id", "workspaceId", "integrationId", "channel", "type", "status", "triggerSource", "scanFromAt", "updatedAt")
                   VALUES (9, 2, 5, 'messenger', 'contact_scan', 'init', 'contact-scan-manual', ?1, ?2)"#,
                params![now - 7 * 86_400_000, now - 60_000],
            )?;
            Ok(())
        })
        .await
        .unwrap();
        let jobs = Arc::new(Jobs::default());
        let deps = Deps {
            db,
            jobs: jobs.clone(),
            waiter: chatbotx_jobs::wait::CompletionRegistry::new(),
            realtime: None,
            channels: crate::Channels::builtin(),
            app_url: "http://app.test".into(),
            variables: None,
            ai: Arc::new(crate::ai::Ai::live()),
            files: None,
            webhook_http: Arc::new(crate::webhook::RecordingWebhookHttp::default()),
            events: None,
            push: None,
        };
        (deps, jobs)
    }

    #[tokio::test]
    async fn the_sweeper_dispatches_and_the_run_imports_the_customers() {
        let (deps, jobs) = setup().await;
        sweep(&deps).await.unwrap();
        let queued = jobs.0.lock().unwrap().clone();
        assert_eq!(queued.len(), 1);
        assert_eq!(queued[0].0, CONTACT_SCAN);
        assert_eq!(queued[0].1.job_id.as_deref(), Some("contact-scan-9-1"));

        let recent = chatbotx_db::repo::iso(now_ms() - 3_600_000);
        let old = chatbotx_db::repo::iso(now_ms() - 8 * 86_400_000);
        let graph = Graph(
            Mutex::default(),
            json!({ "data": [
                { "id": "t1", "updated_time": recent, "participants": { "data": [
                    { "id": "p1", "name": "Page" }, { "id": "u1", "name": "Ana Lima" } ] } },
                { "id": "t2", "updated_time": old, "participants": { "data": [
                    { "id": "u2", "name": "Old Customer" } ] } },
            ], "paging": { "cursors": { "after": "c2" }, "next": "https://graph/next" } }),
        );
        run_with(&deps, &graph, &json!({ "runId": "9", "workspaceId": "2" }))
            .await
            .unwrap();
        let urls = graph.0.lock().unwrap().clone();
        assert_eq!(urls.len(), 1, "the ceiling stops the walk");
        assert!(urls[0].contains("/p1/conversations?") && urls[0].contains("platform=MESSENGER"));
        let (status, imported, scanned, first): (String, i64, i64, String) = deps
            .db
            .read(|c| {
                Ok(c.query_row(
                    r#"SELECT r."status", r."importedContactCount", r."currentScan",
                              (SELECT "firstName" FROM "Contact" WHERE "workspaceId" = 2)
                       FROM "CoexistSyncRun" r WHERE r."id" = 9"#,
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
                )?)
            })
            .await
            .unwrap();
        assert_eq!(
            (status.as_str(), imported, scanned, first.as_str()),
            ("succeeded", 1, 1, "Ana")
        );
    }

    #[test]
    fn graph_errors_are_classified_as_the_original() {
        let err = |status: Option<u16>, body: Value| GraphError {
            status,
            body,
            message: String::new(),
        };
        assert_eq!(classify(&err(None, Value::Null)), "retryable");
        assert_eq!(classify(&err(Some(503), Value::Null)), "retryable");
        assert_eq!(
            classify(&err(Some(400), json!({"error": {"code": 17}}))),
            "retryable"
        );
        assert_eq!(
            classify(&err(Some(400), json!({"error": {"code": 190}}))),
            "tokenInvalid"
        );
        assert_eq!(
            classify(&err(
                Some(400),
                json!({"error": {"type": "OAuthException", "code": 1}})
            )),
            "tokenInvalid"
        );
        assert_eq!(
            classify(&err(Some(403), json!({"error": {"code": 230}}))),
            "graphPermission"
        );
        assert_eq!(
            classify(&err(Some(400), json!({"error": {"code": 100}}))),
            "unknown"
        );
    }

    #[test]
    fn split_name_keeps_the_rest_as_the_last_name() {
        assert_eq!(
            split_name(Some("  Ana  Maria Lima ")),
            (Some("Ana".into()), Some("Maria Lima".into()))
        );
        assert_eq!(split_name(Some("Ana")), (Some("Ana".into()), None));
        assert_eq!(split_name(Some("  ")), (None, None));
    }
}
