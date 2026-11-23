//! `coexistMessengerSync` (`integration/handlers/coexist/messenger-sync.ts`
//! and `messenger-helpers.ts`): the two-phase Messenger history pull. Phase
//! `contacts` walks `/conversations` newest first and imports the
//! participants; phase `messages` walks it again and imports each thread's
//! messages (90-day store window, phone / email discovery). A chunk yields
//! after 4 minutes and queues its continuation. The message phase runs up to
//! 5 conversations at once; the `X-Business-Use-Case-Usage` reading of each
//! Graph answer lowers that (5 / 3 / 1) or pauses the calls.

use std::collections::{HashMap, HashSet};

use chatbotx_business::connection::credentials::{HttpUpstream, Upstream};
use chatbotx_business::contact::extract::{ExtractOptions, extract_contact_info};
use chatbotx_db::repo::coexist_sync_run::{self as runs, Increments};
use chatbotx_db::repo::now_ms;
use chatbotx_db::repo::patch::Patch as Fields;
use chatbotx_db::rusqlite::types::Value as Sql;
use chatbotx_db::rusqlite::{OptionalExtension as _, params};
use chatbotx_jobs::options::Retention;
use chatbotx_jobs::{JobError, JobOptions, QueueName};
use serde_json::{Value, json};

use crate::channel::{InboundInbox, IncomingAttachment, IncomingContact};
use crate::coexist_throttle::{Throttle, Usage, messenger_concurrency, parse_buc, run_limited};
use crate::coexist_whatsapp::{Activity, apply_activity, import_messages, read, write};
use crate::coexist_whatsapp_payload::HistoricalMessage;
use crate::contact_scan::{self as scan, GRAPH};
use crate::{Deps, id_of};

/// `IntegrationJobAction.coexistMessengerSync`.
pub const JOB: &str = "coexistMessengerSync";
/// `CHUNK_BUDGET_MS`.
const CHUNK_BUDGET_MS: i64 = 4 * 60 * 1000;
/// `STORE_WINDOW_MS`: older messages are kept only within the first 100.
const STORE_WINDOW_MS: i64 = 90 * 24 * 60 * 60 * 1000;
/// `MAX_DISCOVERY_PAGES`: pages read past the window for phone / email.
const MAX_DISCOVERY_PAGES: u32 = 10;
/// `DEFAULT_CONCURRENCY`.
const DEFAULT_CONCURRENCY: usize = 5;
/// `MAX_PAUSE_SEC`: the longest in-process BUC pause.
const MAX_PAUSE_SECS: f64 = 300.0;
const MESSAGE_FIELDS: &str = "id,message,from,created_time,attachments{id,name,mime_type,size,image_data,video_data,file_url,generic_template}";

struct Sync<'a, U> {
    deps: &'a Deps,
    upstream: &'a U,
    graph: scan::Context,
    run_id: i64,
    ceiling: Option<i64>,
    default_country: Option<String>,
    ai_reads_synced_history: bool,
    job_start: i64,
    /// `errorRef`: the last non-fatal failure (`currentError`).
    error: Option<String>,
    throttle: Throttle,
}

impl<U> Sync<'_, U> {
    /// `applyBucThrottle`.
    fn apply_buc(&self, usage: Option<Usage>) {
        let next = messenger_concurrency(usage.as_ref());
        if next == 0 {
            let wait = usage.map_or(60.0, |u| u.regain_secs).min(MAX_PAUSE_SECS);
            self.throttle
                .pause_for(std::time::Duration::from_secs_f64(wait.max(0.0)));
            self.throttle.set_concurrency(1);
            tracing::warn!(
                wait,
                run_id = self.run_id,
                "[coexist] BUC budget exhausted — pausing Graph calls"
            );
            return;
        }
        if self.throttle.set_concurrency(next) {
            tracing::info!(
                next,
                run_id = self.run_id,
                "[coexist] BUC throttle adjusted Messenger concurrency"
            );
        }
    }
}

type Fatal = String;

fn text(value: impl Into<String>) -> Sql {
    Sql::Text(value.into())
}

