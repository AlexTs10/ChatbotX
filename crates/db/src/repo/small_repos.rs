//! Ports of small repositories without tests of their own (P3.3): `questionnaire`,
//! `whatsapp-message-template`, `workspace-bot-message-usage`, `whatsapp-flow`,
//! `webhook-execution`, `telegram-chat`, `workspace-mac`, `user-persistent-menu`,
//! `saved-reply-image`, `template-ai-file-dispatch`.

use std::collections::BTreeSet;

use rusqlite::Connection;

use super::models::{TelegramChat, UserPersistentMenu};
use super::utils::escape_like_pattern;
use super::{Json, Result, id_list, iso, new_id, now_ms, text_list};

// --- questionnaire ---------------------------------------------------------------

/// `questionnaireRepository.listNamesWithPrefix`: live questionnaire names that start with
/// `prefix` (case-sensitive `LIKE`, as Postgres; `%`/`_` in the prefix are literal).
pub fn questionnaire_list_names_with_prefix(
    conn: &Connection,
    workspace_id: i64,
    prefix: &str,
) -> Result<Vec<String>> {
    let pattern = format!("{}%", escape_like_pattern(prefix));
    let mut stmt = conn.prepare_cached(
        "SELECT \"name\" FROM \"Questionnaire\" WHERE \"workspaceId\" = ?1 AND \"deletedAt\" IS NULL \
         AND like_cs(\"name\", ?2)",
    )?;
    let rows = stmt
        .query_map(rusqlite::params![workspace_id, pattern], |r| r.get(0))?
        .collect::<rusqlite::Result<Vec<String>>>()?;
    Ok(rows)
}

// --- whatsapp-message-template ------------------------------------------------------

/// `whatsappMessageTemplateRepository.listIdsByIntegration`.
pub fn whatsapp_message_template_list_ids_by_integration(
    conn: &Connection,
    integration_whatsapp_id: i64,
) -> Result<Vec<i64>> {
    let mut stmt = conn.prepare_cached(
        "SELECT \"id\" FROM \"WhatsappMessageTemplate\" WHERE \"integrationWhatsappId\" = ?1",
    )?;
    let rows = stmt
        .query_map([integration_whatsapp_id], |r| r.get(0))?
        .collect::<rusqlite::Result<Vec<i64>>>()?;
    Ok(rows)
}

// --- workspace-bot-message-usage ------------------------------------------------------

/// Input of [`bot_message_usage_increment`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BotMessageUsageIncrement {
    pub count: i64,
    pub owner_id: i64,
    pub period_end: i64,
    pub period_start: i64,
    pub tenant_id: i64,
    pub workspace_id: i64,
}

/// `workspaceBotMessageUsageRepository.increment`: adds `count` to the workspace's row of
/// the period (creates it on the first call).
pub fn bot_message_usage_increment(
    conn: &Connection,
    input: &BotMessageUsageIncrement,
) -> Result<()> {
    let now = now_ms();
    conn.execute(
        "INSERT INTO \"WorkspaceBotMessageUsage\" (\"id\", \"createdAt\", \"updatedAt\", \"workspaceId\", \"ownerId\", \
         \"tenantId\", \"periodStart\", \"periodEnd\", \"count\") VALUES (?1, ?2, ?2, ?3, ?4, ?5, ?6, ?7, ?8) \
         ON CONFLICT (\"workspaceId\", \"periodStart\") DO UPDATE SET \
         \"count\" = \"WorkspaceBotMessageUsage\".\"count\" + excluded.\"count\", \"updatedAt\" = excluded.\"updatedAt\"",
        rusqlite::params![
            new_id(),
            now,
            input.workspace_id,
            input.owner_id,
            input.tenant_id,
            input.period_start,
            input.period_end,
            input.count
        ],
    )?;
    Ok(())
}

// --- whatsapp-flow -------------------------------------------------------------------

