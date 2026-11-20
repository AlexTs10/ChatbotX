//! `coexistInstagramSync` (`integration/handlers/coexist/instagram-sync.ts`,
//! `instagram-adapter.ts`, `instagram-facebook-adapter.ts`,
//! `instagram-normalize.ts`, `graph-conversation-sync.ts`): the Instagram
//! history pull. One walk over the conversations, newest first; each
//! conversation's participant becomes a contact and its messages are
//! imported (90-day store window after the first 100). Native Instagram
//! Login reads `graph.instagram.com/{igId}/conversations` and resolves the
//! customer's name from the user node; Instagram via Facebook reads
//! `graph.facebook.com/{pageId}/conversations?platform=instagram` with the
//! Page token. Up to 3 conversations run at once; the `X-App-Usage` reading
//! lowers that or pauses the calls. A chunk yields after 4 minutes.
//!
//! ponytail: the profile snapshot capture (`enqueueProfileSnapshotJobs`) is
//! not queued — no Rust worker consumes that queue yet. Graph GETs use the
//! shared `withInlineRetry` (`contact_scan::graph_get_with_headers`); after
//! it, a failed conversation read counts as failed, a failed page read fails
//! the run.

use std::time::Duration;

use chatbotx_business::connection::credentials::{HttpUpstream, Upstream};
use chatbotx_business::contact::extract::{ExtractOptions, extract_contact_info};
use chatbotx_db::repo::coexist_sync_run::{self as runs, CoexistIntegrationRow};
use chatbotx_db::repo::now_ms;
use chatbotx_db::rusqlite::types::Value as Sql;
use chatbotx_db::rusqlite::{OptionalExtension as _, params};
use chatbotx_jobs::options::Retention;
use chatbotx_jobs::{JobError, JobOptions, QueueName};
use serde_json::{Value, json};

use crate::channel::{InboundInbox, IncomingAttachment, IncomingContact};
use crate::coexist_throttle::{Throttle, instagram_decision, parse_app_usage, run_limited};
use crate::coexist_whatsapp::{Activity, apply_activity, import_messages, read, write};
use crate::coexist_whatsapp_payload::HistoricalMessage;
use crate::contact_scan::{self as scan, GraphError};
use crate::{Deps, id_of};

/// `IntegrationJobAction.coexistInstagramSync`.
pub const JOB: &str = "coexistInstagramSync";
const DEFAULT_CONCURRENCY: usize = 3;
const CHUNK_BUDGET_MS: i64 = 4 * 60 * 1000;
const MAX_USAGE_PAUSE: Duration = Duration::from_secs(45);
const STORE_WINDOW_MS: i64 = 90 * 24 * 60 * 60 * 1000;
/// `PAGE_LIMIT` / `DETAIL_FETCH_LIMIT` of `graph-conversation-sync.ts`.
const PAGE_LIMIT: &str = "100";
const DETAIL_FETCH_LIMIT: usize = 20;
const MESSAGE_FIELDS: &str = "id,created_time,from,to,message,attachments,is_unsupported";
const NOT_FOUND: &str = "Instagram integration not found or coexist disabled";

/// The provider context (`InstagramCoexistContext` /
/// `InstagramFacebookCoexistContext`).
struct Context {
    /// `type: "facebook"` (Instagram via a Facebook Page).
    facebook: bool,
    inbox: InboundInbox,
    token: String,
    version: String,
    ig_id: String,
    /// The conversations node: `igId` or the Page id.
    node: String,
    default_country: Option<String>,
    ai_reads_synced_history: bool,
}

impl Context {
    fn base(&self) -> &'static str {
        if self.facebook {
            "https://graph.facebook.com"
        } else {
            "https://graph.instagram.com"
        }
    }
}