fn opt_text(value: Option<&str>) -> Sql {
    value.map_or(Sql::Null, |v| Sql::Text(v.to_owned()))
}

fn opt_ms(value: Option<i64>) -> Sql {
    value.map_or(Sql::Null, Sql::Integer)
}

async fn progress<U>(s: &Sync<'_, U>, increments: Increments, fields: Fields) -> Result<(), Fatal> {
    let run_id = s.run_id;
    write(s.deps, move |c| {
        runs::increment_progress(c, run_id, &increments, &fields, None)
    })
    .await
    .map(drop)
    .map_err(|e| e.to_string())
}

async fn fail_run(deps: &Deps, run_id: i64, error: &str) -> Result<(), JobError> {
    let error = error.to_owned();
    write(deps, move |c| runs::mark_failed(c, run_id, &error, None))
        .await
        .map(drop)
}

/// The `coexistMessengerSync` integration job.
pub async fn run(deps: &Deps, data: &Value) -> Result<(), JobError> {
    run_with(deps, &HttpUpstream, data).await
}

pub(crate) async fn run_with(
    deps: &Deps,
    upstream: &impl Upstream,
    data: &Value,
) -> Result<(), JobError> {
    let (Some(run_id), Some(integration_id), Some(workspace_id)) = (
        id_of(&data["runId"]),
        id_of(&data["integrationId"]),
        id_of(&data["workspaceId"]),
    ) else {
        return Err(JobError::unrecoverable("Invalid coexistMessengerSync data"));
    };
    let job_start = now_ms();
    // Not workspace-scoped: "not found" and "workspace mismatch" differ.
    let row = read(deps, move |c| {
        Ok(c.query_row(
            r#"SELECT "workspaceId", "coexistEnabled", "auth", "pageId", "inboxId", "coexistAiReadsSyncedHistory"
               FROM "IntegrationMessenger" WHERE "id" = ?1"#,
            params![integration_id],
            |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, bool>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, i64>(4)?,
                    r.get::<_, bool>(5)?,
                ))
            },
        )
        .optional()?)
    })
    .await?;
    let Some((row_ws, enabled, auth, page_id, inbox_id, ai_reads)) = row else {
        tracing::warn!(integration_id, "[coexist] Messenger integration gone");
        return fail_run(deps, run_id, "Messenger integration not found").await;
    };
    if row_ws != workspace_id {
        tracing::warn!(
            integration_id,
            workspace_id,
            row_ws,
            "[coexist] Messenger sync workspaceId mismatch — refusing"
        );
        return fail_run(
            deps,
            run_id,
            "workspaceId mismatch between integration and run",
        )
        .await;
    }
    if !enabled {
        return fail_run(deps, run_id, "Coexist disabled on integration").await;
    }
    let auth: Value = serde_json::from_str(&auth).unwrap_or(Value::Null);
    let Some(token) = auth["tokens"]["accessToken"].as_str() else {
        return fail_run(
            deps,
            run_id,
            "Messenger auth invalid: tokens.accessToken: Required",
        )
        .await;
    };
    let version = auth["metadata"]["version"]
        .as_str()
        .unwrap_or(crate::channels::whatsapp::DEFAULT_API_VERSION)
        .to_owned();
    let found = read(deps, move |c| {
        let channel: Option<String> = c
            .query_row(
                r#"SELECT "channel" FROM "Inbox" WHERE "id" = ?1"#,
                params![inbox_id],
                |r| r.get(0),
            )
            .optional()?;
        let country: Option<String> = c
            .query_row(
                r#"SELECT "targetCountry" FROM "Workspace" WHERE "id" = ?1"#,
                params![workspace_id],
                |r| r.get(0),
            )
            .optional()?
            .flatten();
        Ok((channel, country))
    })
    .await?;
    let (Some(channel), default_country) = found else {
        return Err(JobError::retryable("Inbox not found"));
    };
    let Some(init) = read(deps, move |c| runs::find_init_state(c, run_id)).await? else {
        tracing::warn!(run_id, "[coexist] CoexistSyncRun row gone — abandoning");
        return Ok(());
    };
    let ceiling = read(deps, move |c| {
        runs::find_resume_ceiling(c, integration_id, "messenger", run_id)
    })
    .await?;
    if write(deps, move |c| runs::reclaim_run_for_retry(c, run_id, true))
        .await?
        .is_none()
    {
        tracing::warn!(
            run_id,
            "[coexist] Messenger run already claimed by another worker — abandoning"
        );
        return Ok(());
    }

    let mut s = Sync {
        deps,
        upstream,
        graph: scan::Context {
            inbox: InboundInbox {
                inbox_id,
                workspace_id,
                channel,
            },
            token: token.to_owned(),
            version,
            page_id,
        },
        run_id,
        ceiling,
        default_country,
        ai_reads_synced_history: ai_reads,
        job_start,
        error: init.current_error.clone(),
        throttle: Throttle::new(DEFAULT_CONCURRENCY),
    };
    let mut phase = init.messenger_sync_phase.clone();
    let mut last_page = 0;
    let outcome: Result<Option<&str>, Fatal> = async {
        loop {
            if now_ms() - job_start >= CHUNK_BUDGET_MS {
                return Ok(None);
            }
            if phase == "contacts" {
                let (done, page) = walk(&mut s, "contacts").await?;
                last_page = page;
                if !done {
                    return Ok(None);
                }
                let fields = vec![
                    ("messengerSyncPhase", text("messages")),
                    ("lastSyncedAt", Sql::Null),
                    ("currentStep", text("contacts done — start message phase")),
                    ("lastHeartbeatAt", Sql::Integer(now_ms())),
                ];
                progress(&s, Increments::default(), fields).await?;
                phase = "messages".into();
                continue;
            }
            let (done, page) = walk(&mut s, "messages").await?;
            last_page = page;
            if !done {
                return Ok(None);
            }
            let counters = read(deps, move |c| runs::find_terminal_counters(c, run_id))
                .await
                .map_err(|e| e.to_string())?;
            return Ok(Some(match counters {
                Some(t)
                    if t.failed_count > 0
                        && (t.imported_message_count > 0 || t.skipped_count > 0) =>
                {
                    "partial"
                }
                Some(t) if t.failed_count > 0 => "failed",
                _ => "succeeded",
            }));
        }
    }
    .await;
    let status = match outcome {
        Ok(Some(status)) => Some(status),
        Ok(None) => {
            continue_later(
                deps,
                run_id,
                integration_id,
                workspace_id,
                init.attempts,
                last_page,
            )
            .await?;
            None
        }
        Err(error) => {
            tracing::error!(run_id, %error, "[coexist] Messenger sync encountered fatal error");
            crate::events::log_failure(
                deps,
                "messenger",
                workspace_id,
                None,
                None,
                json!({ "message": error.to_string() }),
            )
            .await;
            s.error = Some(error);
            Some("failed")
        }
    };
    if let Some(status) = status {
        let now = Sql::Integer(now_ms());
        let fields = vec![
            ("status", text(status)),
            ("finishedAt", now.clone()),
            ("lastHeartbeatAt", now),
            ("currentStep", text("done")),
            ("currentError", opt_text(s.error.as_deref())),
        ];
        write(deps, move |c| {
            runs::update_progress(c, run_id, &fields, None)
        })
        .await?;
    }
    tracing::info!(run_id, ?status, %phase, "[coexist] Messenger sync chunk complete");
    Ok(())
}