/// `whatsappFlowRepository.incrementCompletedCount`: the flow id, or `None` when no flow
/// of the integration has that source id.
pub fn whatsapp_flow_increment_completed_count(
    conn: &Connection,
    integration_whatsapp_id: i64,
    source_id: &str,
) -> Result<Option<i64>> {
    let mut stmt = conn.prepare_cached(
        "UPDATE \"WhatsappFlow\" SET \"completedCount\" = \"completedCount\" + 1, \"updatedAt\" = ?3 \
         WHERE \"integrationWhatsappId\" = ?1 AND \"sourceId\" = ?2 RETURNING \"id\"",
    )?;
    let mut rows = stmt.query(rusqlite::params![
        integration_whatsapp_id,
        source_id,
        now_ms()
    ])?;
    Ok(match rows.next()? {
        Some(row) => Some(row.get(0)?),
        None => None,
    })
}

// --- webhook-execution -----------------------------------------------------------------

/// `listExecutedWebhookPairs`: `"<webhookId>:<contactId>"` of the executed pairs.
pub fn list_executed_webhook_pairs(
    conn: &Connection,
    contact_ids: &[i64],
    webhook_ids: &[i64],
) -> Result<BTreeSet<String>> {
    if contact_ids.is_empty() || webhook_ids.is_empty() {
        return Ok(BTreeSet::new());
    }
    let mut stmt = conn.prepare_cached(
        "SELECT \"webhookId\", \"contactId\" FROM \"WebhookExecution\" \
         WHERE \"webhookId\" IN (SELECT value FROM json_each(?1)) AND \"contactId\" IN (SELECT value FROM json_each(?2))",
    )?;
    let rows = stmt
        .query_map(
            rusqlite::params![id_list(webhook_ids), id_list(contact_ids)],
            |r| Ok(format!("{}:{}", r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)),
        )?
        .collect::<rusqlite::Result<BTreeSet<String>>>()?;
    Ok(rows)
}

/// `markWebhookExecuted` (a repeat of the same pair is ignored).
pub fn mark_webhook_executed(
    conn: &Connection,
    contact_id: i64,
    webhook_id: i64,
    workspace_id: i64,
) -> Result<()> {
    let now = now_ms();
    conn.execute(
        "INSERT INTO \"WebhookExecution\" (\"id\", \"createdAt\", \"updatedAt\", \"executedAt\", \"webhookId\", \
         \"contactId\", \"workspaceId\") VALUES (?1, ?2, ?2, ?2, ?3, ?4, ?5) ON CONFLICT DO NOTHING",
        rusqlite::params![new_id(), now, webhook_id, contact_id, workspace_id],
    )?;
    Ok(())
}

/// `cleanupOldWebhookExecutions`: deletes executions before `older_than`; the count.
pub fn cleanup_old_webhook_executions(conn: &Connection, older_than: i64) -> Result<usize> {
    Ok(conn.execute(
        "DELETE FROM \"WebhookExecution\" WHERE \"executedAt\" < ?1",
        [older_than],
    )?)
}

// --- telegram-chat ---------------------------------------------------------------------

/// Input of [`telegram_chat_upsert`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TelegramChatUpsert {
    pub workspace_id: i64,
    pub inbox_id: i64,
    pub chat_id: String,
    pub title: Option<String>,
    pub r#type: String,
    pub bot_status: String,
}

/// `telegramChatRepository.upsert`: keeps the old title when the new one is NULL.
pub fn telegram_chat_upsert(conn: &Connection, input: &TelegramChatUpsert) -> Result<()> {
    let now = now_ms();
    conn.execute(
        "INSERT INTO \"TelegramChat\" (\"id\", \"createdAt\", \"updatedAt\", \"workspaceId\", \"inboxId\", \"chatId\", \
         \"title\", \"type\", \"botStatus\") VALUES (?1, ?2, ?2, ?3, ?4, ?5, ?6, ?7, ?8) \
         ON CONFLICT (\"inboxId\", \"chatId\") DO UPDATE SET \
         \"title\" = COALESCE(excluded.\"title\", \"TelegramChat\".\"title\"), \"type\" = excluded.\"type\", \
         \"botStatus\" = excluded.\"botStatus\", \"updatedAt\" = excluded.\"updatedAt\"",
        rusqlite::params![
            new_id(),
            now,
            input.workspace_id,
            input.inbox_id,
            input.chat_id,
            input.title,
            input.r#type,
            input.bot_status
        ],
    )?;
    Ok(())
}

