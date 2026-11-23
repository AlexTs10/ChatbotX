//! Channel tag sync (Q.P7-V5): `tagSyncService.enqueue*`
//! (`packages/business/src/tag/sync.service.ts`) and the `default` queue
//! jobs `syncTag` (`apps/worker/src/default/handlers/sync-tag.ts`) and
//! `syncChannelLabels` (`sync-channel-labels.ts`): workspace tags mirrored
//! to Messenger page labels and Zalo OA tags.
//!
//! ponytail: one process-wide lock stands in for the per-(page, tag)
//! `distributedLock` (one worker process); per-key locks if label creation
//! becomes a bottleneck. The label delete on the channel is not ported: the
//! original calls it with `callApi: false` everywhere ("temporarily
//! disabled").

use std::time::Duration;

use chatbotx_db::Db;
use chatbotx_db::repo::models::{ContactInbox, IntegrationMessenger, IntegrationZalo, Tag};
use chatbotx_db::repo::tag_channel::{self, LabelMappingInput, TagChannelKey, TagChannelSummary};
use chatbotx_db::repo::{find_many, find_one};
use chatbotx_db::rusqlite::params;
use chatbotx_jobs::events::emit::EventBuses;
use chatbotx_jobs::{Enqueuer, JobOptions, NewJob, QueueName};
use serde_json::{Value, json};
use tokio::sync::Mutex;

use crate::channel_api::{
    ApiFailure, ChannelHttp, GRAPH_API_URL, META_DEFAULT_VERSION, graph_call, zalo_post,
    zalo_user_detail,
};
use crate::connection::credentials::UpstreamRequest;
use crate::error_log::{
    ProviderErrorInput, Thrown, log_provider_error, log_provider_error_for_channel,
};
use crate::errors::Result;
use crate::tx;

/// `DefaultJobAction.syncTag`.
pub const SYNC_TAG: &str = "syncTag";
/// `DefaultJobAction.syncChannelLabels`.
pub const SYNC_CHANNEL_LABELS: &str = "syncChannelLabels";

const SYNC_TAG_BULK_CHUNK_SIZE: usize = 1000;
const DELETE_CHUNK_SIZE: i64 = 500;
const SCAN_BATCH_SIZE: i64 = 100;
const USER_LABELS_MAX_PAGES: usize = 5;

static LABEL_LOCK: Mutex<()> = Mutex::const_new(());

// ---- producers (`TagSyncService`) --------------------------------------------

async fn add(enqueuer: &dyn Enqueuer, name: &str, data: Value) {
    let payload = json!({ "type": name, "data": data });
    if let Err(error) = enqueuer
        .add(QueueName::Default, name, payload, JobOptions::default())
        .await
    {
        tracing::warn!(job = name, %error, "tag sync: enqueue failed");
    }
}

/// `enqueueCreate`: after the Tag row commits.
pub async fn enqueue_create(enqueuer: &dyn Enqueuer, workspace_id: i64, tag_id: i64) {
    let data = json!({ "action": "create", "workspaceId": workspace_id.to_string(), "tagId": tag_id.to_string() });
    add(enqueuer, SYNC_TAG, data).await;
}

fn pair_data(action: &str, workspace_id: i64, contact_id: i64, tag_id: i64) -> Value {
    json!({
        "action": action,
        "workspaceId": workspace_id.to_string(),
        "contactId": contact_id.to_string(),
        "tagId": tag_id.to_string(),
    })
}

/// `enqueueAttach` for every newly linked `(contactId, tagId)` pair
/// (`enqueueAttachMany`: one bulk add per 1000 pairs).
pub async fn enqueue_attach_many(enqueuer: &dyn Enqueuer, workspace_id: i64, pairs: &[(i64, i64)]) {
    for chunk in pairs.chunks(SYNC_TAG_BULK_CHUNK_SIZE) {
        let jobs = chunk
            .iter()
            .map(|(contact, tag)| NewJob {
                name: SYNC_TAG.to_owned(),
                data: json!({ "type": SYNC_TAG, "data": pair_data("attach", workspace_id, *contact, *tag) }),
                opts: JobOptions::default(),
            })
            .collect();
        if let Err(error) = enqueuer.add_bulk(QueueName::Default, jobs).await {
            tracing::warn!(%error, "tag sync: bulk attach enqueue failed");
        }
    }
}

