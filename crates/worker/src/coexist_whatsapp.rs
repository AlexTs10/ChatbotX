//! WhatsApp Coexistence history sync (`integration/handlers/coexist/
//! whatsapp-{buffer,flush,flush-context,flush-patches,recovery}.ts`,
//! `bulk-historical-import.ts`, `schedule/handlers/scan-coexist-runs.ts`):
//! stage each Meta history payload, drain the staging rows into contacts and
//! historical messages under an exclusive run claim, and recover parked or
//! stranded runs every minute.
//!
//! ponytail: contacts are imported one by one through the inbound
//! `detect_contact_and_conversation` (no `contactCreated` event, analytics
//! row or usage counter); no `logProviderError` row on a failed chunk.
//! Inserted attachments queue `coexistAttachmentDownload` (`coexist_attachment`).
use std::collections::{HashMap, HashSet};
use std::time::Instant;

use chatbotx_core::utils::crypto::sha256_hex;
use chatbotx_db::repo::coexist_sync_run::{
    self as runs, CoexistIntegrationRow, CreateRunInput, LIVE_RUN_STATUSES, PickedCoexistRun,
    WriteGuard,
};
use chatbotx_db::repo::message::{
    self, BulkPatchContentAttributesParams, ContentAttributesPatch, CreateAttachmentInput,
    CreateMessageInput, FindManyBySourceIdsParams, SinceTimeInput,
};
use chatbotx_db::repo::models::{CoexistSyncRun, IntegrationWhatsapp};
use chatbotx_db::repo::patch::{self, Patch as Fields};
use chatbotx_db::repo::{Json, iso, new_id, now_ms, whatsapp_coexist_staging as staging};
use chatbotx_db::rusqlite::types::Value as Sql;
use chatbotx_db::rusqlite::{Connection, OptionalExtension, params};
use chatbotx_jobs::options::Retention;
use chatbotx_jobs::queues::coexist::{
    coexist_flush_job_id, coexist_page_job_id, coexist_revive_job_suffix, coexist_run_job_id,
};
use chatbotx_jobs::{JobError, JobOptions, QueueName};
use serde_json::{Value, json};

use crate::channel::InboundInbox;
use crate::coexist_whatsapp_payload::{
    ContactMessages, HistoricalMessage, HistoryMetadata, Patch, Reduced, is_terminal,
    reduce_metadata, reduce_staged_rows,
};
use crate::{Deps, db_fail, fail, id_of, str_of};

pub const BUFFER_JOB: &str = "coexistWhatsappBuffer";
pub const FLUSH_JOB: &str = "coexistWhatsappFlush";
pub const SCAN_JOB: &str = "scanCoexistRuns";
pub const HISTORY_DECLINED_ERROR: &str = "history_declined";
pub const HISTORY_TIMEOUT_ERROR: &str = "history_timeout";
const HISTORY_WINDOW_MS: i64 = 24 * 60 * 60 * 1000;
const FLUSH_DELAY_MS: u64 = 60_000;
const BATCH_SIZE: i64 = 100;
const CHUNK_BUDGET_MS: u128 = 4 * 60 * 1000;
const PENDING_PATCH_CAP: usize = 500;
const MAX_PK_REMINT_ATTEMPTS: usize = 64;
const SCAN_BATCH: i64 = 500;
const MAX_ATTEMPTS: i64 = 5;
const RECOVERY_BATCH: i64 = 200;
const STRANDED_LIMIT: i64 = 50;
const YEAR_MS: i64 = 365 * 24 * 60 * 60 * 1000;

pub(crate) async fn read<R: Send + 'static>(
    deps: &Deps,
    f: impl FnOnce(&Connection) -> chatbotx_db::repo::Result<R> + Send + 'static,
) -> Result<R, JobError> {
    chatbotx_db::repo::read(&deps.db, f).await.map_err(db_fail)
}

pub(crate) async fn write<R: Send + 'static>(
    deps: &Deps,
    f: impl FnOnce(&Connection) -> chatbotx_db::repo::Result<R> + Send + 'static,
) -> Result<R, JobError> {
    chatbotx_db::repo::write(&deps.db, move |tx| f(tx))
        .await
        .map_err(db_fail)
}

async fn integration_by_phone(
    deps: &Deps,
    phone: &str,
) -> Result<Option<IntegrationWhatsapp>, JobError> {
    let phone = phone.to_owned();
    read(deps, move |c| {
        chatbotx_db::repo::integration_whatsapp::find_by_phone_number_id(c, &phone, None)
    })
    .await
}

// ---------------------------------------------------------------- buffer

/// `coexistWhatsappBuffer`: stage the payload (idempotent on its hash), then
/// one coalesced flush 60 s later when coexist is on.
pub async fn buffer(deps: &Deps, data: &Value) -> Result<(), JobError> {
    let phone = str_of(data, "phoneNumberId").unwrap_or_default().to_owned();
    let Some(integration) = integration_by_phone(deps, &phone).await? else {
        tracing::warn!(
            phone = %phone,
            "[coexist] Dropped history payload for unknown WhatsApp integration"
        );
        return Ok(());
    };
    let payload = data["payload"].clone();
    // ponytail: serde_json key order, not `JSON.stringify`'s; stable across redeliveries.
    let hash = sha256_hex(&payload.to_string());
    let staged_phone = phone.clone();
    write(deps, move |c| {
        staging::stage_payload(c, new_id(), &staged_phone, &Json(payload), &hash)
    })
    .await?;
    if !integration.coexist_enabled {
        return Ok(());
    }
    crate::inbound::add_job(
        deps,
        QueueName::Integration,
        FLUSH_JOB,
        json!({ "type": FLUSH_JOB, "data": { "phoneNumberId": phone } }),
        JobOptions {
            delay_ms: Some(FLUSH_DELAY_MS),
            job_id: Some(coexist_flush_job_id(&phone)),
            remove_on_complete: Some(Retention::Remove),
            remove_on_fail: Some(Retention::Remove),
            ..JobOptions::default()
        },
    )
    .await
}