/// `telegramChatRepository.listByInbox`: by title ascending (NULL titles last, as
/// Postgres `ASC`).
pub fn telegram_chat_list_by_inbox(
    conn: &Connection,
    workspace_id: i64,
    inbox_id: i64,
    bot_statuses: &[&str],
) -> Result<Vec<TelegramChat>> {
    super::find_many(
        conn,
        "WHERE \"workspaceId\" = ?1 AND \"inboxId\" = ?2 AND \"botStatus\" IN (SELECT value FROM json_each(?3)) \
         ORDER BY \"title\" IS NULL, \"title\" ASC",
        &[&workspace_id, &inbox_id, &text_list(bot_statuses)],
    )
}

// --- workspace-mac -----------------------------------------------------------------------

/// `workspaceMacRepository.sumForTenant`: MAC of the tenant's workspaces in the current
/// period (`periodStart <= now < periodEnd`).
pub fn mac_sum_for_tenant(conn: &Connection, tenant_id: i64, now: i64) -> Result<i64> {
    Ok(conn.query_row(
        "SELECT coalesce(sum(\"macCount\"), 0) FROM \"WorkspaceMac\" WHERE \
         (\"tenantId\" = ?1 OR (\"tenantId\" IS NULL AND \"workspaceId\" IN (SELECT \"id\" FROM \"Workspace\" WHERE \"tenantId\" = ?1))) \
         AND \"periodStart\" <= ?2 AND \"periodEnd\" > ?2",
        rusqlite::params![tenant_id, now],
        |r| r.get(0),
    )?)
}

/// `workspaceMacRepository.sumForOwner`: all periods when `cumulative`, else the period
/// that starts at `period_start`; 0 when neither.
pub fn mac_sum_for_owner(
    conn: &Connection,
    owner_id: i64,
    period_start: Option<i64>,
    cumulative: bool,
) -> Result<i64> {
    if !cumulative && period_start.is_none() {
        return Ok(0);
    }
    let owner = "(\"ownerId\" = ?1 OR (\"ownerId\" IS NULL AND \"workspaceId\" IN (SELECT \"id\" FROM \"Workspace\" WHERE \"ownerId\" = ?1)))";
    match (cumulative, period_start) {
        (false, Some(start)) => Ok(conn.query_row(
            &format!("SELECT coalesce(sum(\"macCount\"), 0) FROM \"WorkspaceMac\" WHERE {owner} AND \"periodStart\" = ?2"),
            rusqlite::params![owner_id, start],
            |r| r.get(0),
        )?),
        _ => Ok(conn.query_row(
            &format!("SELECT coalesce(sum(\"macCount\"), 0) FROM \"WorkspaceMac\" WHERE {owner}"),
            [owner_id],
            |r| r.get(0),
        )?),
    }
}

// --- user-persistent-menu -------------------------------------------------------------------

/// `listUserPersistentMenusByWorkspace`: newest first.
pub fn list_user_persistent_menus_by_workspace(
    conn: &Connection,
    workspace_id: i64,
) -> Result<Vec<UserPersistentMenu>> {
    super::find_many(
        conn,
        "WHERE \"workspaceId\" = ?1 ORDER BY \"createdAt\" DESC",
        &[&workspace_id],
    )
}

/// `findUserPersistentMenuById`.
pub fn find_user_persistent_menu_by_id(
    conn: &Connection,
    workspace_id: i64,
    id: i64,
) -> Result<Option<UserPersistentMenu>> {
    super::find_one(
        conn,
        "\"id\" = ?1 AND \"workspaceId\" = ?2",
        &[&id, &workspace_id],
    )
}

/// `createUserPersistentMenu`.
pub fn create_user_persistent_menu(
    conn: &Connection,
    workspace_id: i64,
    name: &str,
    menus: &Json,
) -> Result<Option<UserPersistentMenu>> {
    let now = now_ms();
    Ok(super::returning::<UserPersistentMenu>(
        conn,
        "INSERT INTO \"UserPersistentMenu\" (\"id\", \"createdAt\", \"updatedAt\", \"name\", \"menus\", \"workspaceId\") \
         VALUES (?1, ?2, ?2, ?3, ?4, ?5)",
        &[&new_id(), &now, &name, menus, &workspace_id],
    )?
    .into_iter()
    .next())
}