/// `enqueueDetach` for every removed `(contactId, tagId)` pair.
pub async fn enqueue_detach_many(enqueuer: &dyn Enqueuer, workspace_id: i64, pairs: &[(i64, i64)]) {
    for (contact, tag) in pairs {
        add(
            enqueuer,
            SYNC_TAG,
            pair_data("detach", workspace_id, *contact, *tag),
        )
        .await;
    }
}

/// `enqueueDelete`: a workspace delete (`channel = None`) or the delete of
/// one channel's label (`(channelType, integrationId)`, inbound webhook).
pub async fn enqueue_delete(
    enqueuer: &dyn Enqueuer,
    workspace_id: i64,
    tag_id: i64,
    channel: Option<(&str, i64)>,
) {
    let mut data = json!({ "action": "delete", "workspaceId": workspace_id.to_string(), "tagId": tag_id.to_string() });
    if let Some((channel_type, integration_id)) = channel {
        data["channelType"] = json!(channel_type);
        data["integrationId"] = json!(integration_id.to_string());
    }
    add(enqueuer, SYNC_TAG, data).await;
}

/// `enqueueChannelScan`: import the labels already on a connected channel.
pub async fn enqueue_channel_scan(
    enqueuer: &dyn Enqueuer,
    workspace_id: i64,
    channel_type: &str,
    integration_id: i64,
) {
    let data = json!({
        "workspaceId": workspace_id.to_string(),
        "channelType": channel_type,
        "integrationId": integration_id.to_string(),
    });
    add(enqueuer, SYNC_CHANNEL_LABELS, data).await;
}

/// The Zalo `enqueueChannelScan` of `zaloIntegrationService.connect`: only a
/// genuinely new OA row (a reconnect moves the old row and bumps
/// `updatedAt`). Never fails the caller.
pub async fn enqueue_new_zalo_scans(
    db: &Db,
    enqueuer: &dyn Enqueuer,
    connections: &[crate::connection::ConnectionRow],
) {
    for connection in connections {
        let (Some(inbox_id), "zalo") = (connection.inbox_id, connection.provider.as_str()) else {
            continue;
        };
        let created = tx::read(db, move |conn| {
            Ok(find_one::<IntegrationZalo>(
                conn,
                r#""inboxId" = ?1 AND "createdAt" = "updatedAt""#,
                &[&inbox_id],
            )?)
        })
        .await;
        match created {
            Ok(Some(row)) => {
                enqueue_channel_scan(enqueuer, row.workspace_id, "zalo", row.id).await;
            }
            Ok(None) => {}
            Err(error) => {
                tracing::warn!(inbox_id, %error, "zalo connect: channel tag scan enqueue failed")
            }
        }
    }
}

// ---- provider calls ----------------------------------------------------------

/// The Messenger page token and Graph version of an integration.
struct MessengerAuth {
    token: String,
    version: String,
}

fn messenger_auth(row: &IntegrationMessenger) -> MessengerAuth {
    let auth = &row.auth.0;
    MessengerAuth {
        token: auth["tokens"]["accessToken"]
            .as_str()
            .unwrap_or_default()
            .to_owned(),
        version: auth["version"]
            .as_str()
            .filter(|v| !v.is_empty())
            .unwrap_or(META_DEFAULT_VERSION)
            .to_owned(),
    }
}

fn zalo_token(row: &IntegrationZalo) -> String {
    row.auth.0["tokens"]["accessToken"]
        .as_str()
        .unwrap_or_default()
        .to_owned()
}