// ---------------------------------------------------------------- historical ids

const COEXIST_EPOCH_MS: i64 = 1_075_593_600_000; // 2004-02-01T00:00:00Z
const DISAMBIG_BITS: u32 = 14;
const DISAMBIG_MASK: i64 = (1 << DISAMBIG_BITS) - 1;
/// The TS id is 53 + 14 bits; an `int8` keeps 49 bits of milliseconds.
const MAX_TS: i64 = 1 << 49;

/// FNV-1a over the UTF-16 units (`charCodeAt`), masked to 14 bits.
fn hash_key(key: &str) -> i64 {
    let mut hash: u32 = 2_166_136_261;
    for unit in key.encode_utf16() {
        hash ^= u32::from(unit);
        hash = hash.wrapping_mul(16_777_619);
    }
    i64::from(hash) & DISAMBIG_MASK
}

/// `createHistoricalIdFactory`: ids sort by the message time; a re-mint
/// moves to the next free disambiguator.
#[derive(Default)]
pub struct HistoricalIds {
    used: HashSet<i64>,
}

impl HistoricalIds {
    pub fn mint(
        &mut self,
        at_ms: i64,
        source_id: &str,
        contact_inbox_id: i64,
    ) -> Result<i64, String> {
        let base = at_ms - COEXIST_EPOCH_MS;
        if !(0..MAX_TS).contains(&base) {
            return Err(format!(
                "createHistoricalIdFactory: {} out of range",
                iso(at_ms)
            ));
        }
        let start = hash_key(&format!("{contact_inbox_id}:{source_id}"));
        for ts in base..MAX_TS {
            for offset in 0..=DISAMBIG_MASK {
                let id = (ts << DISAMBIG_BITS) | ((start + offset) & DISAMBIG_MASK);
                if self.used.insert(id) {
                    return Ok(id);
                }
            }
        }
        Err(format!(
            "createHistoricalIdFactory: exhausted disambiguator space at {}",
            iso(at_ms)
        ))
    }
}

// ---------------------------------------------------------------- bulk import

