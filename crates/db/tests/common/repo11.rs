//! Shared fixtures of the P3.2b slices of implementer 6 (google-ads-conversion-event and
//! later), on top of `common/mod.rs` and `common/repo.rs` (both locked).
//!
//! Include after `repo_common`:
//! ```ignore
//! #[path = "common/repo11.rs"]
//! mod repo11;
//! ```

#![allow(dead_code)]

use std::sync::atomic::{AtomicI64, Ordering};

use chatbotx_db::Db;
use chatbotx_db::repo::models::GoogleAdsConversionEvent;
use chatbotx_db::repo::patch::Patch;
use chatbotx_db::repo::{self};
use chatbotx_db::rusqlite::Connection;
use chatbotx_db::rusqlite::types::Value;

use crate::common::{
    ColumnOverride, GOOGLE_ADS_EVENT_OPTIONS, TestDb, fresh_db, google_ads_event_row,
    insert_google_ads_event, seed, seed_workspace,
};

/// The seeded workspace (`seed`, owner = user 1001).
pub const WS: i64 = 2001;
/// A second workspace (owner = user 1002).
pub const OTHER_WS: i64 = 2002;

/// A fresh file with the seed graph and `OTHER_WS`.
pub async fn two_ws_db() -> TestDb {
    let db = fresh_db().await;
    seed(&db).await;
    seed_workspace(&db, 1002, OTHER_WS).await;
    db
}

/// Unix ms of an RFC 3339 instant.
pub fn ms(iso: &str) -> i64 {
    repo::parse_iso(iso).expect("RFC 3339 instant")
}

/// A text override value.
pub fn t(value: &str) -> Option<Value> {
    Some(Value::Text(value.to_owned()))
}

/// An integer override value.
pub fn i(value: i64) -> Option<Value> {
    Some(Value::Integer(value))
}

/// A NULL override value.
pub fn null() -> Option<Value> {
    Some(Value::Null)
}

/// A repository read on a reader (panics on error).
pub async fn read<T: Send + 'static>(
    db: &Db,
    f: impl FnOnce(&Connection) -> repo::Result<T> + Send + 'static,
) -> T {
    crate::repo_common::ok(repo::read(db, f).await)
}

/// A repository write in one committed job (panics on error).
pub async fn write<T: Send + 'static>(
    db: &Db,
    f: impl FnOnce(&Connection) -> repo::Result<T> + Send + 'static,
) -> T {
    crate::repo_common::ok(repo::write(db, move |tx| f(tx)).await)
}

/// Runs SQL statements in a committed write (test setup).
pub async fn exec(db: &Db, sql: impl Into<String>) {
    let sql = sql.into();
    db.write(move |tx| Ok(tx.execute_batch(&sql)?))
        .await
        .expect("exec");
}

/// Inserts one `GoogleAdsConversionEvent` (`common::google_ads_event_row` with
/// `transactionId` = `t-<id>`, status `pending`) with `overrides`.
pub async fn put_event(db: &Db, id: i64, workspace_id: i64, overrides: Vec<ColumnOverride>) {
    db.write(move |tx| {
        insert_google_ads_event(tx, id, workspace_id, &format!("t-{id}"), &overrides)?;
        Ok(())
    })
    .await
    .expect("put event");
}

/// Inserts `count` events with ids `first..first + count` of `workspace_id`, each with
/// `overrides`, in one write.
pub async fn put_events(
    db: &Db,
    first: i64,
    count: i64,
    workspace_id: i64,
    overrides: Vec<ColumnOverride>,
) {
    db.write(move |tx| {
        for id in first..first + count {
            insert_google_ads_event(tx, id, workspace_id, &format!("t-{id}"), &overrides)?;
        }
        Ok(())
    })
    .await
    .expect("put events");
}

static STATS_SEQUENCE: AtomicI64 = AtomicI64::new(10_000);

