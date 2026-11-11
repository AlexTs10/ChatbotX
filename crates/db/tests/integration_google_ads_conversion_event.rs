//! Port of `packages/database/__tests__/integration/google-ads-conversion-event.test.ts`, the
//! schema cases (P2.5 spec section 3.1, rows 9.1-9.12). The repository cases
//! (`insertIgnoreDuplicate` duplicates, options, claim / completion / redrive, the
//! multi-connection race) belong to the Google Ads repository unit (P3).
//! Class C: runs against a migrated SQLite file.
//!
//! The TS `eventValues` row is `common::google_ads_event_row` (timestamps as unix ms,
//! `options` as JSON text). Every insert that must fail runs as its own write job, which
//! rolls back like the original's `withRolledBackTransaction`; its `DbError` carries the
//! mapped SQLSTATE and constraint name.

mod common;

use chatbotx_db::rusqlite::types::Value;
use chatbotx_db::{Db, DbError};
use common::{
    ColumnOverride, TestDb, column, fresh_db, insert_google_ads_event,
    insert_google_ads_event_ignore_duplicate, insert_integration_google_ads, seed_integration,
    seed_workspace, table_sql,
};

const WORKSPACE_ID: i64 = 2001;
const EVENT_ID: i64 = 9001;

/// The TS `seedWorkspace` (committed; each test has its own database file).
async fn seeded_db() -> TestDb {
    let db = fresh_db().await;
    seed_workspace(&db, 1001, WORKSPACE_ID).await;
    db
}

fn text(value: &str) -> Option<Value> {
    Some(Value::Text(value.to_owned()))
}

/// The `insertIgnoreDuplicate` insert of `eventValues(workspaceId, transaction_id)` with
/// `overrides`, returning the column `returning` (committed).
async fn insert_ignore_duplicate(
    db: &Db,
    transaction_id: &'static str,
    overrides: Vec<ColumnOverride>,
    returning: &'static str,
) -> Option<Value> {
    db.write(move |tx| {
        insert_google_ads_event_ignore_duplicate(
            tx,
            EVENT_ID,
            WORKSPACE_ID,
            transaction_id,
            &overrides,
            returning,
        )
    })
    .await
    .expect("insertIgnoreDuplicate")
}

/// `tx.insert(googleAdsConversionEventModel).values({...eventValues(...), ...overrides})`
/// in its own write job; the insert must fail.
async fn rejected_insert(
    db: &Db,
    transaction_id: &'static str,
    overrides: Vec<ColumnOverride>,
) -> DbError {
    db.write(move |tx| {
        insert_google_ads_event(tx, EVENT_ID, WORKSPACE_ID, transaction_id, &overrides)?;
        Ok(())
    })
    .await
    .expect_err("the insert must be rejected")
}

/// The insert fails with SQLSTATE `23514` and the CHECK constraint `name`.
async fn assert_check_rejects(overrides: Vec<ColumnOverride>, name: &str) {
    let db = seeded_db().await;
    let error = rejected_insert(&db, "gads-check", overrides).await;
    assert_eq!(error.sqlstate(), Some("23514"), "{error}");
    assert_eq!(error.constraint(), Some(name), "{error}");
}

/// Port of: packages/database/__tests__/integration/google-ads-conversion-event.test.ts:120 "GoogleAdsConversionEvent > occurredAt may precede the click receipt (no CHECK; receipt is a proxy)"
///
/// Adapted (raw insert instead of the repository): the `insertIgnoreDuplicate` SQL with
/// `occurredAt` 2026-09-01T00:00:00Z (before the 2026-10-01 receipt) returns that value.
#[tokio::test]
async fn occurred_at_may_precede_the_click_receipt_no_check_receipt_is_a_proxy() {
    let db = seeded_db().await;
    let occurred_at: i64 = 1_788_220_800_000; // 2026-09-01T00:00:00Z
    let stored = insert_ignore_duplicate(
        &db,
        "gads-backdated",
        vec![("occurredAt", Some(Value::Integer(occurred_at)))],
        "occurredAt",
    )
    .await;
    assert_eq!(stored, Some(Value::Integer(occurred_at)));
}

/// Port of: packages/database/__tests__/integration/google-ads-conversion-event.test.ts:162 "GoogleAdsConversionEvent > the channel column is plain text, so a new channel needs no migration"
///
/// Adapted (raw insert instead of the repository): `channel = 'zalo'` is stored and returned,
/// and the table SQL has no CHECK on `"channel"`.
#[tokio::test]
async fn the_channel_column_is_plain_text_so_a_new_channel_needs_no_migration() {
    let db = seeded_db().await;
    let stored = insert_ignore_duplicate(
        &db,
        "gads-channel",
        vec![("channel", text("zalo"))],
        "channel",
    )
    .await;
    assert_eq!(stored, text("zalo"));

    let sql = table_sql(&db, "GoogleAdsConversionEvent")
        .await
        .expect("GoogleAdsConversionEvent is missing");
    let channel_line = sql
        .lines()
        .find(|l| l.trim_start().starts_with("\"channel\""))
        .expect("the channel column definition");
    assert!(
        !channel_line.contains("CHECK"),
        "channel must be plain text: {channel_line}"
    );
    assert!(
        !sql.contains("\"channel\" IN"),
        "no CHECK may restrict channel: {sql}"
    );
}