#[derive(Debug, Default)]
struct ImportResult {
    imported_contacts: i64,
    imported_messages: i64,
    skipped: i64,
    failed_messages: i64,
    attachment_ids: Vec<i64>,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct Activity {
    pub(crate) contact_inbox_id: i64,
    pub(crate) contact_id: i64,
    pub(crate) conversation_id: i64,
    pub(crate) newest: Option<i64>,
    pub(crate) oldest: Option<i64>,
    pub(crate) newest_incoming: Option<i64>,
    pub(crate) ai_marker: Option<i64>,
}

pub(crate) struct MessagesResult {
    pub(crate) imported: i64,
    pub(crate) skipped: i64,
    pub(crate) newest_message_id: Option<i64>,
    pub(crate) attachment_ids: Vec<i64>,
}

/// One insert with the re-mint loop (`remintRow`): a primary-key collision
/// with an earlier sync takes the next id.
fn insert_one(
    conn: &Connection,
    ids: &mut HistoricalIds,
    mut row: CreateMessageInput,
) -> chatbotx_db::repo::Result<Option<i64>> {
    let at = row.created_at.unwrap_or_default();
    let source = row.source_id.clone().unwrap_or_default();
    for _ in 0..MAX_PK_REMINT_ATTEMPTS {
        match message::bulk_create(conn, std::slice::from_ref(&row)) {
            Ok(rows) => return Ok(rows.first().map(|r| r.id)),
            Err(err) if message::is_message_primary_key_violation(&err) => {
                row.id = Some(
                    ids.mint(at, &source, row.contact_inbox_id)
                        .map_err(chatbotx_db::repo::Error::msg)?,
                );
            }
            Err(err) => return Err(err),
        }
    }
    Err(chatbotx_db::repo::Error::msg(format!(
        "[coexist] Message PK collision unresolved after {MAX_PK_REMINT_ATTEMPTS} re-mints (sourceId={source})"
    )))
}

/// `bulkImportMessages` for one contact, in one transaction.
pub(crate) fn import_messages(
    conn: &Connection,
    workspace_id: i64,
    link: (i64, i64, i64),
    messages: &[HistoricalMessage],
) -> chatbotx_db::repo::Result<MessagesResult> {
    let (contact_inbox_id, contact_id, conversation_id) = link;
    let mut ids = HistoricalIds::default();
    let fallback = now_ms();
    let (mut imported, mut newest_message_id) = (0, None::<i64>);
    let mut attachment_ids = Vec::new();
    for m in messages {
        let created_at = m.created_at.unwrap_or(fallback);
        let outgoing = m.message_type == "outgoing";
        let row = CreateMessageInput {
            id: Some(
                ids.mint(created_at, &m.source_id, contact_inbox_id)
                    .map_err(chatbotx_db::repo::Error::msg)?,
            ),
            contact_inbox_id,
            conversation_id,
            workspace_id,
            content_attributes: m.content_attributes.clone(),
            content_type: "text".into(),
            message_type: m.message_type.into(),
            sender_type: if outgoing { "user" } else { "contact" }.into(),
            sender_id: (!outgoing).then_some(contact_id),
            source_id: Some(m.source_id.clone()),
            parent_id: None,
            text: Some(m.text.clone()),
            r#type: None,
            created_at: Some(created_at),
            updated_at: None,
        };
        let Some(id) = insert_one(conn, &mut ids, row)? else {
            continue;
        };
        imported += 1;
        newest_message_id = newest_message_id.max(Some(id));
        if !m.attachments.is_empty() {
            let rows: Vec<_> = m
                .attachments
                .iter()
                .map(|a| attachment_row(workspace_id, conversation_id, id, created_at, a))
                .collect();
            attachment_ids.extend(message::bulk_create_attachments(conn, &rows)?);
        }
    }
    Ok(MessagesResult {
        imported,
        skipped: messages.len() as i64 - imported,
        newest_message_id,
        attachment_ids,
    })
}

fn attachment_row(
    workspace_id: i64,
    conversation_id: i64,
    message_id: i64,
    message_created_at: i64,
    a: &crate::channel::IncomingAttachment,
) -> CreateAttachmentInput {
    CreateAttachmentInput {
        workspace_id,
        conversation_id,
        message_id,
        message_created_at,
        file_type: a.file_type.clone(),
        mime_type: a.mime_type.clone(),
        origin_path: a.origin_path.clone(),
        size: a.size,
        width: a.width,
        height: a.height,
        name: a.name.clone(),
        source_id: a.source_id.clone(),
        // `toKeepOriginal`.
        keep_original: Some(
            !(a.is_photo_message && matches!(a.file_type.as_str(), "image" | "gif")),
        ),
        ..CreateAttachmentInput::default()
    }
}

/// `applyCoexistActivityUpdates`: move the tracking times and the AI context
/// marker, never backwards (`LEAST` / `GREATEST` ignore NULL, like Postgres).
pub(crate) fn apply_activity(
    conn: &Connection,
    workspace_id: i64,
    updates: &[Activity],
) -> chatbotx_db::repo::Result<()> {
    let mut by_inbox: HashMap<i64, (i64, i64, i64, Option<i64>)> = HashMap::new();
    let mut by_conversation: HashMap<i64, (Option<i64>, Option<i64>)> = HashMap::new();
    for u in updates {
        if let (Some(newest), Some(oldest)) = (u.newest, u.oldest) {
            let e = by_inbox.entry(u.contact_inbox_id).or_insert((
                u.contact_id,
                newest,
                oldest,
                u.newest_incoming,
            ));
            e.1 = e.1.max(newest);
            e.2 = e.2.min(oldest);
            e.3 = e.3.max(u.newest_incoming);
        }
        let c = by_conversation
            .entry(u.conversation_id)
            .or_insert((u.newest, u.ai_marker));
        c.0 = c.0.max(u.newest);
        c.1 = c.1.max(u.ai_marker);
    }
    let mut oldest_by_contact: HashMap<i64, i64> = HashMap::new();
    for (id, (contact_id, newest, oldest, incoming)) in &by_inbox {
        conn.execute(
            r#"UPDATE "ContactInbox" SET
                 "firstInteractionAt" = min(COALESCE("firstInteractionAt", ?2), ?2),
                 "lastMessageAt" = max(COALESCE("lastMessageAt", ?3), ?3),
                 "lastIncomingMessageAt" = CASE WHEN ?4 IS NULL THEN "lastIncomingMessageAt"
                   ELSE max(COALESCE("lastIncomingMessageAt", ?4), ?4) END,
                 "createdAt" = min("createdAt", ?2)
               WHERE "id" = ?1 AND "contactId" = ?5
                 AND EXISTS (SELECT 1 FROM "Contact" c WHERE c."id" = ?5 AND c."workspaceId" = ?6)"#,
            params![id, oldest, newest, incoming, contact_id, workspace_id],
        )?;
        let o = oldest_by_contact.entry(*contact_id).or_insert(*oldest);
        *o = (*o).min(*oldest);
    }
    let rows: Vec<_> = oldest_by_contact
        .into_iter()
        .map(|(c, at)| (c, workspace_id, at))
        .collect();
    chatbotx_db::repo::contact::backdate_created_at(conn, &rows)?;
    for (id, (newest, marker)) in by_conversation {
        conn.execute(
            r#"UPDATE "Conversation" SET
                 "lastActivityAt" = CASE WHEN ?2 IS NOT NULL AND ("lastActivityAt" IS NULL OR "lastActivityAt" < ?2)
                   THEN ?2 ELSE "lastActivityAt" END,
                 "aiContextLastMessageId" = CASE WHEN ?3 IS NOT NULL AND ("aiContextLastMessageId" IS NULL OR "aiContextLastMessageId" < ?3)
                   THEN ?3 ELSE "aiContextLastMessageId" END
               WHERE "id" = ?1 AND "workspaceId" = ?4"#,
            params![id, newest, marker, workspace_id],
        )?;
    }
    Ok(())
}

/// `bulkImportHistorical`.
async fn import_batch(
    deps: &Deps,
    inbox: &InboundInbox,
    batch: &[ContactMessages],
    ai_reads_synced_history: bool,
) -> Result<ImportResult, JobError> {
    let mut out = ImportResult::default();
    let mut activity = Vec::new();
    for entry in batch {
        let (contact_inbox, conversation, is_new) =
            crate::inbound::detect_contact_and_conversation(
                deps,
                inbox,
                &entry.contact,
                "inboundMessage",
            )
            .await?;
        out.imported_contacts += i64::from(is_new);
        if entry.messages.is_empty() {
            continue;
        }
        let link = (contact_inbox.id, contact_inbox.contact_id, conversation.id);
        let (ws, messages) = (inbox.workspace_id, entry.messages.clone());
        match write(deps, move |c| import_messages(c, ws, link, &messages)).await {
            Ok(res) => {
                out.imported_messages += res.imported;
                out.skipped += res.skipped;
                out.attachment_ids.extend(res.attachment_ids);
                let times = || entry.messages.iter().filter_map(|m| m.created_at);
                let newest = times().max();
                let ai_marker = if ai_reads_synced_history {
                    None
                } else {
                    res.newest_message_id
                };
                if newest.is_some() || ai_marker.is_some() {
                    activity.push(Activity {
                        contact_inbox_id: link.0,
                        contact_id: link.1,
                        conversation_id: link.2,
                        newest,
                        oldest: times().min().or(newest),
                        newest_incoming: entry
                            .messages
                            .iter()
                            .filter(|m| m.message_type == "incoming")
                            .filter_map(|m| m.created_at)
                            .max(),
                        ai_marker,
                    });
                }
            }
            Err(error) => {
                tracing::error!(%error, source_id = %entry.contact.source_id, "[coexist] bulkImportMessages threw inside bulkImportHistorical");
                out.failed_messages += entry.messages.len() as i64;
            }
        }
    }
    let ws = inbox.workspace_id;
    write(deps, move |c| apply_activity(c, ws, &activity)).await?;
    Ok(out)
}

// ---------------------------------------------------------------- post-batch patches

/// `applyPostBatchPatches`: edits, revokes and late media on messages already
/// imported; what cannot be matched yet comes back to be carried.
fn apply_patches(
    conn: &Connection,
    workspace_id: i64,
    inbox_id: i64,
    candidates: Vec<Patch>,
) -> chatbotx_db::repo::Result<(Vec<Patch>, Vec<i64>)> {
    if candidates.is_empty() {
        return Ok((candidates, Vec::new()));
    }
    let wa_ids: Vec<String> = candidates.iter().map(|p| p.contact_wa_id.clone()).collect();
    let inboxes: HashMap<String, (i64, i64, Option<i64>)> =
        chatbotx_db::repo::contact_inbox::find_by_inbox_and_source_ids(conn, inbox_id, &wa_ids)?
            .into_iter()
            .map(|r| {
                (
                    r.source_id,
                    (r.id, r.created_at, r.last_incoming_message_at),
                )
            })
            .collect();
    let Some(since) = inboxes
        .values()
        .filter_map(|(_, created, incoming)| {
            message::get_safe_since_time(
                Some(SinceTimeInput::Date(incoming.unwrap_or(*created))),
                Some(YEAR_MS),
            )
        })
        .min()
    else {
        return Ok((candidates, Vec::new()));
    };
    let (mut ci_ids, mut source_ids) = (Vec::new(), Vec::new());
    for p in &candidates {
        if let Some((ci, _, _)) = inboxes.get(&p.contact_wa_id) {
            ci_ids.push(*ci);
            source_ids.push(p.source_id.clone());
        }
    }
    let rows: HashMap<(i64, String), message::MessageSourceRow> = message::find_many_by_source_ids(
        conn,
        &FindManyBySourceIdsParams {
            contact_inbox_ids: ci_ids,
            since_time: Some(since),
            source_ids,
            workspace_id,
        },
    )?
    .into_iter()
    .filter_map(|r| r.source_id.clone().map(|s| ((r.contact_inbox_id, s), r)))
    .collect();
    let (mut unresolved, mut attachments, mut patches) = (Vec::new(), Vec::new(), Vec::new());
    for p in candidates {
        let found = inboxes
            .get(&p.contact_wa_id)
            .and_then(|(ci, _, _)| rows.get(&(*ci, p.source_id.clone())).map(|row| (*ci, row)));
        let Some((ci, row)) = found else {
            unresolved.push(p);
            continue;
        };
        if let Some(a) = &p.attachment {
            attachments.push(attachment_row(
                workspace_id,
                row.conversation_id,
                row.id,
                row.created_at,
                a,
            ));
        }
        let overlay = match p.kind {
            "edit" => json!({ "edited": true }),
            "revoke" => json!({ "revoked": true }),
            _ => continue,
        };
        patches.push(ContentAttributesPatch {
            contact_inbox_id: ci,
            overlay: overlay.as_object().cloned().unwrap_or_default(),
            source_id: p.source_id.clone(),
            text: (p.kind == "edit")
                .then(|| p.text.clone())
                .flatten()
                .map(Some),
        });
    }
    let attachment_ids = message::bulk_create_attachments(conn, &attachments)?;
    message::bulk_patch_content_attributes(
        conn,
        &BulkPatchContentAttributesParams {
            patches,
            since_time: Some(since),
            workspace_id,
        },
    )?;
    Ok((unresolved, attachment_ids))
}

/// `capPendingPatches`: one entry per key (the latest), the newest 500.
fn cap_pending(entries: Vec<Patch>) -> Vec<Patch> {
    let mut order: Vec<String> = Vec::new();
    let mut by_key: HashMap<String, Patch> = HashMap::new();
    for p in entries {
        let key = p.key();
        if by_key.insert(key.clone(), p).is_none() {
            order.push(key);
        }
    }
    let mut unique: Vec<Patch> = order
        .into_iter()
        .filter_map(|k| by_key.remove(&k))
        .collect();
    if unique.len() > PENDING_PATCH_CAP {
        unique.sort_by(|a, b| a.staged_at.cmp(&b.staged_at));
        let dropped = unique.len() - PENDING_PATCH_CAP;
        tracing::warn!(
            dropped,
            cap = PENDING_PATCH_CAP,
            "[coexist] WhatsApp pending patch buffer full — dropped oldest patches"
        );
        unique.drain(..dropped);
    }
    unique
}

// ---------------------------------------------------------------- flush

struct Context {
    integration: IntegrationWhatsapp,
    inbox: InboundInbox,
    run: CoexistSyncRun,
    token: Option<String>,
    started: Instant,
    phone: String,
}

#[derive(Debug, Default)]
struct State {
    imported_contacts: i64,
    imported_messages: i64,
    skipped: i64,
    failed: i64,
    total_rows: i64,
    batch_number: i64,
    current_error: Option<String>,
    metadata: Option<HistoryMetadata>,
    pending: Vec<Patch>,
    terminal_seen: bool,
}

impl State {
    fn seed(run: &CoexistSyncRun) -> State {
        State {
            imported_contacts: run.imported_contact_count,
            imported_messages: run.imported_message_count,
            skipped: run.skipped_count,
            failed: run.failed_count,
            total_rows: run.current_scan,
            batch_number: run.current_page_number,
            current_error: run.current_error.clone(),
            metadata: run.last_phase.map(|phase| HistoryMetadata {
                phase: phase as f64,
                chunk_order: run.last_chunk_order.unwrap_or(0) as f64,
                progress: run.sync_progress as f64,
            }),
            pending: run
                .pending_patches
                .as_ref()
                .and_then(|p| p.0["entries"].as_array().cloned())
                .unwrap_or_default()
                .iter()
                .filter_map(Patch::from_json)
                .collect(),
            terminal_seen: false,
        }
    }