async fn graph(
    http: &dyn ChannelHttp,
    method: &'static str,
    auth: &MessengerAuth,
    path: &str,
    query: &[(&str, &str)],
) -> std::result::Result<Value, ApiFailure> {
    let endpoint = format!("{}/{path}", auth.version);
    let mut url = url::Url::parse(&format!("{GRAPH_API_URL}/{endpoint}"))
        .map_err(|e| ApiFailure::new(e.to_string()))?;
    if !query.is_empty() {
        url.query_pairs_mut().extend_pairs(query.iter().copied());
    }
    let request = UpstreamRequest {
        method,
        url: url.to_string(),
        headers: vec![("Authorization".into(), format!("Bearer {}", auth.token))],
        body: None,
    };
    graph_call(http, request, &endpoint).await
}

/// `createCustomLabel`: the new label id.
async fn create_label(
    http: &dyn ChannelHttp,
    auth: &MessengerAuth,
    page_id: &str,
    name: &str,
) -> std::result::Result<String, ApiFailure> {
    let path = format!("{page_id}/custom_labels");
    let reply = graph(http, "POST", auth, &path, &[("page_label_name", name)]).await?;
    Ok(match &reply["id"] {
        Value::String(id) => id.clone(),
        Value::Number(id) => id.to_string(),
        _ => String::new(),
    })
}

/// `getUserLabels`: `(id, page_label_name)`, at most 5 pages.
async fn user_labels(
    http: &dyn ChannelHttp,
    auth: &MessengerAuth,
    psid: &str,
) -> std::result::Result<Vec<(String, String)>, ApiFailure> {
    let path = format!("{psid}/custom_labels");
    let mut labels = Vec::new();
    let mut after: Option<String> = None;
    for _ in 0..USER_LABELS_MAX_PAGES {
        let mut query = vec![("fields", "id,page_label_name")];
        if let Some(cursor) = after.as_deref() {
            query.push(("after", cursor));
        }
        let reply = graph(http, "GET", auth, &path, &query).await?;
        for label in reply["data"].as_array().into_iter().flatten() {
            labels.push((
                label["id"].as_str().unwrap_or_default().to_owned(),
                label["page_label_name"]
                    .as_str()
                    .unwrap_or_default()
                    .to_owned(),
            ));
        }
        after = reply["paging"]["next"]
            .is_string()
            .then(|| {
                reply["paging"]["cursors"]["after"]
                    .as_str()
                    .map(str::to_owned)
            })
            .flatten();
        if after.is_none() {
            return Ok(labels);
        }
    }
    tracing::warn!(
        psid,
        fetched = labels.len(),
        "Messenger user labels truncated at the page limit"
    );
    Ok(labels)
}

fn thrown(failure: &ApiFailure) -> Thrown {
    Thrown::Error {
        message: failure.message.clone(),
        stack: None,
        status: failure.status.map(i64::from),
    }
}

// ---- rows --------------------------------------------------------------------

fn id_of(data: &Value, key: &str) -> Option<i64> {
    match &data[key] {
        Value::String(s) => s.parse().ok(),
        Value::Number(n) => n.as_i64(),
        _ => None,
    }
}

async fn tag_of(db: &Db, workspace_id: i64, tag_id: i64) -> Result<Option<Tag>> {
    tx::read(db, move |conn| {
        Ok(find_one::<Tag>(
            conn,
            r#""id" = ?1 AND "workspaceId" = ?2"#,
            &[&tag_id, &workspace_id],
        )?)
    })
    .await
}

async fn messenger_by(
    db: &Db,
    where_sql: &'static str,
    value: i64,
) -> Result<Vec<IntegrationMessenger>> {
    tx::read(db, move |conn| {
        Ok(find_many::<IntegrationMessenger>(
            conn,
            where_sql,
            &[&value],
        )?)
    })
    .await
}