/// `updateUserPersistentMenu`.
pub fn update_user_persistent_menu(
    conn: &Connection,
    workspace_id: i64,
    id: i64,
    name: &str,
    menus: &Json,
) -> Result<Option<UserPersistentMenu>> {
    Ok(super::returning::<UserPersistentMenu>(
        conn,
        "UPDATE \"UserPersistentMenu\" SET \"name\" = ?3, \"menus\" = ?4, \"updatedAt\" = ?5 \
         WHERE \"id\" = ?1 AND \"workspaceId\" = ?2",
        &[&id, &workspace_id, &name, menus, &now_ms()],
    )?
    .into_iter()
    .next())
}

/// `deleteUserPersistentMenus`: the deleted ids.
pub fn delete_user_persistent_menus(
    conn: &Connection,
    workspace_id: i64,
    ids: &[i64],
) -> Result<Vec<i64>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let mut stmt = conn.prepare_cached(
        "DELETE FROM \"UserPersistentMenu\" WHERE \"id\" IN (SELECT value FROM json_each(?1)) AND \"workspaceId\" = ?2 \
         RETURNING \"id\"",
    )?;
    let rows = stmt
        .query_map(rusqlite::params![id_list(ids), workspace_id], |r| r.get(0))?
        .collect::<rusqlite::Result<Vec<i64>>>()?;
    Ok(rows)
}

// --- saved-reply-image -------------------------------------------------------------------

/// `SavedReplyImageWithFile`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavedReplyImageWithFile {
    pub id: i64,
    pub saved_reply_id: i64,
    pub media_library_file_id: i64,
    pub position: i64,
    pub file_name: String,
    pub file_path: String,
    pub file_mime_type: String,
    pub file_size: i64,
}

/// `savedReplyImageRepository.countReferences`.
pub fn saved_reply_image_count_references(
    conn: &Connection,
    media_library_file_id: i64,
) -> Result<i64> {
    super::client::count_where(
        conn,
        "SavedReplyImage",
        Some("\"mediaLibraryFileId\" = ?1"),
        &[&media_library_file_id],
    )
}

/// `savedReplyImageRepository.listBySavedReplyIds`: by position.
pub fn saved_reply_image_list_by_saved_reply_ids(
    conn: &Connection,
    workspace_id: i64,
    saved_reply_ids: &[i64],
) -> Result<Vec<SavedReplyImageWithFile>> {
    if saved_reply_ids.is_empty() {
        return Ok(Vec::new());
    }
    let mut stmt = conn.prepare_cached(
        "SELECT i.\"id\", i.\"savedReplyId\", i.\"mediaLibraryFileId\", i.\"position\", f.\"name\", f.\"path\", \
         f.\"mimeType\", f.\"size\" FROM \"SavedReplyImage\" i \
         INNER JOIN \"MediaLibraryFile\" f ON f.\"id\" = i.\"mediaLibraryFileId\" \
         WHERE i.\"workspaceId\" = ?1 AND i.\"savedReplyId\" IN (SELECT value FROM json_each(?2)) \
         ORDER BY i.\"position\" ASC",
    )?;
    let rows = stmt
        .query_map(
            rusqlite::params![workspace_id, id_list(saved_reply_ids)],
            |r| {
                Ok(SavedReplyImageWithFile {
                    id: r.get(0)?,
                    saved_reply_id: r.get(1)?,
                    media_library_file_id: r.get(2)?,
                    position: r.get(3)?,
                    file_name: r.get(4)?,
                    file_path: r.get(5)?,
                    file_mime_type: r.get(6)?,
                    file_size: r.get(7)?,
                })
            },
        )?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// `savedReplyImageRepository.countBySavedReplyId`.
pub fn saved_reply_image_count_by_saved_reply_id(
    conn: &Connection,
    workspace_id: i64,
    saved_reply_id: i64,
) -> Result<i64> {
    super::client::count_where(
        conn,
        "SavedReplyImage",
        Some("\"workspaceId\" = ?1 AND \"savedReplyId\" = ?2"),
        &[&workspace_id, &saved_reply_id],
    )
}

/// `savedReplyImageRepository.replaceForSavedReply`: replaces the images in order
/// (position = index). Run inside one write job.
pub fn saved_reply_image_replace_for_saved_reply(
    conn: &Connection,
    workspace_id: i64,
    saved_reply_id: i64,
    media_library_file_ids: &[i64],
) -> Result<()> {
    conn.execute(
        "DELETE FROM \"SavedReplyImage\" WHERE \"workspaceId\" = ?1 AND \"savedReplyId\" = ?2",
        rusqlite::params![workspace_id, saved_reply_id],
    )?;
    let now = now_ms();
    let mut stmt = conn.prepare_cached(
        "INSERT INTO \"SavedReplyImage\" (\"id\", \"createdAt\", \"updatedAt\", \"workspaceId\", \"savedReplyId\", \
         \"mediaLibraryFileId\", \"position\") VALUES (?1, ?2, ?2, ?3, ?4, ?5, ?6)",
    )?;
    for (position, file_id) in media_library_file_ids.iter().enumerate() {
        stmt.execute(rusqlite::params![
            new_id(),
            now,
            workspace_id,
            saved_reply_id,
            file_id,
            i64::try_from(position).unwrap_or(i64::MAX)
        ])?;
    }
    Ok(())
}

// --- template-ai-file-dispatch ---------------------------------------------------------------

/// `QueuedTemplateAiFile`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueuedTemplateAiFile {
    pub installation_id: i64,
    pub ai_file_id: i64,
    pub workspace_id: i64,
    pub run_id: String,
    pub queued_at: String,
}