    fn counters(&self) -> Fields {
        let now = iso(now_ms());
        let entries: Vec<Value> = self.pending.iter().map(|p| p.to_json(&now)).collect();
        vec![
            ("currentScan", Sql::Integer(self.total_rows)),
            ("importedContactCount", Sql::Integer(self.imported_contacts)),
            ("importedMessageCount", Sql::Integer(self.imported_messages)),
            ("skippedCount", Sql::Integer(self.skipped)),
            ("failedCount", Sql::Integer(self.failed)),
            (
                "pendingPatches",
                patch::json(&json!({ "entries": entries })),
            ),
        ]
    }
}

fn text(value: Option<&str>) -> Sql {
    value.map_or(Sql::Null, |v| Sql::Text(v.to_owned()))
}

/// A guarded `updateProgress`: the row count (0 = the claim was lost).
async fn guarded(deps: &Deps, ctx: &Context, fields: Fields) -> Result<usize, JobError> {
    let (id, token) = (ctx.run.id, ctx.token.clone());
    write(deps, move |c| {
        let guard = WriteGuard {
            status: "running",
            claim_token: token.as_deref(),
        };
        runs::update_progress(c, id, &fields, Some(&guard))
    })
    .await
}

/// `stillOwnsRun`: a heartbeat under the guard.
async fn still_owns(deps: &Deps, ctx: &Context) -> Result<bool, JobError> {
    Ok(guarded(deps, ctx, vec![("lastHeartbeatAt", Sql::Integer(now_ms()))]).await? > 0)
}

fn abandon(ctx: &Context, reason: &str) {
    tracing::warn!(
        run_id = ctx.run.id,
        phone = %ctx.phone,
        reason,
        "[coexist] WhatsApp flush run no longer claimed by this worker — abandoning"
    );
}

/// `loadFlushContext`.
async fn load_context(deps: &Deps, data: &Value) -> Result<Option<Context>, JobError> {
    let phone = str_of(data, "phoneNumberId").unwrap_or_default().to_owned();
    let Some(integration) = integration_by_phone(deps, &phone).await? else {
        tracing::warn!(phone = %phone, "[coexist] Flush: WhatsApp integration gone");
        return Ok(None);
    };
    if !integration.coexist_enabled {
        tracing::info!(
            phone = %phone,
            "[coexist] Flush skipped — coexist disabled, payloads remain staged"
        );
        return Ok(None);
    }
    let integration_id = integration.id;
    let run_id = match id_of(&data["runId"]) {
        Some(id) => Some(id),
        None => read(deps, move |c| {
            runs::find_live_run(c, integration_id, "whatsapp")
        })
        .await?
        .map(|r| r.id),
    };
    let Some(run_id) = run_id else {
        tracing::info!(
            phone = %phone,
            "[coexist] Flush: no live run — payloads remain staged"
        );
        return Ok(None);
    };
    let Some(run) = write(deps, move |c| {
        runs::claim_run_with_new_token(c, run_id, Some(LIVE_RUN_STATUSES))
    })
    .await?
    else {
        tracing::warn!(
            run_id,
            phone = %phone,
            "[coexist] WhatsApp flush run already claimed by another worker — abandoning"
        );
        return Ok(None);
    };
    let token = run.claim_token.clone();
    if integration.workspace_id != run.workspace_id {
        tracing::warn!(
            run_id,
            phone = %phone,
            "[coexist] Flush: workspaceId mismatch — refusing"
        );
        let token = token.clone();
        write(deps, move |c| {
            let guard = WriteGuard {
                status: "running",
                claim_token: token.as_deref(),
            };
            runs::mark_failed(
                c,
                run_id,
                "workspaceId mismatch between integration and run",
                Some(&guard),
            )
        })
        .await?;
        return Ok(None);
    }
    let (inbox_id, ws) = (integration.inbox_id, integration.workspace_id);
    let channel: Option<String> = read(deps, move |c| {
        Ok(c.query_row(
            r#"SELECT "channel" FROM "Inbox" WHERE "id" = ?1"#,
            params![inbox_id],
            |r| r.get(0),
        )
        .optional()?)
    })
    .await?;
    let Some(channel) = channel else {
        return Err(fail("Inbox not found"));
    };
    Ok(Some(Context {
        inbox: InboundInbox {
            inbox_id,
            workspace_id: ws,
            channel,
        },
        integration,
        run,
        token,
        started: Instant::now(),
        phone,
    }))
}

enum Batch {
    Next,
    Exhausted,
    Declined,
    LostClaim,
}

/// `drainBatch`: ownership is re-checked before the select, before the import
/// and before the staging rows are marked.
async fn drain_batch(deps: &Deps, ctx: &Context, state: &mut State) -> Result<Batch, JobError> {
    state.batch_number += 1;
    if !still_owns(deps, ctx).await? {
        return Ok(Batch::LostClaim);
    }
    let phone = ctx.phone.clone();
    let rows = read(deps, move |c| staging::list_pending(c, &phone, BATCH_SIZE)).await?;
    if rows.is_empty() {
        return Ok(Batch::Exhausted);
    }
    let staged: Vec<(i64, Value)> = rows.iter().map(|r| (r.id, r.payload.0.clone())).collect();
    let reduced = reduce_staged_rows(&staged);
    state.terminal_seen |= reduced.terminal_seen;
    if let Some(meta) = reduced.metadata {
        state.metadata = Some(reduce_metadata(state.metadata, meta));
    }
    if !still_owns(deps, ctx).await? {
        return Ok(Batch::LostClaim);
    }
    import_and_patch(deps, ctx, state, &reduced, rows.len() as i64).await?;
    if !still_owns(deps, ctx).await? {
        return Ok(Batch::LostClaim);
    }
    let parked = reduced.parse_failed_row_ids.clone();
    let processed: Vec<i64> = rows
        .iter()
        .map(|r| r.id)
        .filter(|id| !parked.contains(id))
        .collect();
    write(deps, move |c| {
        staging::mark_processed(c, &processed)?;
        staging::mark_parse_failed(c, &parked)
    })
    .await?;
    let mut fields = state.counters();
    fields.extend([
        ("currentPageNumber", Sql::Integer(state.batch_number)),
        (
            "currentStep",
            Sql::Text(format!("flushing batch {}", state.batch_number)),
        ),
        ("currentError", text(state.current_error.as_deref())),
        ("lastHeartbeatAt", Sql::Integer(now_ms())),
    ]);
    if let Some(meta) = state.metadata {
        fields.extend([
            ("lastPhase", Sql::Integer(meta.phase as i64)),
            ("lastChunkOrder", Sql::Integer(meta.chunk_order as i64)),
            ("syncProgress", Sql::Integer(meta.progress as i64)),
        ]);
    }
    if guarded(deps, ctx, fields).await? == 0 {
        return Ok(Batch::LostClaim);
    }
    Ok(if reduced.declined {
        Batch::Declined
    } else if (rows.len() as i64) < BATCH_SIZE {
        Batch::Exhausted
    } else {
        Batch::Next
    })
}

/// `importAndPatch` + `importBatch` + `applyBatchPatches`. A throw counts the
/// batch as failed and leaves its staging rows for the retry.
async fn import_and_patch(
    deps: &Deps,
    ctx: &Context,
    state: &mut State,
    reduced: &Reduced,
    rows: i64,
) -> Result<(), JobError> {
    let result = match import_batch(
        deps,
        &ctx.inbox,
        &reduced.batch,
        ctx.integration.coexist_ai_reads_synced_history,
    )
    .await
    {
        Ok(result) => result,
        Err(error) => {
            state.failed += reduced
                .batch
                .iter()
                .map(|b| b.messages.len() as i64)
                .sum::<i64>();
            state.total_rows += rows;
            tracing::error!(%error, run_id = ctx.run.id, batch = state.batch_number, "[coexist] WhatsApp bulk import threw — batch lost");
            return Err(error);
        }
    };
    state.imported_contacts += result.imported_contacts;
    state.imported_messages += result.imported_messages;
    state.skipped += result.skipped;
    state.failed += result.failed_messages;
    state.total_rows += rows;
    // Carried patches keep their first `stagedAt`.
    let staged_at: HashMap<String, Option<String>> = state
        .pending
        .iter()
        .map(|p| (p.key(), p.staged_at.clone()))
        .collect();
    let mut candidates = std::mem::take(&mut state.pending);
    candidates.extend(reduced.patches.iter().cloned());
    let (ws, inbox) = (ctx.integration.workspace_id, ctx.inbox.inbox_id);
    let (unresolved, patch_attachment_ids) =
        write(deps, move |c| apply_patches(c, ws, inbox, candidates)).await?;
    // Best effort: a failed enqueue leaves the bytes pending, never the chunk failed.
    let mut attachment_ids = result.attachment_ids;
    attachment_ids.extend(patch_attachment_ids);
    if let Err(error) = crate::coexist_attachment::enqueue(
        deps,
        ws,
        ctx.integration.id,
        "whatsapp",
        &attachment_ids,
    )
    .await
    {
        tracing::error!(%error, run_id = ctx.run.id, count = attachment_ids.len(), "[coexist] WhatsApp attachment download enqueue failed — bytes left as pending");
    }
    let now = iso(now_ms());
    state.pending = cap_pending(
        unresolved
            .into_iter()
            .map(|mut p| {
                if p.staged_at.is_none() {
                    p.staged_at = staged_at
                        .get(&p.key())
                        .cloned()
                        .flatten()
                        .or_else(|| Some(now.clone()));
                }
                p
            })
            .collect(),
    );
    Ok(())
}

enum Drain {
    Exhausted,
    ContinueLater,
    LostClaim,
}

async fn drain_run(deps: &Deps, ctx: &Context, state: &mut State) -> Result<Drain, JobError> {
    while ctx.started.elapsed().as_millis() < CHUNK_BUDGET_MS {
        match drain_batch(deps, ctx, state).await? {
            Batch::LostClaim => return Ok(Drain::LostClaim),
            Batch::Declined => {
                // A decline is terminal: Meta never sends history for this number.
                let id = ctx.integration.id;
                write(deps, move |c| {
                    chatbotx_db::repo::integration_whatsapp::mark_history_declined(c, id)
                })
                .await?;
                state.terminal_seen = true;
                state.current_error = Some(HISTORY_DECLINED_ERROR.into());
                return Ok(Drain::Exhausted);
            }
            Batch::Exhausted => return Ok(Drain::Exhausted),
            Batch::Next => {}
        }
    }
    Ok(Drain::ContinueLater)
}

/// `resolveFinalStatus`: `None` = late rows, continue instead.
async fn final_status(
    deps: &Deps,
    ctx: &Context,
    state: &State,
) -> Result<Option<&'static str>, JobError> {
    let phone = ctx.phone.clone();
    if !read(deps, move |c| staging::list_pending(c, &phone, 1))
        .await?
        .is_empty()
    {
        return Ok(None);
    }
    let terminal = state.terminal_seen
        || is_terminal(
            state.metadata.map(|m| m.phase),
            state.metadata.map(|m| m.progress),
        );
    Ok(Some(if !terminal {
        "waiting"
    } else if state.failed > 0 && (state.imported_messages > 0 || state.skipped > 0) {
        "partial"
    } else if state.failed > 0 {
        "failed"
    } else {
        "succeeded"
    }))
}