/// The TS `insertEvent` of `integration/google-ads-conversion-event-stats.test.ts`:
/// status `processed`, action `111` "Purchase" / `PURCHASE`, `occurredAt`
/// `2026-10-02T12:00:00Z`; a `sent` / `processed` row gets `requestId` `req-<n>` and
/// `sentAt`. Returns the new id.
pub async fn stats_event(db: &Db, workspace_id: i64, overrides: Vec<ColumnOverride>) -> i64 {
    let id = STATS_SEQUENCE.fetch_add(1, Ordering::SeqCst);
    let status = overrides
        .iter()
        .find(|(name, _)| *name == "status")
        .and_then(|(_, v)| match v {
            Some(Value::Text(s)) => Some(s.clone()),
            _ => None,
        })
        .unwrap_or_else(|| "processed".to_owned());
    let sent_like = status == "sent" || status == "processed";
    let mut all: Vec<ColumnOverride> = vec![
        ("status", t(&status)),
        ("conversionActionName", t("Purchase")),
        ("conversionActionCategory", t("PURCHASE")),
        ("occurredAt", i(ms("2026-10-02T12:00:00Z"))),
        (
            "requestId",
            if sent_like {
                t(&format!("req-{id}"))
            } else {
                null()
            },
        ),
        (
            "sentAt",
            if sent_like {
                i(ms("2026-10-02T12:00:00Z"))
            } else {
                null()
            },
        ),
    ];
    all.extend(overrides);
    db.write(move |tx| {
        insert_google_ads_event(tx, id, workspace_id, &format!("stats-{id}"), &all)?;
        Ok(())
    })
    .await
    .expect("stats event");
    id
}

/// The stored event `id`.
pub async fn event(db: &Db, id: i64) -> Option<GoogleAdsConversionEvent> {
    read(db, move |c| repo::find_one(c, "\"id\" = ?1", &[&id])).await
}

/// The `insertIgnoreDuplicate` values of `common::google_ads_event_row` (no `id`; the
/// `options` snapshot of `GOOGLE_ADS_EVENT_OPTIONS`) with `overrides`.
pub fn event_values(
    workspace_id: i64,
    transaction_id: &str,
    overrides: &[ColumnOverride],
) -> Patch {
    let (columns, values) = google_ads_event_row(0, workspace_id, transaction_id, overrides);
    columns
        .into_iter()
        .zip(values)
        .filter(|(name, _)| *name != "id")
        .collect()
}

/// `GOOGLE_ADS_EVENT_OPTIONS` parsed.
pub fn event_options() -> serde_json::Value {
    serde_json::from_str(GOOGLE_ADS_EVENT_OPTIONS).expect("options")
}

/// Inserts one `FacebookMarketingMessage` of `workspace_id` (`campaignId` = `meta-<id>`,
/// ad account `act_1`, page `page-1`, USD daily budget 100, content `{"v":0}`) with
/// `status`, then applies `set` (an SQL SET list, empty for none).
pub async fn put_campaign(db: &Db, id: i64, workspace_id: i64, status: &str, set: &str) {
    let mut sql = format!(
        r#"INSERT INTO "FacebookMarketingMessage" ("id", "workspaceId", "name", "pageId", "adAccountId", "currency", "currencyOffset", "budgetType", "budgetMinorUnits", "content", "campaignId", "status") VALUES ({id}, {workspace_id}, 'c{id}', 'page-1', 'act_1', 'USD', 100, 'daily', 100, '{{"v":0}}', 'meta-{id}', '{status}');"#
    );
    if !set.is_empty() {
        sql.push_str(&format!(
            r#"UPDATE "FacebookMarketingMessage" SET {set} WHERE "id" = {id};"#
        ));
    }
    exec(db, sql).await;
}

/// Inserts one `FacebookMarketingMessageRecipient` of `campaign_row_id` (workspace `WS`,
/// `subscriptionToken` = `tok-<id>`) with `status`, then applies `set`.
pub async fn put_recipient(db: &Db, id: i64, campaign_row_id: i64, status: &str, set: &str) {
    let mut sql = format!(
        r#"INSERT INTO "FacebookMarketingMessageRecipient" ("id", "workspaceId", "campaignRowId", "subscriptionToken", "status") VALUES ({id}, {WS}, {campaign_row_id}, 'tok-{id}', '{status}');"#
    );
    if !set.is_empty() {
        sql.push_str(&format!(
            r#"UPDATE "FacebookMarketingMessageRecipient" SET {set} WHERE "id" = {id};"#
        ));
    }
    exec(db, sql).await;
}

/// The stored `FacebookMarketingMessage` `id`.
pub async fn campaign(db: &Db, id: i64) -> chatbotx_db::repo::models::FacebookMarketingMessage {
    read(db, move |c| repo::find_one(c, "\"id\" = ?1", &[&id]))
        .await
        .expect("campaign row")
}

/// The stored `FacebookMarketingMessageRecipient` `id`.
pub async fn recipient(
    db: &Db,
    id: i64,
) -> chatbotx_db::repo::models::FacebookMarketingMessageRecipient {
    read(db, move |c| repo::find_one(c, "\"id\" = ?1", &[&id]))
        .await
        .expect("recipient row")
}

static NUMBER_SEQUENCE: AtomicI64 = AtomicI64::new(7_000);