/// The hot-chained next chunk; a failed enqueue reopens the run for the scheduler.
async fn continue_later(
    deps: &Deps,
    run_id: i64,
    integration_id: i64,
    workspace_id: i64,
    attempts: i64,
    last_page: i64,
) -> Result<(), JobError> {
    let added = crate::inbound::add_job(
        deps,
        QueueName::Integration,
        JOB,
        json!({ "type": JOB, "data": {
            "runId": run_id.to_string(),
            "integrationId": integration_id.to_string(),
            "workspaceId": workspace_id.to_string(),
        }}),
        JobOptions {
            job_id: Some(format!(
                "coexist-run-{run_id}-{attempts}-page-{}",
                last_page + 1
            )),
            attempts: Some(1),
            remove_on_complete: Some(Retention::Remove),
            remove_on_fail: Some(Retention::KeepCount(100)),
            ..JobOptions::default()
        },
    )
    .await;
    if let Err(error) = added {
        tracing::error!(%error, run_id, "[coexist] Messenger continuation enqueue failed — fallback to scheduler");
        let fields = vec![
            ("status", text("init")),
            ("lastHeartbeatAt", Sql::Integer(now_ms())),
        ];
        write(deps, move |c| {
            runs::update_progress(c, run_id, &fields, None)
        })
        .await?;
    }
    Ok(())
}