/// `enqueueContinuation`: hand the run back (`init`), then queue the next chunk.
async fn enqueue_continuation(deps: &Deps, ctx: &Context, state: &State) -> Result<(), JobError> {
    let released = guarded(
        deps,
        ctx,
        vec![
            ("status", Sql::Text("init".into())),
            ("lastHeartbeatAt", Sql::Integer(now_ms())),
        ],
    )
    .await?;
    if released == 0 {
        abandon(ctx, "release before continuation matched no rows");
        return Ok(());
    }
    let run_id = ctx.run.id.to_string();
    let job_id = coexist_page_job_id(
        &run_id,
        ctx.run.attempts as u32,
        (state.batch_number + 1) as u32,
    );
    let added = crate::inbound::add_job(
        deps,
        QueueName::Integration,
        FLUSH_JOB,
        json!({ "type": FLUSH_JOB, "data": { "runId": run_id, "phoneNumberId": ctx.phone } }),
        JobOptions {
            job_id: Some(job_id),
            attempts: Some(1),
            remove_on_complete: Some(Retention::Remove),
            remove_on_fail: Some(Retention::KeepCount(100)),
            ..JobOptions::default()
        },
    )
    .await;
    if let Err(error) = added {
        // The run is `init` with a fresh heartbeat: the scheduler drives it.
        tracing::error!(%error, %run_id, "[coexist] WhatsApp continuation enqueue failed — fallback to scheduler");
    }
    Ok(())
}