async fn zalo_by(db: &Db, where_sql: &'static str, value: i64) -> Result<Vec<IntegrationZalo>> {
    tx::read(db, move |conn| {
        Ok(find_many::<IntegrationZalo>(conn, where_sql, &[&value])?)
    })
    .await
}

fn key(
    workspace_id: i64,
    tag_id: i64,
    channel_type: &str,
    integration_id: i64,
    label: &str,
) -> TagChannelKey {
    TagChannelKey {
        workspace_id,
        tag_id,
        channel_type: channel_type.to_owned(),
        integration_id,
        external_label_id: label.to_owned(),
    }
}

// ---- `syncTag` -----------------------------------------------------------------

/// `handleSyncTag`: the `action` of the job selects the operation. A provider
/// failure of one channel is logged; database errors fail the job.
pub async fn sync_tag(
    db: &Db,
    http: &dyn ChannelHttp,
    buses: Option<&EventBuses>,
    data: &Value,
) -> Result<()> {
    let (Some(workspace_id), Some(tag_id)) = (id_of(data, "workspaceId"), id_of(data, "tagId"))
    else {
        tracing::warn!(%data, "syncTag: bad data");
        return Ok(());
    };
    match data["action"].as_str() {
        Some("create") => create(db, http, buses, workspace_id, tag_id).await,
        Some("attach") => match id_of(data, "contactId") {
            Some(contact) => attach(db, http, workspace_id, contact, tag_id).await,
            None => Ok(()),
        },
        Some("detach") => match id_of(data, "contactId") {
            Some(contact) => detach(db, http, buses, workspace_id, contact, tag_id).await,
            None => Ok(()),
        },
        Some("delete") => {
            let channel = data["channelType"].as_str().filter(|c| !c.is_empty());
            delete(
                db,
                workspace_id,
                tag_id,
                channel,
                id_of(data, "integrationId"),
            )
            .await
        }
        _ => {
            tracing::warn!(%data, "syncTag: unknown action");
            Ok(())
        }
    }
}

/// `syncTagCreate`: the page label on every sync-enabled Messenger page; a
/// name mapping on every sync-enabled Zalo OA (no Zalo API call).
async fn create(
    db: &Db,
    http: &dyn ChannelHttp,
    buses: Option<&EventBuses>,
    workspace_id: i64,
    tag_id: i64,
) -> Result<()> {
    let Some(tag) = tag_of(db, workspace_id, tag_id).await? else {
        tracing::warn!(tag_id, "syncTag(create): tag missing");
        return Ok(());
    };
    for integration in messenger_by(db, r#"WHERE "workspaceId" = ?1"#, workspace_id).await? {
        if integration.sync_tag_enabled_at.is_none() {
            continue;
        }
        let _guard = LABEL_LOCK.lock().await;
        let auth = messenger_auth(&integration);
        match create_label(http, &auth, &integration.page_id, &tag.name).await {
            Ok(label) => {
                let k = key(workspace_id, tag.id, "messenger", integration.id, &label);
                tx::write(db, move |conn| {
                    match tag_channel::find_by_tag_and_integration(
                        conn,
                        k.workspace_id,
                        k.tag_id,
                        "messenger",
                        k.integration_id,
                    )? {
                        Some(existing) => {
                            tag_channel::update_external_label_id(conn, existing.id, &label)?
                        }
                        None => tag_channel::insert_if_absent(conn, &k)?,
                    }
                    Ok(())
                })
                .await?;
            }
            Err(failure) => {
                tracing::warn!(integration_id = integration.id, tag_id, error = %failure.message, "syncTag(create): messenger label create failed");
                let input = ProviderErrorInput::new(
                    "messenger",
                    &workspace_id.to_string(),
                    thrown(&failure),
                );
                log_provider_error(buses, input).await;
            }
        }
    }
    for integration in zalo_by(db, r#"WHERE "workspaceId" = ?1"#, workspace_id).await? {
        if integration.sync_tag_enabled_at.is_none() {
            continue;
        }
        let k = key(workspace_id, tag.id, "zalo", integration.id, &tag.name);
        tx::write(db, move |conn| Ok(tag_channel::insert_if_absent(conn, &k)?)).await?;
    }
    Ok(())
}