/// `loadContext`: `Ok(None)` = not found, another type, or coexist off.
async fn load_context(
    deps: &Deps,
    workspace_id: i64,
    integration_id: i64,
    facebook: bool,
) -> Result<Option<Context>, JobError> {
    let row = read(deps, move |c| {
        runs::find_integration_for_coexist(c, workspace_id, integration_id, "instagram")
    })
    .await?;
    let Some(CoexistIntegrationRow::Instagram(row)) = row else {
        return Ok(None);
    };
    if (row.r#type == "facebook") != facebook || !row.coexist_enabled {
        return Ok(None);
    }
    let auth = &row.auth.0;
    let label = if facebook {
        "Instagram (Facebook)"
    } else {
        "Instagram"
    };
    let invalid =
        |path: &str| JobError::retryable(format!("{label} auth invalid: {path}: Required"));
    let token = auth["tokens"]["accessToken"]
        .as_str()
        .ok_or_else(|| invalid("tokens.accessToken"))?;
    let ig_id = auth["metadata"]["igId"]
        .as_str()
        .ok_or_else(|| invalid("metadata.igId"))?;
    let node = if facebook {
        auth["metadata"]["pageId"]
            .as_str()
            .ok_or_else(|| invalid("metadata.pageId"))?
    } else {
        ig_id
    };
    let version =
        auth["metadata"]["version"]
            .as_str()
            .unwrap_or(if facebook { "v23.0" } else { "v22.0" });
    let inbox_id = row.inbox_id;
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
    Ok(Some(Context {
        facebook,
        inbox: InboundInbox {
            inbox_id,
            workspace_id,
            channel,
        },
        token: token.to_owned(),
        version: version.to_owned(),
        ig_id: ig_id.to_owned(),
        node: node.to_owned(),
        default_country,
        ai_reads_synced_history: row.coexist_ai_reads_synced_history,
    }))
}

async fn mark(
    deps: &Deps,
    run_id: i64,
    status: &'static str,
    error: Option<String>,
) -> Result<(), JobError> {
    write(deps, move |c| match status {
        "failed" => runs::mark_failed(c, run_id, error.as_deref().unwrap_or_default(), None),
        "partial" => runs::mark_partial(c, run_id, error.as_deref(), None),
        _ => runs::mark_succeeded(c, run_id, None),
    })
    .await
    .map(drop)
}

/// The `coexistInstagramSync` integration job.
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
        return Err(JobError::unrecoverable("Invalid coexistInstagramSync data"));
    };
    let job_start = now_ms();
    let row = read(deps, move |c| {
        runs::find_integration_for_coexist(c, workspace_id, integration_id, "instagram")
    })
    .await?;
    let Some(CoexistIntegrationRow::Instagram(row)) = row else {
        return mark(deps, run_id, "failed", Some(NOT_FOUND.into())).await;
    };
    let facebook = row.r#type == "facebook";
    if load_context(deps, workspace_id, integration_id, facebook)
        .await?
        .is_none()
    {
        return mark(deps, run_id, "failed", Some(NOT_FOUND.into())).await;
    }
    let Some(claimed) = write(deps, move |c| {
        runs::claim_run_with_new_token(c, run_id, None)
    })
    .await?
    else {
        tracing::warn!(
            run_id,
            integration_id,
            "[coexist] Instagram run already claimed by another worker"
        );
        return Ok(());
    };
    let Some(ctx) = load_context(deps, workspace_id, integration_id, facebook).await? else {
        return mark(
            deps,
            run_id,
            "failed",
            Some(format!("{NOT_FOUND} after claim")),
        )
        .await;
    };
    let ceiling = read(deps, move |c| {
        runs::find_resume_ceiling(c, integration_id, "instagram", run_id)
    })
    .await?;
    let s = Pull {
        upstream,
        ctx,
        throttle: Throttle::new(DEFAULT_CONCURRENCY),
        run_id,
        cutoff: now_ms() - STORE_WINDOW_MS,
        frontier: claimed.last_synced_at,
        ceiling,
    };
    let mut totals = [
        claimed.imported_contact_count,
        claimed.imported_message_count,
        claimed.skipped_count,
        claimed.failed_count,
    ];
    let mut page_number = claimed.current_page_number;
    let outcome = walk(deps, &s, job_start, &mut page_number, &mut totals).await;
    let result = match outcome {
        Ok((true, error)) => continue_later(
            deps,
            &s,
            integration_id,
            workspace_id,
            claimed.attempts,
            page_number,
        )
        .await
        .map(|()| None)
        .map_err(|e| (e, error)),
        Ok((false, error)) => Ok(Some(error)),
        Err(fatal) => Err((fatal, None)),
    };
    match result {
        Ok(None) => Ok(()),
        Ok(Some(error)) => {
            let counters = read(deps, move |c| runs::find_terminal_counters(c, run_id)).await?;
            match counters {
                Some(t)
                    if t.failed_count > 0
                        && (t.imported_message_count > 0 || t.skipped_count > 0) =>
                {
                    mark(deps, run_id, "partial", error).await
                }
                Some(t) if t.failed_count > 0 => {
                    mark(
                        deps,
                        run_id,
                        "failed",
                        Some(error.unwrap_or_else(|| "Instagram sync failed".into())),
                    )
                    .await
                }
                _ => mark(deps, run_id, "succeeded", None).await,
            }
        }
        Err((fatal, _)) => {
            tracing::error!(run_id, error = %fatal, "[coexist] Instagram sync encountered fatal error");
            mark(deps, run_id, "failed", Some(fatal.clone())).await?;
            crate::events::log_provider_errors(
                deps.events.as_ref(),
                &[crate::events::ProviderError {
                    provider: "instagram".into(),
                    workspace_id: workspace_id.to_string(),
                    error: json!({ "message": fatal }),
                    ..Default::default()
                }],
            )
            .await;
            Ok(())
        }
    }
}