/// `coexistWhatsappFlush`: one chunk under an exclusive claim, then a
/// continuation, a parked `waiting` run or a terminal status.
pub async fn flush(deps: &Deps, data: &Value) -> Result<(), JobError> {
    let Some(ctx) = load_context(deps, data).await? else {
        return Ok(());
    };
    let mut state = State::seed(&ctx.run);
    let outcome: Result<Option<Option<&'static str>>, JobError> = async {
        match drain_run(deps, &ctx, &mut state).await? {
            Drain::LostClaim => Ok(None),
            Drain::Exhausted => Ok(Some(final_status(deps, &ctx, &state).await?)),
            Drain::ContinueLater => Ok(Some(None)),
        }
    }
    .await;
    match outcome {
        Ok(None) => abandon(&ctx, "a guarded write matched no rows"),
        Ok(Some(None)) => enqueue_continuation(deps, &ctx, &state).await?,
        Ok(Some(Some(status))) => {
            let waiting = status == "waiting";
            let mut fields = state.counters();
            fields.extend([
                ("status", Sql::Text(status.into())),
                (
                    "finishedAt",
                    if waiting {
                        Sql::Null
                    } else {
                        Sql::Integer(now_ms())
                    },
                ),
                ("lastHeartbeatAt", Sql::Integer(now_ms())),
                (
                    "currentStep",
                    Sql::Text(
                        if waiting {
                            "waiting for history"
                        } else {
                            "done"
                        }
                        .into(),
                    ),
                ),
                ("currentError", text(state.current_error.as_deref())),
            ]);
            if guarded(deps, &ctx, fields).await? == 0 {
                abandon(&ctx, &format!("finalize to {status} matched no rows"));
            }
        }
        Err(error) => {
            // `handleDrainError`: transient until proven otherwise — back to
            // `init` with the counters, and let the attempts bound it.
            state.current_error = Some(error.to_string());
            tracing::error!(%error, run_id = ctx.run.id, "[coexist] WhatsApp flush chunk failed — will retry");
            let mut fields = state.counters();
            fields.extend([
                ("status", Sql::Text("init".into())),
                ("currentError", text(state.current_error.as_deref())),
                ("lastHeartbeatAt", Sql::Integer(now_ms())),
            ]);
            if guarded(deps, &ctx, fields).await? == 0 {
                abandon(&ctx, "reset after error matched no rows");
                return Ok(());
            }
            return Err(error);
        }
    }
    tracing::info!(
        phone = %ctx.phone,
        run_id = ctx.run.id,
        imported_contacts = state.imported_contacts,
        imported_messages = state.imported_messages,
        skipped = state.skipped,
        failed = state.failed,
        rows = state.total_rows,
        "[coexist] WhatsApp flush chunk complete"
    );
    Ok(())
}