/// `syncTagAttach`: the tag on every Messenger / Zalo inbox of the contact.
/// A provider failure fails the job (the original rethrows).
async fn attach(
    db: &Db,
    http: &dyn ChannelHttp,
    workspace_id: i64,
    contact_id: i64,
    tag_id: i64,
) -> Result<()> {
    let Some(tag) = tag_of(db, workspace_id, tag_id).await? else {
        tracing::warn!(tag_id, "syncTag(attach): tag missing");
        return Ok(());
    };
    let inboxes = tx::read(db, move |conn| {
        Ok(find_many::<ContactInbox>(
            conn,
            r#"WHERE "contactId" = ?1"#,
            &[&contact_id],
        )?)
    })
    .await?;
    for inbox in inboxes {
        match inbox.channel.as_str() {
            "messenger" => attach_messenger(db, http, workspace_id, &tag, &inbox).await?,
            "zalo" => attach_zalo(db, http, workspace_id, &tag, &inbox).await?,
            _ => {}
        }
    }
    Ok(())
}

fn provider(failure: ApiFailure) -> crate::errors::BusinessError {
    crate::errors::BusinessError::exception(failure.message, "providerError", 500)
}

async fn attach_messenger(
    db: &Db,
    http: &dyn ChannelHttp,
    workspace_id: i64,
    tag: &Tag,
    inbox: &ContactInbox,
) -> Result<()> {
    let Some(integration) = messenger_by(db, r#"WHERE "inboxId" = ?1"#, inbox.inbox_id)
        .await?
        .into_iter()
        .next()
    else {
        return Ok(());
    };
    if integration.sync_tag_enabled_at.is_none() {
        return Ok(());
    }
    let auth = messenger_auth(&integration);
    let tag_channel = {
        let _guard = LABEL_LOCK.lock().await;
        let (ws, tag_id, integration_id) = (workspace_id, tag.id, integration.id);
        let existing = tx::read(db, move |conn| {
            Ok(tag_channel::find_by_tag_and_integration(
                conn,
                ws,
                tag_id,
                "messenger",
                integration_id,
            )?)
        })
        .await?;
        match existing {
            Some(row) => Some(row),
            None => {
                let label = create_label(http, &auth, &integration.page_id, &tag.name)
                    .await
                    .map_err(provider)?;
                let k = key(ws, tag_id, "messenger", integration_id, &label);
                tx::write(db, move |conn| Ok(tag_channel::insert_or_fetch(conn, &k)?)).await?
            }
        }
    };
    let Some(tag_channel) = tag_channel else {
        tracing::warn!(
            tag_id = tag.id,
            integration_id = integration.id,
            "syncTag(attach): failed to resolve messenger TagChannel"
        );
        return Ok(());
    };
    let path = format!("{}/label", tag_channel.external_label_id);
    graph(http, "POST", &auth, &path, &[("user", &inbox.source_id)])
        .await
        .map_err(provider)?;
    let (tag_id, channel_id, inbox_id) = (tag.id, tag_channel.id, inbox.id);
    tx::write(db, move |conn| {
        Ok(tag_channel::link_contact_inbox(
            conn, tag_id, channel_id, inbox_id,
        )?)
    })
    .await
}

async fn attach_zalo(
    db: &Db,
    http: &dyn ChannelHttp,
    workspace_id: i64,
    tag: &Tag,
    inbox: &ContactInbox,
) -> Result<()> {
    let Some(integration) = zalo_by(db, r#"WHERE "inboxId" = ?1"#, inbox.inbox_id)
        .await?
        .into_iter()
        .next()
    else {
        return Ok(());
    };
    if integration.sync_tag_enabled_at.is_none() {
        return Ok(());
    }
    let body = json!({ "user_id": inbox.source_id, "tag_name": tag.name });
    zalo_post(
        http,
        &zalo_token(&integration),
        "v2.0/oa/tag/tagfollower",
        &body,
    )
    .await
    .map_err(provider)?;
    let k = key(workspace_id, tag.id, "zalo", integration.id, &tag.name);
    let inbox_id = inbox.id;
    tx::write(db, move |conn| {
        if let Some(row) = tag_channel::upsert_by_tag_and_integration(conn, &k)? {
            tag_channel::link_contact_inbox(conn, k.tag_id, row.id, inbox_id)?;
        }
        Ok(())
    })
    .await
}

/// `syncTagDetach`: unassign on each sync-enabled channel, then drop the
/// local mapping whatever the API answered.
async fn detach(
    db: &Db,
    http: &dyn ChannelHttp,
    buses: Option<&EventBuses>,
    workspace_id: i64,
    contact_id: i64,
    tag_id: i64,
) -> Result<()> {
    let rows = tx::read(db, move |conn| {
        Ok(tag_channel::list_contact_tag_channel_rows(
            conn, tag_id, contact_id,
        )?)
    })
    .await?;
    for row in rows {
        if let Err(failure) = unassign(db, http, &row).await? {
            tracing::warn!(tag_channel_id = row.tag_channel_id, error = %failure.message, "syncTag(detach): skip per-row unassign error");
            let mut input = ProviderErrorInput::new(
                &row.channel_type,
                &workspace_id.to_string(),
                thrown(&failure),
            );
            input.contact_id = Some(contact_id.to_string());
            input.source_id = Some(row.source_id.clone());
            log_provider_error_for_channel(buses, &row.channel_type.clone(), input).await;
        }
        let (channel_id, inbox_id) = (row.tag_channel_id, row.contact_inbox_id);
        tx::write(db, move |conn| {
            Ok(tag_channel::unlink_contact_inbox(
                conn, channel_id, inbox_id,
            )?)
        })
        .await?;
    }
    Ok(())
}

/// `unassignOnChannel`: `Ok(Err(_))` = the provider refused.
async fn unassign(
    db: &Db,
    http: &dyn ChannelHttp,
    row: &tag_channel::ContactTagChannelRow,
) -> Result<std::result::Result<(), ApiFailure>> {
    match row.channel_type.as_str() {
        "messenger" => {
            let Some(integration) = messenger_by(db, r#"WHERE "id" = ?1"#, row.integration_id)
                .await?
                .into_iter()
                .next()
            else {
                return Ok(Ok(()));
            };
            if integration.sync_tag_enabled_at.is_none() {
                return Ok(Ok(()));
            }
            let path = format!("{}/label", row.external_label_id);
            let auth = messenger_auth(&integration);
            Ok(
                graph(http, "DELETE", &auth, &path, &[("user", &row.source_id)])
                    .await
                    .map(|_| ()),
            )
        }
        "zalo" => {
            let Some(integration) = zalo_by(db, r#"WHERE "id" = ?1"#, row.integration_id)
                .await?
                .into_iter()
                .next()
            else {
                return Ok(Ok(()));
            };
            if integration.sync_tag_enabled_at.is_none() {
                return Ok(Ok(()));
            }
            let body = json!({ "user_id": row.source_id, "tag_name": row.external_label_id });
            Ok(zalo_post(
                http,
                &zalo_token(&integration),
                "v2.0/oa/tag/rmfollowerfromtag",
                &body,
            )
            .await
            .map(|_| ()))
        }
        _ => Ok(Ok(())),
    }
}

/// `syncTagDelete`: one channel's mappings (and the contacts tagged through
/// it) for a channel-scoped delete; else every channel, the manual links and
/// the soft-deleted Tag row.
async fn delete(
    db: &Db,
    workspace_id: i64,
    tag_id: i64,
    channel_type: Option<&str>,
    integration_id: Option<i64>,
) -> Result<()> {
    let scoped = channel_type.zip(integration_id);
    let filter = scoped.map(|(c, i)| (c.to_owned(), i));
    let channels = tx::read(db, move |conn| {
        Ok(tag_channel::list_by_tag(
            conn,
            workspace_id,
            tag_id,
            filter.as_ref().map(|(c, _)| c.as_str()),
            filter.as_ref().map(|(_, i)| *i),
        )?)
    })
    .await?;
    for channel in channels {
        if let Err(error) = delete_on_channel(db, tag_id, &channel).await {
            if scoped.is_some() {
                return Err(error);
            }
            tracing::warn!(tag_id, channel_id = channel.id, %error, "syncTag(delete): skip per-channel cleanup error");
        }
    }
    if scoped.is_some() {
        return Ok(());
    }
    let mut after: Option<i64> = None;
    loop {
        let page = tx::read(db, move |conn| {
            Ok(tag_channel::list_tagged_contact_ids_page(
                conn,
                tag_id,
                after,
                DELETE_CHUNK_SIZE,
            )?)
        })
        .await?;
        let Some(last) = page.last().copied() else {
            break;
        };
        let full = page.len() as i64 == DELETE_CHUNK_SIZE;
        tx::write(db, move |conn| {
            Ok(tag_channel::delete_contact_tags_for_contacts(
                conn, tag_id, &page,
            )?)
        })
        .await?;
        if !full {
            break;
        }
        after = Some(last);
    }
    tx::write(db, move |conn| {
        conn.execute(
            r#"DELETE FROM "Tag" WHERE "id" = ?1 AND "workspaceId" = ?2 AND "deletedAt" IS NOT NULL"#,
            params![tag_id, workspace_id],
        )?;
        Ok(())
    })
    .await
}

/// `deleteTagOnChannel` (`callApi: false`): the channel's links, the
/// workspace tag of exactly its contacts, then the TagChannel row.
async fn delete_on_channel(db: &Db, tag_id: i64, channel: &TagChannelSummary) -> Result<()> {
    let channel_id = channel.id;
    let mut after: Option<i64> = None;
    loop {
        let page = tx::read(db, move |conn| {
            Ok(tag_channel::list_contact_inbox_ids_for_channel_page(
                conn,
                channel_id,
                after,
                DELETE_CHUNK_SIZE,
            )?)
        })
        .await?;
        let Some(last) = page.last().copied() else {
            break;
        };
        let full = page.len() as i64 == DELETE_CHUNK_SIZE;
        tx::write(db, move |conn| {
            let marks = vec!["?"; page.len()].join(",");
            let mut stmt = conn.prepare(&format!(
                r#"SELECT DISTINCT "contactId" FROM "ContactInbox" WHERE "id" IN ({marks})"#
            ))?;
            let contacts: Vec<i64> = stmt
                .query_map(chatbotx_db::rusqlite::params_from_iter(page.iter()), |r| {
                    r.get(0)
                })?
                .collect::<std::result::Result<_, _>>()?;
            drop(stmt);
            tag_channel::delete_links_for_channel(conn, channel_id, &page)?;
            tag_channel::delete_contact_tags_for_contacts(conn, tag_id, &contacts)?;
            Ok(())
        })
        .await?;
        if !full {
            break;
        }
        after = Some(last);
    }
    tx::write(db, move |conn| {
        Ok(tag_channel::delete_by_id(conn, channel_id)?)
    })
    .await
}

// ---- `syncChannelLabels` ---------------------------------------------------------

/// `handleSyncChannelLabels`: every contact inbox of the integration's inbox
/// gets its channel labels as workspace tags. A failing contact is skipped;
/// one error log row per scan. `pause` = the 200 ms between contacts.
pub async fn sync_channel_labels(
    db: &Db,
    http: &dyn ChannelHttp,
    buses: Option<&EventBuses>,
    data: &Value,
    pause: Duration,
) -> Result<()> {
    let (Some(workspace_id), Some(integration_id)) =
        (id_of(data, "workspaceId"), id_of(data, "integrationId"))
    else {
        return Ok(());
    };
    let channel_type = data["channelType"].as_str().unwrap_or_default();
    let (inbox_id, scan): (i64, Scan) = match channel_type {
        "messenger" => {
            let Some(row) = messenger_by(db, r#"WHERE "id" = ?1"#, integration_id)
                .await?
                .into_iter()
                .next()
            else {
                tracing::warn!(integration_id, "scan: messenger integration not found");
                return Ok(());
            };
            (row.inbox_id, Scan::Messenger(messenger_auth(&row)))
        }
        "zalo" => {
            let Some(row) = zalo_by(db, r#"WHERE "id" = ?1"#, integration_id)
                .await?
                .into_iter()
                .next()
            else {
                tracing::warn!(integration_id, "scan: zalo integration not found");
                return Ok(());
            };
            (row.inbox_id, Scan::Zalo(zalo_token(&row)))
        }
        _ => return Ok(()),
    };
    let mut first_failure: Option<ApiFailure> = None;
    let mut failures = 0usize;
    let mut after: Option<i64> = None;
    loop {
        let batch = tx::read(db, move |conn| {
            Ok(find_many::<ContactInbox>(
                conn,
                r#"WHERE "inboxId" = ?1 AND (?2 IS NULL OR "id" > ?2) ORDER BY "id" ASC LIMIT ?3"#,
                &[&inbox_id, &after, &SCAN_BATCH_SIZE],
            )?)
        })
        .await?;
        let Some(last) = batch.last().map(|ci| ci.id) else {
            break;
        };
        let full = batch.len() as i64 == SCAN_BATCH_SIZE;
        for inbox in batch {
            match scan.labels(http, &inbox.source_id).await {
                Ok(labels) => {
                    for (external_label_id, label_name) in labels {
                        let input = LabelMappingInput {
                            workspace_id,
                            channel_type: channel_type.to_owned(),
                            integration_id,
                            external_label_id,
                            label_name,
                            contact_inbox_id: inbox.id,
                            contact_id: inbox.contact_id,
                        };
                        tx::write(db, move |conn| {
                            Ok(tag_channel::upsert_label_mapping(conn, &input)?)
                        })
                        .await?;
                    }
                }
                Err(failure) => {
                    tracing::warn!(contact_inbox_id = inbox.id, error = %failure.message, "scan: per-user error, skipping");
                    failures += 1;
                    first_failure.get_or_insert(failure);
                }
            }
            tokio::time::sleep(pause).await;
        }
        if !full {
            break;
        }
        after = Some(last);
    }
    if let Some(failure) = first_failure {
        tracing::warn!(
            failures,
            inbox_id,
            provider = channel_type,
            "scan: contacts failed, recording one error log for the run"
        );
        let input =
            ProviderErrorInput::new(channel_type, &workspace_id.to_string(), thrown(&failure));
        log_provider_error(buses, input).await;
    }
    Ok(())
}

enum Scan {
    Messenger(MessengerAuth),
    Zalo(String),
}

impl Scan {
    /// `(externalLabelId, name)` of one user.
    async fn labels(
        &self,
        http: &dyn ChannelHttp,
        source_id: &str,
    ) -> std::result::Result<Vec<(String, String)>, ApiFailure> {
        match self {
            Scan::Messenger(auth) => user_labels(http, auth, source_id).await,
            Scan::Zalo(token) => {
                let detail = zalo_user_detail(http, token, source_id).await?;
                Ok(detail["tags_and_notes_info"]["tag_names"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|n| n.as_str().map(|n| (n.to_owned(), n.to_owned())))
                    .collect())
            }
        }
    }
}