/// One window entry: conversation id, participant, updated time.
type Item = (String, Option<IncomingContact>, i64);

/// `walkConversationsPages` + `filterConversations`: `(done, pages read)`.
async fn walk<U: Upstream>(s: &mut Sync<'_, U>, phase: &str) -> Result<(bool, i64), Fatal> {
    let run_id = s.run_id;
    let Some(frontier) = read(s.deps, move |c| runs::find_last_synced_at(c, run_id))
        .await
        .map_err(|e| e.to_string())?
    else {
        return Ok((true, 0));
    };
    let mut oldest = frontier;
    let mut cursor: Option<String> = None;
    let mut page_number = 0;
    loop {
        if now_ms() - s.job_start >= CHUNK_BUDGET_MS {
            return Ok((false, page_number));
        }
        s.throttle.respect_pause().await;
        page_number += 1;
        let ((convs, after), headers) =
            scan::list_page_with_headers(s.upstream, &s.graph, cursor.as_deref())
                .await
                .map_err(|e| e.message)?;
        s.apply_buc(parse_buc(&headers));
        // A BUC pause holds even when this is the last page.
        s.throttle.respect_pause().await;
        let step = format!(
            "phase={phase} page {page_number} — {} conversations",
            convs.len()
        );
        let fields = vec![
            ("currentStep", text(step)),
            ("lastHeartbeatAt", Sql::Integer(now_ms())),
        ];
        progress(s, Increments::default(), fields).await?;
        let mut stop_all = false;
        let mut items: Vec<Item> = Vec::new();
        let mut window_oldest = oldest;
        for (id, contact, updated) in convs {
            let Some(updated) = updated else { continue };
            if s.ceiling.is_some_and(|c| updated <= c) {
                stop_all = true;
                break;
            }
            if frontier.is_some_and(|f| updated > f) {
                continue;
            }
            items.push((id, contact, updated));
            if window_oldest.is_none_or(|o| updated < o) {
                window_oldest = Some(updated);
            }
        }
        oldest = if phase == "contacts" {
            contacts_page(s, &items, page_number, window_oldest).await?
        } else {
            messages_page(s, &items, page_number, oldest).await?
        };
        cursor = after;
        if cursor.is_none() || stop_all {
            return Ok((true, page_number));
        }
    }
}

/// Phase 1: the participants as contacts (`bulkImportContacts`).
async fn contacts_page<U>(
    s: &mut Sync<'_, U>,
    items: &[Item],
    page_number: i64,
    window_oldest: Option<i64>,
) -> Result<Option<i64>, Fatal> {
    let mut imported = 0;
    let mut seen = HashSet::new();
    for contact in items.iter().filter_map(|i| i.1.as_ref()) {
        if !seen.insert(contact.source_id.clone()) {
            continue;
        }
        match crate::inbound::detect_contact_and_conversation(
            s.deps,
            &s.graph.inbox,
            contact,
            "inboundMessage",
        )
        .await
        {
            Ok((_, _, is_new)) => imported += i64::from(is_new),
            Err(error) => {
                tracing::error!(%error, run_id = s.run_id, page_number, "[coexist] Messenger contact bulk import threw — page lost");
                s.error = Some(format!(
                    "phase=contacts page {page_number} bulk import failed: {error}"
                ));
                break;
            }
        }
    }
    let increments = Increments {
        current_scan: Some(items.len() as i64),
        imported_contact_count: Some(imported),
        skipped_count: Some(0),
        ..Increments::default()
    };
    let fields = vec![
        ("lastSyncedAt", opt_ms(window_oldest)),
        (
            "currentStep",
            text(format!("phase=contacts page {page_number} processed")),
        ),
        ("currentError", opt_text(s.error.as_deref())),
        ("lastHeartbeatAt", Sql::Integer(now_ms())),
    ];
    progress(s, increments, fields).await?;
    Ok(window_oldest)
}