// ---------------------------------------------------------------- scan + recovery

/// `coexistRunEnqueuers[channel]`.
async fn enqueue_run(
    deps: &Deps,
    run: &PickedCoexistRun,
    suffix: Option<&str>,
) -> Result<(), JobError> {
    let run_id = run.id.to_string();
    let (name, data) = match run.channel.as_str() {
        "whatsapp" => {
            let (ws, id) = (run.workspace_id, run.integration_id);
            let row = read(deps, move |c| {
                runs::find_integration_for_coexist(c, ws, id, "whatsapp")
            })
            .await?;
            let phone = match row {
                Some(CoexistIntegrationRow::Whatsapp(i)) if !i.phone_number_id.is_empty() => {
                    i.phone_number_id
                }
                _ => {
                    let id = run.id;
                    write(deps, move |c| {
                        runs::mark_failed(c, id, "integration missing phoneNumberId", None)
                    })
                    .await?;
                    tracing::warn!(
                        %run_id,
                        "scanCoexistRuns: missing phoneNumberId, marked failed"
                    );
                    return Ok(());
                }
            };
            (
                FLUSH_JOB,
                json!({ "runId": run_id, "phoneNumberId": phone }),
            )
        }
        // The Instagram pull sync is a later unit; its job fails as an
        // undefined action until then, as before this port.
        "messenger" => ("coexistMessengerSync", pull_data(run)),
        _ => ("coexistInstagramSync", pull_data(run)),
    };
    crate::inbound::add_job(
        deps,
        QueueName::Integration,
        name,
        json!({ "type": name, "data": data }),
        JobOptions {
            job_id: Some(coexist_run_job_id(&run_id, run.attempts as u32, suffix)),
            attempts: Some(1),
            remove_on_complete: Some(Retention::Remove),
            remove_on_fail: Some(Retention::KeepCount(100)),
            ..JobOptions::default()
        },
    )
    .await
}