/// The ids `seed_number` created.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SeededNumber {
    pub workspace_id: i64,
    pub integration_id: i64,
    pub inbox_id: i64,
    pub connection_id: Option<i64>,
}

/// The TS `seedNumber` of `integration/integration-whatsapp-lifecycle-targets.test.ts`: a
/// new workspace of `owner_id` (in `tenant_id`, else the root tenant), a whatsapp inbox, an
/// `IntegrationWhatsapp` on `waba_id` (`auth.clientId` = `app_id`, `auth.metadata.isManual`
/// when `is_manual`) and, with `with_connection`, a channel `Connection` of the inbox.
#[allow(clippy::too_many_arguments)]
pub async fn seed_number(
    db: &Db,
    owner_id: i64,
    tenant_id: Option<i64>,
    waba_id: &str,
    phone_number_id: &str,
    is_manual: bool,
    with_connection: bool,
    app_id: Option<&str>,
) -> SeededNumber {
    let n = NUMBER_SEQUENCE.fetch_add(1, Ordering::SeqCst);
    let mut auth = serde_json::Map::new();
    if let Some(app) = app_id {
        auth.insert("clientId".to_owned(), serde_json::json!(app));
    }
    if is_manual {
        auth.insert(
            "metadata".to_owned(),
            serde_json::json!({ "isManual": true }),
        );
    }
    let auth = serde_json::Value::Object(auth).to_string();
    let (waba, phone) = (waba_id.to_owned(), phone_number_id.to_owned());
    let connection_id = with_connection.then_some(n + 300_000);
    db.write(move |tx| {
        tx.execute(
            r#"INSERT INTO "Workspace" ("id", "name", "ownerId", "tenantId") VALUES (?1, ?2, ?3, ?4)"#,
            chatbotx_db::rusqlite::params![n, format!("lifecycle-{phone}"), owner_id, tenant_id.unwrap_or(1)],
        )?;
        tx.execute(
            r#"INSERT INTO "Inbox" ("id", "name", "channel", "sourceId", "workspaceId") VALUES (?1, ?2, 'whatsapp', ?2, ?3)"#,
            chatbotx_db::rusqlite::params![n + 100_000, phone, n],
        )?;
        tx.execute(
            r#"INSERT INTO "IntegrationWhatsapp" ("id", "auth", "phoneNumberId", "wabaId", "businessId", "name", "workspaceId", "inboxId") VALUES (?1, ?2, ?3, ?4, 'business', ?3, ?5, ?6)"#,
            chatbotx_db::rusqlite::params![n + 200_000, auth, phone, waba, n, n + 100_000],
        )?;
        if let Some(cid) = connection_id {
            tx.execute(
                r#"INSERT INTO "Connection" ("id", "workspaceId", "provider", "kind", "channel", "inboxId", "sourceId", "displayName") VALUES (?1, ?2, 'whatsapp', 'channel', 'whatsapp', ?3, ?4, ?4)"#,
                chatbotx_db::rusqlite::params![cid, n, n + 100_000, phone],
            )?;
        }
        Ok(())
    })
    .await
    .expect("seed number");
    SeededNumber {
        workspace_id: n,
        integration_id: n + 200_000,
        inbox_id: n + 100_000,
        connection_id,
    }
}

static USER_SEQUENCE: AtomicI64 = AtomicI64::new(5_000);

/// The TS `seedUser`: a new `User`; returns its id.
pub async fn seed_user(db: &Db) -> i64 {
    let id = USER_SEQUENCE.fetch_add(1, Ordering::SeqCst);
    exec(
        db,
        format!(
            r#"INSERT INTO "User" ("id", "email") VALUES ({id}, 'lifecycle-{id}@example.test')"#
        ),
    )
    .await;
    id
}

/// A reseller `Tenant` owned by `owner_id`; returns its id.
pub async fn seed_tenant(db: &Db, owner_id: i64) -> i64 {
    let id = owner_id + 10_000;
    exec(
        db,
        format!(r#"INSERT INTO "Tenant" ("id", "ownerId") VALUES ({id}, {owner_id})"#),
    )
    .await;
    id
}

/// A `whatsapp` `PlatformCredential` of `user_id`.
pub async fn seed_platform_credential(db: &Db, user_id: i64, use_platform_credential: bool) {
    let flag = i64::from(use_platform_credential);
    exec(
        db,
        format!(
            r#"INSERT INTO "PlatformCredential" ("id", "userId", "type", "publicConfig", "value", "usePlatformCredential") VALUES ({id}, {user_id}, 'whatsapp', '{{}}', '{{}}', {flag})"#,
            id = user_id + 20_000
        ),
    )
    .await;
}