/// Port of: packages/database/__tests__/integration/google-ads-conversion-event.test.ts:420 "GoogleAdsConversionEvent > CHECK constraints reject an impossible event"
#[tokio::test]
async fn check_constraints_reject_an_impossible_event() {
    let db = seeded_db().await;

    let sent = rejected_insert(&db, "gads-3", vec![("status", text("sent"))]).await;
    assert_eq!(sent.sqlstate(), Some("23514"), "{sent}");
    assert_eq!(
        sent.constraint(),
        Some("GoogleAdsConversionEvent_sent_requires_request_check"),
        "{sent}"
    );

    let value = rejected_insert(&db, "gads-4", vec![("value", text("10"))]).await;
    assert_eq!(value.sqlstate(), Some("23514"), "{value}");
    assert_eq!(
        value.constraint(),
        Some("GoogleAdsConversionEvent_value_currency_check"),
        "{value}"
    );
}

/// Port of: packages/database/__tests__/integration/google-ads-conversion-event.test.ts:460 "GoogleAdsConversionEvent > uploadMethod has no database default, so an insert must write it"
#[tokio::test]
async fn upload_method_has_no_database_default_so_an_insert_must_write_it() {
    let db = seeded_db().await;
    let error = rejected_insert(&db, "gads-um-missing", vec![("uploadMethod", None)]).await;
    assert_eq!(error.sqlstate(), Some("23502"), "{error}");
    assert_eq!(
        error.detail(),
        Some("NOT NULL constraint failed: GoogleAdsConversionEvent.uploadMethod"),
        "{error}"
    );
    assert_eq!(
        column(&db, "GoogleAdsConversionEvent", "uploadMethod")
            .await
            .dflt_value,
        None
    );
}

/// Port of: packages/database/__tests__/integration/google-ads-conversion-event.test.ts:475 "GoogleAdsConversionEvent > CHECK rejects an unknown uploadMethod"
#[tokio::test]
async fn check_rejects_an_unknown_upload_method() {
    let db = seeded_db().await;
    let error = rejected_insert(&db, "gads-um-bad", vec![("uploadMethod", text("soap"))]).await;
    assert_eq!(error.sqlstate(), Some("23514"), "{error}");
    assert_eq!(
        error.constraint(),
        Some("GoogleAdsConversionEvent_uploadMethod_check"),
        "{error}"
    );
}

/// Port of: packages/database/__tests__/integration/google-ads-conversion-event.test.ts:512 "GoogleAdsConversionEvent > CHECK rejects sending without a claim"
#[tokio::test]
async fn check_rejects_sending_without_a_claim() {
    assert_check_rejects(
        vec![("status", text("sending"))],
        "GoogleAdsConversionEvent_sending_requires_claim_check",
    )
    .await;
}

/// Port of: packages/database/__tests__/integration/google-ads-conversion-event.test.ts:512 "GoogleAdsConversionEvent > CHECK rejects failureStage on a non-failed row"
#[tokio::test]
async fn check_rejects_failure_stage_on_a_non_failed_row() {
    assert_check_rejects(
        vec![("failureStage", text("delivery"))],
        "GoogleAdsConversionEvent_failure_stage_check",
    )
    .await;
}

/// Port of: packages/database/__tests__/integration/google-ads-conversion-event.test.ts:512 "GoogleAdsConversionEvent > CHECK rejects processingStatus on a pending row"
#[tokio::test]
async fn check_rejects_processing_status_on_a_pending_row() {
    assert_check_rejects(
        vec![("processingStatus", text("processing"))],
        "GoogleAdsConversionEvent_processing_status_check",
    )
    .await;
}

/// Port of: packages/database/__tests__/integration/google-ads-conversion-event.test.ts:512 "GoogleAdsConversionEvent > CHECK rejects negative value"
#[tokio::test]
async fn check_rejects_negative_value() {
    assert_check_rejects(
        vec![("value", text("-1")), ("currency", text("USD"))],
        "GoogleAdsConversionEvent_value_non_negative_check",
    )
    .await;
}

/// Port of: packages/database/__tests__/integration/google-ads-conversion-event.test.ts:512 "GoogleAdsConversionEvent > CHECK rejects short click id"
#[tokio::test]
async fn check_rejects_short_click_id() {
    assert_check_rejects(
        vec![("clickId", text("short"))],
        "GoogleAdsConversionEvent_clickId_length_check",
    )
    .await;
}

/// Port of: packages/database/__tests__/integration/google-ads-conversion-event.test.ts:512 "GoogleAdsConversionEvent > CHECK rejects negative attempt"
#[tokio::test]
async fn check_rejects_negative_attempt() {
    assert_check_rejects(
        vec![("attempt", Some(Value::Integer(-1)))],
        "GoogleAdsConversionEvent_attempt_check",
    )
    .await;
}

/// Port of: packages/database/__tests__/integration/google-ads-conversion-event.test.ts:578 "IntegrationGoogleAds > one satellite per workspace (engine duplicateConstraint name)"
///
/// Both satellites are inserted in one write job (the original's rolled-back transaction);
/// the second one fails with `23505` and the unique index name.
#[tokio::test]
async fn one_satellite_per_workspace_engine_duplicate_constraint_name() {
    let db = seeded_db().await;
    let error = db
        .write(|tx| {
            for (n, customer_id) in [(0_i64, "1111111111"), (1, "2222222222")] {
                let integration_id = seed_integration(tx, 5001 + n, WORKSPACE_ID, "googleAds")?;
                insert_integration_google_ads(
                    tx,
                    6001 + n,
                    WORKSPACE_ID,
                    integration_id,
                    customer_id,
                )?;
            }
            Ok(())
        })
        .await
        .expect_err("the second satellite must be rejected");
    assert_eq!(error.sqlstate(), Some("23505"), "{error}");
    assert_eq!(
        error.constraint(),
        Some("IntegrationGoogleAds_workspaceId_key"),
        "{error}"
    );
}