fn pull_data(run: &PickedCoexistRun) -> Value {
    json!({
        "runId": run.id.to_string(),
        "integrationId": run.integration_id.to_string(),
        "workspaceId": run.workspace_id.to_string(),
    })
}

/// `recoverWhatsappRuns`: revive parked runs with new rows, close the ones
/// past the 24 h window, open a run for stranded staging rows.
async fn recover_whatsapp(deps: &Deps) -> Result<(), JobError> {
    let revived = write(deps, |c| {
        runs::revive_waiting_runs_with_pending_staging(c, RECOVERY_BATCH)
    })
    .await?;
    let suffix = coexist_revive_job_suffix();
    for run in &revived {
        if let Err(error) = enqueue_run(deps, run, Some(&suffix)).await {
            tracing::error!(%error, run_id = run.id, "revived run enqueue failed");
        }
    }
    write(deps, |c| {
        runs::finalize_timed_out_waiting_runs(
            c,
            HISTORY_WINDOW_MS,
            HISTORY_TIMEOUT_ERROR,
            RECOVERY_BATCH,
        )
    })
    .await?;
    let stranded = read(deps, |c| {
        runs::find_stranded_coexist_whatsapp_integrations(c, STRANDED_LIMIT)
    })
    .await?;
    for candidate in stranded {
        let run = write(deps, move |c| {
            runs::create_run(
                c,
                &CreateRunInput {
                    workspace_id: candidate.workspace_id,
                    integration_id: candidate.integration_id,
                    channel: "whatsapp",
                    trigger_source: "scheduler-recovery",
                },
            )
        })
        .await;
        let picked = run.map(|run| PickedCoexistRun {
            id: run.id,
            attempts: run.attempts,
            channel: run.channel,
            integration_id: run.integration_id,
            workspace_id: run.workspace_id,
        });
        let result = match picked {
            Ok(picked) => enqueue_run(deps, &picked, None).await,
            Err(error) => Err(error),
        };
        if let Err(error) = result {
            tracing::error!(%error, integration_id = candidate.integration_id, "recovery run creation failed");
        }
    }
    Ok(())
}

/// `scanCoexistRuns`.
pub async fn scan_runs(deps: &Deps) -> Result<(), JobError> {
    if let Err(error) = recover_whatsapp(deps).await {
        tracing::error!(%error, channel = "whatsapp", "scanCoexistRuns: recovery pass failed");
    }
    write(deps, |c| {
        runs::mark_max_attempts_failed(c, "coexist", MAX_ATTEMPTS)
    })
    .await?;
    let picked = write(deps, |c| {
        runs::pick_due_runs(c, "coexist", SCAN_BATCH, MAX_ATTEMPTS)
    })
    .await?;
    for run in &picked {
        if let Err(error) = enqueue_run(deps, run, None).await {
            tracing::error!(%error, run_id = run.id, "scanCoexistRuns: enqueue failed");
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "coexist_whatsapp_tests.rs"]
mod tests;