struct Pull<'a, U> {
    upstream: &'a U,
    ctx: Context,
    throttle: Throttle,
    run_id: i64,
    cutoff: i64,
    frontier: Option<i64>,
    ceiling: Option<i64>,
}

impl<U: Upstream> Pull<'_, U> {
    /// `applyUsageThrottle`.
    fn apply_usage(&self, headers: &[(String, String)]) {
        let usage = parse_app_usage(headers);
        let (concurrency, pause) =
            instagram_decision(usage.as_ref(), DEFAULT_CONCURRENCY, MAX_USAGE_PAUSE);
        if !pause.is_zero() {
            self.throttle.pause_for(pause);
            tracing::warn!(
                run_id = self.run_id,
                ?usage,
                ?pause,
                "[coexist] Instagram usage budget exhausted; pausing Graph calls"
            );
        }
        if self.throttle.set_concurrency(concurrency.max(1)) {
            tracing::info!(
                run_id = self.run_id,
                concurrency = concurrency.max(1),
                "[coexist] Instagram usage throttle adjusted concurrency"
            );
        }
    }

    /// A Graph GET whose usage reading feeds the throttle.
    async fn get(&self, path: &str, query: &[(&str, &str)]) -> Result<Value, String> {
        let (body, headers) = self.get_raw(path, query).await?;
        self.apply_usage(&headers);
        Ok(body)
    }

    async fn get_raw(
        &self,
        path: &str,
        query: &[(&str, &str)],
    ) -> Result<(Value, Vec<(String, String)>), String> {
        let mut url = url::Url::parse(&format!("{}/{}/{path}", self.ctx.base(), self.ctx.version))
            .map_err(|e| e.to_string())?;
        url.query_pairs_mut().extend_pairs(query);
        scan::graph_get_with_headers(self.upstream, &self.ctx.token, url.as_str())
            .await
            .map_err(|e: GraphError| e.message)
    }

    /// `fetchConversationMessages` with `hydrateMissingDetails`.
    async fn messages(
        &self,
        conversation_id: &str,
        after: Option<&str>,
    ) -> Result<(Vec<Value>, Option<String>), String> {
        let edge = match after {
            Some(after) => format!("messages.after({after}){{{MESSAGE_FIELDS}}}"),
            None => format!("messages{{{MESSAGE_FIELDS}}}"),
        };
        let body = self.get(conversation_id, &[("fields", &edge)]).await?;
        let mut messages: Vec<Value> = body["messages"]["data"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        let missing: Vec<usize> = messages
            .iter()
            .enumerate()
            .filter(|(_, m)| !has_inline_details(m))
            .map(|(i, _)| i)
            .take(DETAIL_FETCH_LIMIT)
            .collect();
        for index in missing {
            let Some(id) = messages[index]["id"].as_str().map(str::to_owned) else {
                continue;
            };
            // Detail reads only reach the 20 newest messages; a miss keeps the ref.
            if let Ok((detail, _)) = self.get_raw(&id, &[("fields", MESSAGE_FIELDS)]).await
                && detail["id"].is_string()
            {
                messages[index] = detail;
            }
        }
        Ok((messages, scan::next_cursor(&body["messages"])))
    }
}

fn has_inline_details(m: &Value) -> bool {
    ["created_time", "from", "to", "message", "attachments"]
        .iter()
        .any(|k| !m[*k].is_null())
}

/// One conversation's result.
#[derive(Default)]
struct ConvResult {
    contacts: i64,
    messages: i64,
    skipped: i64,
    failed: bool,
    error: Option<String>,
    activity: Vec<Activity>,
    updated: Option<i64>,
}

