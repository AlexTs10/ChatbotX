//! Port of `packages/database/__tests__/inbound-webhook-partial.test.ts`, the DDL cases
//! (P2.4 spec section 3.1, rows 7.1-7.7: `:207`, `:218`-`:263`). The other declarations test
//! zod partials and belong to the partials port. Class C: runs against a migrated SQLite file.

mod common;

use common::{
    column, column_in_list, fresh_db, in_list_values, index, indexes, schema_sql, table_sql,
};

/// `inboundWebhookRequestStatuses.options` (zod enum of `src/partials`), same order.
// TODO(P2.4 test port): use the Rust `INBOUND_WEBHOOK_REQUEST_STATUSES` of the partials port once it exists.
const INBOUND_WEBHOOK_REQUEST_STATUSES: &[&str] = &[
    "received",
    "processing",
    "processed",
    "completedWithErrors",
    "skipped",
    "failed",
];

/// Port of: packages/database/__tests__/inbound-webhook-partial.test.ts:207 "request statuses > the database enum carries the same values"
///
/// Adapted: the pgEnum is the CHECK `"status" IN (...)` list of `InboundWebhookRequest`.
#[tokio::test]
async fn the_database_enum_carries_the_same_values() {
    let db = fresh_db().await;
    let sql = table_sql(&db, "InboundWebhookRequest")
        .await
        .expect("table InboundWebhookRequest");
    let list = column_in_list(&sql, "status").expect("CHECK (\"status\" IN (...))");
    assert_eq!(in_list_values(&list), INBOUND_WEBHOOK_REQUEST_STATUSES);
}

/// Port of: packages/database/__tests__/inbound-webhook-partial.test.ts:218 "InboundWebhook tables > the webhook token is unique"
#[tokio::test]
async fn the_webhook_token_is_unique() {
    let db = fresh_db().await;
    let idx = index(&db, "InboundWebhook", "InboundWebhook_token_key").await;
    assert!(idx.unique, "InboundWebhook_token_key must be unique");
}

/// Port of: packages/database/__tests__/inbound-webhook-partial.test.ts:225 "InboundWebhook tables > a webhook starts inactive"
#[tokio::test]
async fn a_webhook_starts_inactive() {
    let db = fresh_db().await;
    let active = column(&db, "InboundWebhook", "active").await;
    assert_eq!(active.dflt_value.as_deref(), Some("0"));
    assert!(active.notnull, "InboundWebhook.active must be NOT NULL");
}

/// Port of: packages/database/__tests__/inbound-webhook-partial.test.ts:229 "InboundWebhook tables > the idempotency key is unique per webhook only when present"
#[tokio::test]
async fn the_idempotency_key_is_unique_per_webhook_only_when_present() {
    let db = fresh_db().await;
    let name = "InboundWebhookRequest_inboundWebhookId_idempotencyKey_key";
    let idx = index(&db, "InboundWebhookRequest", name).await;
    assert!(idx.unique, "{name} must be unique");
    assert!(idx.partial, "{name} must have a WHERE clause");
    let sql = schema_sql(&db, "index", name).await.expect("index SQL");
    assert!(
        sql.trim_end()
            .ends_with(r#"WHERE "idempotencyKey" IS NOT NULL"#),
        "{name}: {sql}"
    );
}

/// Port of: packages/database/__tests__/inbound-webhook-partial.test.ts:239 "InboundWebhook tables > indexes cover every SET NULL foreign key and both cron scans"
#[tokio::test]
async fn indexes_cover_every_set_null_foreign_key_and_both_cron_scans() {
    let db = fresh_db().await;
    let names: Vec<String> = indexes(&db, "InboundWebhookRequest")
        .await
        .into_iter()
        .map(|i| i.name)
        .collect();
    for expected in [
        "InboundWebhookRequest_contactId_idx",
        "InboundWebhookRequest_inboxId_idx",
        "InboundWebhookRequest_createdAt_received_idx",
        "InboundWebhookRequest_claimedAt_processing_idx",
        "InboundWebhookRequest_createdAt_idx",
    ] {
        assert!(
            names.iter().any(|n| n == expected),
            "missing index {expected}: {names:?}"
        );
    }
}

/// Port of: packages/database/__tests__/inbound-webhook-partial.test.ts:251 "InboundWebhook tables > the scan indexes are partial on the status they serve"
#[tokio::test]
async fn the_scan_indexes_are_partial_on_the_status_they_serve() {
    let db = fresh_db().await;
    let partials: Vec<_> = indexes(&db, "InboundWebhookRequest")
        .await
        .into_iter()
        .filter(|i| i.name.ends_with("_received_idx") || i.name.ends_with("_processing_idx"))
        .collect();
    assert_eq!(partials.len(), 2, "{partials:?}");
    for idx in &partials {
        assert!(idx.partial, "{} must have a WHERE clause", idx.name);
    }
}

/// Port of: packages/database/__tests__/inbound-webhook-partial.test.ts:263 "InboundWebhook tables > jsonb columns have no DB default (they are written explicitly)"
#[tokio::test]
async fn jsonb_columns_have_no_db_default_they_are_written_explicitly() {
    let db = fresh_db().await;
    for name in ["identifiers", "fieldMappings", "actions"] {
        let col = column(&db, "InboundWebhook", name).await;
        assert_eq!(
            col.dflt_value, None,
            "InboundWebhook.{name} must have no default"
        );
        assert!(col.notnull, "InboundWebhook.{name} must be NOT NULL");
    }
}