/// Phase 2: each conversation's messages into its contact's conversation.
async fn messages_page<U: Upstream>(
    s: &mut Sync<'_, U>,
    items: &[Item],
    page_number: i64,
    current_oldest: Option<i64>,
) -> Result<Option<i64>, Fatal> {
    let mut by_participant: Vec<(String, &Item)> = Vec::new();
    for item in items {
        if let Some(contact) = &item.1 {
            by_participant.retain(|(id, _)| *id != contact.source_id);
            by_participant.push((contact.source_id.clone(), item));
        }
    }
    let source_ids: Vec<String> = by_participant.iter().map(|p| p.0.clone()).collect();
    let inbox_id = s.graph.inbox.inbox_id;
    // `listContactLinksBySourceIds` (the last conversation row wins).
    let links: HashMap<String, (i64, i64, i64)> = read(s.deps, move |c| {
        let mut links = HashMap::new();
        for source in &source_ids {
            let mut stmt = c.prepare_cached(
                r#"SELECT ci."id", ci."contactId", cv."id" FROM "ContactInbox" ci
                   JOIN "Conversation" cv ON cv."contactId" = ci."contactId"
                   WHERE ci."inboxId" = ?1 AND ci."sourceId" = ?2 ORDER BY cv."id""#,
            )?;
            let rows = stmt.query_map(params![inbox_id, source], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?))
            })?;
            for row in rows {
                links.insert(source.clone(), row?);
            }
        }
        Ok(links)
    })
    .await
    .map_err(|e| e.to_string())?;

    let (mut imported, mut skipped, mut failed) = (0, 0, 0);
    let mut page_oldest = current_oldest;
    let mut activity = Vec::new();
    let shared: &Sync<'_, U> = s;
    let jobs: Vec<_> = by_participant
        .iter()
        .filter_map(|(source_id, (conv_id, _, updated))| {
            // A contact rejected in phase 1 has no link.
            let link = *links.get(source_id)?;
            Some(async move {
                let result =
                    import_conversation(shared, conv_id, link, updated - STORE_WINDOW_MS).await;
                (conv_id, *updated, result)
            })
        })
        .collect();
    let results = run_limited(jobs, shared.throttle.concurrency()).await;
    for (conv_id, updated, result) in results {
        match result {
            Ok(result) => {
                imported += result.imported;
                skipped += result.skipped;
                if result.activity.newest.is_some() || result.activity.ai_marker.is_some() {
                    activity.push(result.activity);
                }
                if page_oldest.is_none_or(|o| updated < o) {
                    page_oldest = Some(updated);
                }
            }
            Err(error) => {
                tracing::error!(%error, conversation_id = %conv_id, run_id = s.run_id, "[coexist] Messenger phase=messages conv failed");
                s.error = Some(format!("conv {conv_id} message fetch failed: {error}"));
                failed += 1;
            }
        }
    }
    let ws = s.graph.inbox.workspace_id;
    write(s.deps, move |c| apply_activity(c, ws, &activity))
        .await
        .map_err(|e| e.to_string())?;
    let increments = Increments {
        imported_message_count: Some(imported),
        skipped_count: Some(skipped),
        failed_count: Some(failed),
        ..Increments::default()
    };
    let fields = vec![
        ("lastSyncedAt", opt_ms(page_oldest)),
        (
            "currentStep",
            text(format!("phase=messages page {page_number} processed")),
        ),
        ("currentError", opt_text(s.error.as_deref())),
        ("lastHeartbeatAt", Sql::Integer(now_ms())),
    ];
    progress(s, increments, fields).await?;
    Ok(page_oldest)
}

/// One conversation's import (the original does not queue media downloads
/// for Messenger: the Graph URLs stay as they are).
struct Imported {
    imported: i64,
    skipped: i64,
    activity: Activity,
}