/// The conversation pages; `Ok((continue_later, current_error))`.
async fn walk<U: Upstream>(
    deps: &Deps,
    s: &Pull<'_, U>,
    job_start: i64,
    page_number: &mut i64,
    totals: &mut [i64; 4],
) -> Result<(bool, Option<String>), String> {
    let mut cursor: Option<String> = None;
    let mut current_error: Option<String> = None;
    loop {
        if now_ms() - job_start >= CHUNK_BUDGET_MS {
            return Ok((true, current_error));
        }
        s.throttle.respect_pause().await;
        let mut query = vec![
            ("fields", "id,participants,updated_time"),
            ("limit", PAGE_LIMIT),
        ];
        if s.ctx.facebook {
            query.push(("platform", "instagram"));
        }
        if let Some(after) = cursor.as_deref() {
            query.push(("after", after));
        }
        let page = s
            .get(&format!("{}/conversations", s.ctx.node), &query)
            .await?;
        *page_number += 1;
        let conversations: Vec<Value> = page["data"].as_array().cloned().unwrap_or_default();
        let jobs: Vec<_> = conversations
            .iter()
            .map(|c| conversation(deps, s, c))
            .collect();
        let results = run_limited(jobs, s.throttle.concurrency()).await;
        let mut page_oldest: Option<i64> = None;
        let mut activity = Vec::new();
        let mut page_totals = [0i64; 4];
        for result in results {
            page_totals[0] += result.contacts;
            page_totals[1] += result.messages;
            page_totals[2] += result.skipped;
            page_totals[3] += i64::from(result.failed);
            if result.error.is_some() {
                current_error = result.error;
            }
            if let Some(updated) = result.updated
                && page_oldest.is_none_or(|o| updated < o)
            {
                page_oldest = Some(updated);
            }
            activity.extend(result.activity);
        }
        let ws = s.ctx.inbox.workspace_id;
        write(deps, move |c| apply_activity(c, ws, &activity))
            .await
            .map_err(|e| e.to_string())?;
        for (total, add) in totals.iter_mut().zip(page_totals) {
            *total += add;
        }
        let now = Sql::Integer(now_ms());
        let mut fields = vec![
            ("importedContactCount", Sql::Integer(totals[0])),
            ("importedMessageCount", Sql::Integer(totals[1])),
            ("skippedCount", Sql::Integer(totals[2])),
            ("failedCount", Sql::Integer(totals[3])),
            ("currentPageNumber", Sql::Integer(*page_number)),
            (
                "currentStep",
                Sql::Text(format!("instagram page {page_number} processed")),
            ),
            (
                "currentError",
                current_error.clone().map_or(Sql::Null, Sql::Text),
            ),
            ("lastHeartbeatAt", now),
        ];
        // Only a page that processed a conversation moves the watermark.
        if let Some(oldest) = page_oldest {
            fields.push(("lastSyncedAt", Sql::Integer(oldest)));
        }
        let run_id = s.run_id;
        write(deps, move |c| {
            runs::update_progress(c, run_id, &fields, None)
        })
        .await
        .map_err(|e| e.to_string())?;
        cursor = scan::next_cursor(&page);
        if cursor.is_none() {
            return Ok((false, current_error));
        }
    }
}

/// One conversation: contact, then its message pages.
async fn conversation<U: Upstream>(deps: &Deps, s: &Pull<'_, U>, conv: &Value) -> ConvResult {
    let mut out = ConvResult::default();
    s.throttle.respect_pause().await;
    let updated = conv["updated_time"]
        .as_str()
        .and_then(chatbotx_core::js::parse_date);
    if let Some(updated) = updated
        && (s.frontier.is_some_and(|f| updated >= f) || s.ceiling.is_some_and(|c| updated <= c))
    {
        return out;
    }
    let conv_id = conv["id"].as_str().unwrap_or_default();
    match import_conversation(deps, s, conv, conv_id, &mut out).await {
        Ok(true) => {
            out.updated = updated;
        }
        Ok(false) => {}
        Err(error) => {
            tracing::error!(%error, run_id = s.run_id, conversation_id = conv_id, "[coexist] Instagram conversation sync failed");
            out.failed = true;
            out.error = Some(error);
        }
    }
    out
}