fn select_queued(
    conn: &Connection,
    extra: &str,
    param: Option<rusqlite::types::Value>,
    limit: i64,
) -> Result<Vec<QueuedTemplateAiFile>> {
    let sql = format!(
        "SELECT f.\"id\", f.\"workspaceId\", r.\"installationId\", f.\"processingRun\" FROM \"AIFile\" f \
         INNER JOIN \"TemplateInstalledResource\" r ON r.\"workspaceId\" = f.\"workspaceId\" \
           AND r.\"resourceId\" = f.\"id\" AND r.\"resourceKind\" = 'aiFile' AND r.\"detachedAt\" IS NULL \
         WHERE f.\"processingRun\" ->> 'status' = 'queued' {extra} ORDER BY f.\"id\" ASC LIMIT ?2"
    );
    let mut stmt = conn.prepare_cached(&sql)?;
    let p1 = param.unwrap_or(rusqlite::types::Value::Null);
    let rows = stmt
        .query_map(rusqlite::params![p1, limit], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, Option<Json>>(3)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows
        .into_iter()
        .filter_map(|(ai_file_id, workspace_id, installation_id, run)| {
            let run = run?;
            Some(QueuedTemplateAiFile {
                installation_id,
                ai_file_id,
                workspace_id,
                run_id: run.get("id")?.as_str()?.to_owned(),
                queued_at: run.get("queuedAt")?.as_str()?.to_owned(),
            })
        })
        .collect())
}

/// `templateAiFileDispatchRepository.listQueuedForInstallation` (at most 500).
pub fn template_ai_file_list_queued_for_installation(
    conn: &Connection,
    installation_id: i64,
) -> Result<Vec<QueuedTemplateAiFile>> {
    select_queued(
        conn,
        "AND r.\"installationId\" = ?1",
        Some(rusqlite::types::Value::Integer(installation_id)),
        500,
    )
}

/// `templateAiFileDispatchRepository.listQueuedBefore`: runs queued before `queued_before`
/// (ISO text compare, as the TS).
pub fn template_ai_file_list_queued_before(
    conn: &Connection,
    queued_before: i64,
    limit: i64,
) -> Result<Vec<QueuedTemplateAiFile>> {
    select_queued(
        conn,
        "AND (f.\"processingRun\" ->> 'queuedAt') < ?1",
        Some(rusqlite::types::Value::Text(iso(queued_before))),
        limit,
    )
}