/// `fetchConvMessages` with its per-page `bulkImportMessages` flush.
async fn import_conversation<U: Upstream>(
    s: &Sync<'_, U>,
    conv_id: &str,
    link: (i64, i64, i64),
    cutoff: i64,
) -> Result<Imported, Fatal> {
    let mut out = Imported {
        imported: 0,
        skipped: 0,
        activity: Activity {
            contact_inbox_id: link.0,
            contact_id: link.1,
            conversation_id: link.2,
            newest: None,
            oldest: None,
            newest_incoming: None,
            ai_marker: None,
        },
    };
    let mut discovered: (Option<String>, Option<String>) = (None, None);
    let mut cursor: Option<String> = None;
    let (mut total, mut discovery_pages, mut hit_older) = (0, 0, false);
    let mut newest_id = None::<i64>;
    loop {
        let mut url = url::Url::parse(&format!("{GRAPH}/{}/{conv_id}/messages", s.graph.version))
            .map_err(|e| e.to_string())?;
        url.query_pairs_mut()
            .append_pair("fields", MESSAGE_FIELDS)
            .append_pair("limit", "499");
        if let Some(after) = &cursor {
            url.query_pairs_mut().append_pair("after", after);
        }
        s.throttle.respect_pause().await;
        let (body, headers) =
            scan::graph_get_with_headers(s.upstream, &s.graph.token, url.as_str())
                .await
                .map_err(|e| e.message)?;
        s.apply_buc(parse_buc(&headers));
        let mut page = Vec::new();
        for m in body["data"].as_array().into_iter().flatten() {
            let text_body = m["message"].as_str().filter(|t| !t.is_empty());
            let attachments = attachments_of(m);
            let template = button_template(m);
            if text_body.is_none() && attachments.is_empty() && template.is_none() {
                continue;
            }
            total += 1;
            let created_at = m["created_time"]
                .as_str()
                .and_then(chatbotx_core::js::parse_date);
            if let (Some(at), Some(ceiling)) = (created_at, s.ceiling)
                && at <= ceiling
            {
                hit_older = true;
                continue;
            }
            if text_body.is_some() && (discovered.0.is_none() || discovered.1.is_none()) {
                let found = extract_contact_info(
                    text_body,
                    s.default_country.as_deref(),
                    ExtractOptions {
                        skip_phone: discovered.0.is_some(),
                        skip_email: discovered.1.is_some(),
                    },
                );
                discovered.0 = discovered.0.take().or(found.phone_number);
                discovered.1 = discovered.1.take().or(found.email);
            }
            if created_at.is_none_or(|at| at >= cutoff) || total <= 100 {
                let outgoing = m["from"]["id"].as_str() == Some(s.graph.page_id.as_str());
                page.push(HistoricalMessage {
                    source_id: m["id"].as_str().unwrap_or_default().to_owned(),
                    message_type: if outgoing { "outgoing" } else { "incoming" },
                    text: text_body
                        .map(str::to_owned)
                        .or_else(|| template.as_ref().map(|t| t.0.clone()))
                        .unwrap_or_default(),
                    created_at,
                    attachments,
                    content_attributes: template.map(|t| t.1),
                });
            } else {
                hit_older = true;
            }
        }
        flush_page(s, link, page, &discovered, &mut out, &mut newest_id).await?;
        cursor = scan::next_cursor(&body);
        if cursor.is_none() {
            break;
        }
        if hit_older {
            discovery_pages += 1;
            if (discovered.0.is_some() && discovered.1.is_some())
                || discovery_pages >= MAX_DISCOVERY_PAGES
            {
                break;
            }
        }
    }
    out.activity.oldest = out.activity.oldest.or(out.activity.newest);
    out.activity.ai_marker = if s.ai_reads_synced_history {
        None
    } else {
        newest_id
    };
    Ok(out)
}