/// `Ok(false)` = skipped (no customer, or the contact was not imported).
async fn import_conversation<U: Upstream>(
    deps: &Deps,
    s: &Pull<'_, U>,
    conv: &Value,
    conv_id: &str,
    out: &mut ConvResult,
) -> Result<bool, String> {
    let participants: Vec<Value> = conv["participants"]["data"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let mut cursor: Option<String> = None;
    let mut link: Option<(i64, i64, i64)> = None;
    let mut pending: Vec<Value> = Vec::new();
    let mut seen = 0usize;
    loop {
        s.throttle.respect_pause().await;
        let (messages, after) = s.messages(conv_id, cursor.as_deref()).await?;
        pending.extend(messages);
        let link = match link {
            Some(link) => link,
            None => {
                let Some(participant) = find_customer(&participants, &pending, &s.ctx.ig_id) else {
                    cursor = after;
                    if cursor.is_none() {
                        out.skipped += pending.len().max(1) as i64;
                        return Ok(false);
                    }
                    continue;
                };
                let mut contact = to_incoming_contact(&participant);
                if !s.ctx.facebook {
                    resolve_profile(s, &mut contact).await;
                }
                match crate::inbound::detect_contact_and_conversation(
                    deps,
                    &s.ctx.inbox,
                    &contact,
                    "inboundMessage",
                )
                .await
                {
                    Ok((contact_inbox, conversation, is_new)) => {
                        out.contacts += i64::from(is_new);
                        *link.insert((contact_inbox.id, contact_inbox.contact_id, conversation.id))
                    }
                    Err(error) => {
                        // `failureReason`: the contact was not imported.
                        out.error = Some(error.to_string());
                        out.skipped += pending.len() as i64;
                        return Ok(false);
                    }
                }
            }
        };
        let mut page = Vec::new();
        for message in pending.drain(..) {
            seen += 1;
            if let Some(historical) = historical_message(&message, &s.ctx.ig_id, s.cutoff, seen) {
                page.push(historical);
            }
        }
        flush(deps, s, link, page, out).await?;
        cursor = after;
        if cursor.is_none() {
            return Ok(true);
        }
    }
}

/// `resolveContactProfile` + `splitDisplayName` (best effort).
async fn resolve_profile<U: Upstream>(s: &Pull<'_, U>, contact: &mut IncomingContact) {
    s.throttle.respect_pause().await;
    match s
        .get(&contact.source_id, &[("fields", "id,name,username")])
        .await
    {
        Ok(profile) => {
            let (first, last) = split_display_name(profile["name"].as_str());
            if first.is_some() {
                contact.first_name = first;
            }
            if last.is_some() {
                contact.last_name = last;
            }
        }
        Err(error) => {
            tracing::warn!(%error, run_id = s.run_id, source_id = %contact.source_id, "[coexist] Instagram contact profile resolution failed");
        }
    }
}

/// One `bulkImportMessages` call with its `discoverContactEnrichment`.
async fn flush<U>(
    deps: &Deps,
    s: &Pull<'_, U>,
    link: (i64, i64, i64),
    page: Vec<HistoricalMessage>,
    out: &mut ConvResult,
) -> Result<(), String> {
    let mut discovered: (Option<String>, Option<String>) = (None, None);
    for message in &page {
        if message.text.is_empty() || (discovered.0.is_some() && discovered.1.is_some()) {
            continue;
        }
        let found = extract_contact_info(
            Some(&message.text),
            s.ctx.default_country.as_deref(),
            ExtractOptions {
                skip_phone: discovered.0.is_some(),
                skip_email: discovered.1.is_some(),
            },
        );
        discovered.0 = discovered.0.take().or(found.phone_number);
        discovered.1 = discovered.1.take().or(found.email);
    }
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
    let ws = s.ctx.inbox.workspace_id;
    let (phone, email) = discovered;
    let result = write(deps, move |c| {
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
    out.messages += result.imported;
    out.skipped += result.skipped;
    let ai_marker = if s.ctx.ai_reads_synced_history {
        None
    } else {
        result.newest_message_id
    };
    if newest.is_some() || ai_marker.is_some() {
        out.activity.push(Activity {
            contact_inbox_id: link.0,
            contact_id: link.1,
            conversation_id: link.2,
            newest,
            oldest,
            newest_incoming,
            ai_marker,
        });
    }
    Ok(())
}

/// The hot-chained next chunk (`buildCoexistPageJobId`).
async fn continue_later<U>(
    deps: &Deps,
    s: &Pull<'_, U>,
    integration_id: i64,
    workspace_id: i64,
    attempts: i64,
    page_number: i64,
) -> Result<(), String> {
    crate::inbound::add_job(
        deps,
        QueueName::Integration,
        JOB,
        json!({ "type": JOB, "data": {
            "runId": s.run_id.to_string(),
            "integrationId": integration_id.to_string(),
            "workspaceId": workspace_id.to_string(),
        }}),
        JobOptions {
            job_id: Some(format!(
                "coexist-run-{}-{attempts}-page-{}",
                s.run_id,
                page_number + 1
            )),
            attempts: Some(1),
            remove_on_complete: Some(Retention::Remove),
            remove_on_fail: Some(Retention::KeepCount(100)),
            ..JobOptions::default()
        },
    )
    .await
    .map(drop)
    .map_err(|e| e.to_string())
}

/// `splitDisplayName`.
fn split_display_name(raw: Option<&str>) -> (Option<String>, Option<String>) {
    let Some(trimmed) = raw.map(str::trim).filter(|t| !t.is_empty()) else {
        return (None, None);
    };
    let mut parts = trimmed.split_whitespace();
    let first = parts.next().map(str::to_owned);
    let rest = parts.collect::<Vec<_>>().join(" ");
    (first, (!rest.is_empty()).then_some(rest))
}

/// `findCustomerParticipant`.
pub(crate) fn find_customer(
    participants: &[Value],
    messages: &[Value],
    ig_id: &str,
) -> Option<Value> {
    let other = |p: &Value| p["id"].as_str().is_some_and(|id| id != ig_id);
    if let Some(p) = participants.iter().find(|p| other(p)) {
        return Some(p.clone());
    }
    messages.iter().find_map(|m| {
        if m["from"].is_object() && m["from"]["id"].as_str() != Some(ig_id) {
            return Some(m["from"].clone());
        }
        m["to"]["data"]
            .as_array()?
            .iter()
            .find(|p| other(p))
            .cloned()
    })
}

/// `toIncomingContact`.
pub(crate) fn to_incoming_contact(participant: &Value) -> IncomingContact {
    let id = participant["id"].as_str().unwrap_or_default().to_owned();
    let username = participant["username"].as_str().map(str::to_owned);
    let (first, last) = split_display_name(participant["name"].as_str().or(username.as_deref()));
    IncomingContact {
        first_name: Some(
            first
                .or_else(|| username.clone())
                .unwrap_or_else(|| id.clone()),
        ),
        last_name: last,
        source_username: username,
        source_id: id,
        ..IncomingContact::default()
    }
}

/// `toIncomingAttachment`: needs both a URL and the provider id.
fn attachments_of(m: &Value) -> Vec<IncomingAttachment> {
    let mut out = Vec::new();
    for raw in m["attachments"]["data"].as_array().into_iter().flatten() {
        let image = raw.get("image_data").filter(|v| v.is_object());
        let video = raw.get("video_data").filter(|v| v.is_object());
        let url = raw["payload"]["url"]
            .as_str()
            .or_else(|| image.and_then(|i| i["url"].as_str()))
            .or_else(|| video.and_then(|v| v["url"].as_str()))
            .or_else(|| raw["file_url"].as_str());
        let (Some(url), Some(id)) = (url, raw["id"].as_str()) else {
            continue;
        };
        let mime = raw["mime_type"]
            .as_str()
            .unwrap_or("application/octet-stream")
            .to_owned();
        let dims = image.or(video);
        out.push(IncomingAttachment {
            source_id: Some(id.to_owned()),
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
        });
    }
    out
}

/// `buildHistoricalMessage`.
fn historical_message(
    m: &Value,
    ig_id: &str,
    cutoff: i64,
    seen: usize,
) -> Option<HistoricalMessage> {
    let attachments = attachments_of(m);
    let text = m["message"].as_str().unwrap_or_default();
    if text.is_empty() && attachments.is_empty() {
        return None;
    }
    let created_at = m["created_time"]
        .as_str()
        .and_then(chatbotx_core::js::parse_date);
    if created_at.is_some_and(|at| at < cutoff) && seen > 100 {
        return None;
    }
    Some(HistoricalMessage {
        source_id: m["id"].as_str().unwrap_or_default().to_owned(),
        message_type: if m["from"]["id"].as_str() == Some(ig_id) {
            "outgoing"
        } else {
            "incoming"
        },
        text: text.to_owned(),
        created_at,
        attachments,
        content_attributes: None,
    })
}

#[cfg(test)]
#[path = "coexist_instagram_tests.rs"]
mod tests;