/// One `bulkImportMessages` call (+ `enrichIfNull` in the same transaction).
async fn flush_page<U>(
    s: &Sync<'_, U>,
    link: (i64, i64, i64),
    page: Vec<HistoricalMessage>,
    discovered: &(Option<String>, Option<String>),
    out: &mut Imported,
    newest_id: &mut Option<i64>,
) -> Result<(), Fatal> {
    let enrich = discovered.0.is_some() || discovered.1.is_some();
    if page.is_empty() && !enrich {
        return Ok(());
    }
    let times = |incoming: bool| {
        page.iter()
            .filter(move |m| !incoming || m.message_type == "incoming")
            .filter_map(|m| m.created_at)
    };
    let (newest, oldest, newest_incoming) =
        (times(false).max(), times(false).min(), times(true).max());
    let ws = s.graph.inbox.workspace_id;
    let (phone, email) = discovered.clone();
    let result = write(s.deps, move |c| {
        let result = import_messages(c, ws, link, &page)?;
        if enrich {
            chatbotx_db::repo::contact::enrich_if_null(
                c,
                link.1,
                phone.as_deref(),
                email.as_deref(),
            )?;
        }
        Ok(result)
    })
    .await
    .map_err(|e| e.to_string())?;
    out.imported += result.imported;
    out.skipped += result.skipped;
    *newest_id = (*newest_id).max(result.newest_message_id);
    let a = &mut out.activity;
    a.newest = a.newest.max(newest);
    a.oldest = match (a.oldest, oldest) {
        (Some(x), Some(y)) => Some(x.min(y)),
        (x, y) => x.or(y),
    };
    a.newest_incoming = a.newest_incoming.max(newest_incoming);
    Ok(())
}

/// `extractAttachments` / `toIncomingAttachment`.
fn attachments_of(m: &Value) -> Vec<IncomingAttachment> {
    let mut out = Vec::new();
    for raw in m["attachments"]["data"].as_array().into_iter().flatten() {
        let image = raw.get("image_data").filter(|v| v.is_object());
        let dims = image.or(raw.get("video_data").filter(|v| v.is_object()));
        let url = image
            .and_then(|i| i["url"].as_str())
            .or_else(|| raw["video_data"]["url"].as_str())
            .or_else(|| raw["file_url"].as_str());
        let Some(url) = url else { continue };
        let mime = raw["mime_type"]
            .as_str()
            .unwrap_or("application/octet-stream")
            .to_owned();
        out.push(IncomingAttachment {
            source_id: raw["id"].as_str().map(str::to_owned),
            file_type: chatbotx_files::guess_file_type_from_mime_type(&mime)
                .as_str()
                .to_owned(),
            mime_type: mime,
            origin_path: url.to_owned(),
            size: Some(raw["size"].as_i64().unwrap_or(0)),
            width: dims.and_then(|d| d["width"].as_i64()),
            height: dims.and_then(|d| d["height"].as_i64()),
            name: raw["name"].as_str().map(str::to_owned),
            is_photo_message: image.is_some(),
            optimized: false,
        });
    }
    out
}

/// `extractButtonTemplate`: the first generic template as a button card.
fn button_template(m: &Value) -> Option<(String, Value)> {
    m["attachments"]["data"]
        .as_array()?
        .iter()
        .find_map(|att| {
            let template = att.get("generic_template").filter(|t| t.is_object())?;
            let id = att["id"].as_str().unwrap_or_default();
            let buttons: Vec<Value> = template["cta"]
                .as_array()
                .into_iter()
                .flatten()
                .enumerate()
                .map(|(index, cta)| {
                    let id = format!("{id}-{index}");
                    let label = cta["title"].as_str().unwrap_or_default();
                    match cta["url"].as_str() {
                        Some(url) if cta["type"] == "web_url" => {
                            json!({ "id": id, "label": label, "buttonType": "url", "url": url })
                        }
                        _ => {
                            json!({ "id": id, "label": label, "buttonType": "postback", "postback": "" })
                        }
                    }
                })
                .collect();
            Some((
                template["title"].as_str().unwrap_or_default().to_owned(),
                json!({ "type": "template", "payload": { "templateType": "button", "buttons": buttons } }),
            ))
        })
}

#[cfg(test)]
#[path = "coexist_messenger_tests.rs"]
mod tests;
