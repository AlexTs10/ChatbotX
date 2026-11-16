//! Ports of the Meta Catalog route-side tests:
//! `packages/business/__tests__/integration-meta-catalog.service.test.ts`
//! (connection read, disconnect, resolveAuth), `meta-catalog-sync-run-service`
//! (`list`), `meta-catalog-operation-service` and the builder
//! `__tests__/meta-catalog-operations.test.ts`. Adapted: the mocked
//! repositories / transactions become real rows, `queue.add` a recording
//! enqueuer and the Graph client a fake.

mod common;
mod x1_support;

use std::sync::Mutex;

use chatbotx_business::errors::{BusinessError, Result};
use chatbotx_business::meta_catalog::{
    self as mc, Business, Catalog, Deps, ENGLISH_REASONS, MetaGraph, PushScope, Reasons, StoredAuth,
};
use chatbotx_core::encryption::{Encryption, Keys};
use chatbotx_db::Db;
use chatbotx_db::rusqlite::{Connection, params};
use chatbotx_jobs::QueueName;
use common::{TestDb, WS, product, setup, two_workspaces};
use serde_json::{Value, json};
use x1_support::RecordingEnqueuer;

const CONNECTION: i64 = 501;
const KEY: &str = "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f";

fn encryption() -> Encryption {
    Encryption::new(Keys::new(KEY).expect("key"))
}

/// A bound connection (`catalog_id`) with an encrypted token.
fn connection(
    tx: &Connection,
    catalog_id: Option<&str>,
    import_status: &str,
) -> std::result::Result<(), chatbotx_db::DbError> {
    let auth = encryption()
        .encrypt_object(&json!({ "accessToken": "tok-1", "version": "v23.0" }), None)
        .expect("encrypt");
    tx.execute(
        r#"INSERT INTO "Integration" ("id", "workspaceId", "integrationType") VALUES (500, ?1, 'metaCatalog')"#,
        params![WS],
    )?;
    tx.execute(
        r#"INSERT INTO "IntegrationMetaCatalog" ("id", "workspaceId", "integrationId", "catalogId", "encryptedAuth", "status", "importStatus")
           VALUES (?1, ?2, 500, ?3, ?4, 'active', ?5)"#,
        params![
            CONNECTION,
            WS,
            catalog_id,
            serde_json::to_string(&auth).expect("json"),
            import_status
        ],
    )?;
    Ok(())
}

fn run(
    tx: &Connection,
    id: i64,
    status: &str,
    skipped: Value,
) -> std::result::Result<(), chatbotx_db::DbError> {
    tx.execute(
        r#"INSERT INTO "MetaCatalogSyncRun" ("id", "workspaceId", "integrationMetaCatalogId", "scope", "direction", "status", "skippedItems")
           VALUES (?1, ?2, ?3, 'all', 'push', ?4, ?5)"#,
        params![id, WS, CONNECTION, status, skipped.to_string()],
    )?;
    Ok(())
}

async fn db_with(catalog_id: Option<&'static str>, import_status: &'static str) -> TestDb {
    let db = two_workspaces().await;
    setup(&db, move |tx| connection(tx, catalog_id, import_status)).await;
    db
}

async fn text(db: &Db, sql: &'static str) -> Option<String> {
    db.read(move |c| Ok(c.query_row(sql, [], |r| r.get::<_, Option<String>>(0))?))
        .await
        .expect("read")
}

async fn count(db: &Db, sql: &'static str) -> i64 {
    db.read(move |c| Ok(c.query_row(sql, [], |r| r.get::<_, i64>(0))?))
        .await
        .expect("read")
}

#[derive(Default)]
struct FakeGraph {
    get_catalog: Mutex<Vec<String>>,
}

impl MetaGraph for FakeGraph {
    async fn get_catalog(&self, auth: &StoredAuth, catalog_id: &str) -> Result<Catalog> {
        assert_eq!(auth.access_token, "tok-1");
        self.get_catalog
            .lock()
            .unwrap()
            .push(catalog_id.to_string());
        Ok(Catalog {
            id: catalog_id.to_string(),
            name: Some(format!("Catalog {catalog_id}")),
            business_id: Some("77".into()),
        })
    }

    async fn list_businesses(&self, _auth: &StoredAuth) -> Result<Vec<Business>> {
        Ok(vec![])
    }

    async fn create_catalog(
        &self,
        _auth: &StoredAuth,
        business_id: &str,
        name: &str,
    ) -> Result<Catalog> {
        Ok(Catalog {
            id: "900".into(),
            name: Some(name.into()),
            business_id: Some(business_id.into()),
        })
    }
}

fn exception_code(error: &BusinessError) -> (Option<&str>, Option<u16>) {
    (error.code(), error.status())
}

// ---- integrationMetaCatalogService -----------------------------------------

// Adapted: `findByWorkspaceId` is read through `getMetaCatalogState`.

#[tokio::test]
async fn integration_meta_catalog_service_only_exposes_an_active_workspace_connection() {
    let db = db_with(Some("9"), "idle").await;
    let (found, _) = mc::get_state(&db, WS).await.expect("state");
    assert_eq!(found.map(|row| row.id), Some(CONNECTION));
    setup(&db, |tx| {
        tx.execute(r#"UPDATE "IntegrationMetaCatalog" SET "deletedAt" = 1"#, [])?;
        Ok(())
    })
    .await;
    let (found, _) = mc::get_state(&db, WS).await.expect("state");
    assert!(found.is_none());
}

#[tokio::test]
async fn integration_meta_catalog_service_soft_deletes_credentials_only_when_no_sync_or_import_is_active()
 {
    let db = db_with(Some("9"), "succeeded").await;
    mc::disconnect(&db, WS).await.expect("disconnect");
    let row = db
        .read(|c| {
            Ok(c.query_row(
                r#"SELECT "deletedAt" IS NOT NULL, "encryptedAuth", "importError", "importStatus", "status", "tokenExpiresAt" FROM "IntegrationMetaCatalog""#,
                [],
                |r| {
                    Ok((
                        r.get::<_, bool>(0)?,
                        r.get::<_, Option<String>>(1)?,
                        r.get::<_, Option<String>>(2)?,
                        r.get::<_, String>(3)?,
                        r.get::<_, String>(4)?,
                        r.get::<_, Option<i64>>(5)?,
                    ))
                },
            )?)
        })
        .await
        .expect("read");
    assert_eq!(
        row,
        (
            true,
            None,
            Some("Meta Catalog was disconnected".into()),
            "failed".into(),
            "invalid".into(),
            None
        )
    );
}

#[tokio::test]
async fn integration_meta_catalog_service_refuses_to_disconnect_while_a_sync_run_is_active() {
    let db = db_with(Some("9"), "idle").await;
    setup(&db, |tx| run(tx, 1, "running", json!([]))).await;
    let error = mc::disconnect(&db, WS).await.expect_err("busy");
    assert_eq!(
        exception_code(&error),
        (Some("metaCatalogSyncAlreadyRunning"), Some(409))
    );
    assert_eq!(
        count(
            &db,
            r#"SELECT count(*) FROM "IntegrationMetaCatalog" WHERE "deletedAt" IS NULL"#
        )
        .await,
        1
    );
}

#[tokio::test]
async fn integration_meta_catalog_service_refuses_to_disconnect_while_a_legacy_import_is_active() {
    let db = db_with(Some("9"), "running").await;
    let error = mc::disconnect(&db, WS).await.expect_err("busy");
    assert_eq!(
        exception_code(&error),
        (Some("metaCatalogSyncAlreadyRunning"), Some(409))
    );
    assert_eq!(
        count(
            &db,
            r#"SELECT count(*) FROM "IntegrationMetaCatalog" WHERE "deletedAt" IS NULL"#
        )
        .await,
        1
    );
}

#[tokio::test]
async fn integration_meta_catalog_service_refuses_to_resolve_credentials_that_were_cleared_on_disconnect()
 {
    let db = db_with(Some("9"), "idle").await;
    let auth = mc::resolve_auth(&db, &encryption(), CONNECTION)
        .await
        .expect("auth");
    assert_eq!(auth.access_token, "tok-1");
    setup(&db, |tx| {
        tx.execute(
            r#"UPDATE "IntegrationMetaCatalog" SET "encryptedAuth" = NULL"#,
            [],
        )?;
        Ok(())
    })
    .await;
    let error = mc::resolve_auth(&db, &encryption(), CONNECTION)
        .await
        .expect_err("cleared");
    assert_eq!(error.code(), Some("metaCatalogReconnectRequired"));
}

// ---- metaCatalogSyncRunService.list ------------------------------------------

#[tokio::test]
async fn meta_catalog_sync_run_service_list_enriches_skipped_items_with_the_product_s_current_name()
{
    let db = db_with(Some("9"), "idle").await;
    setup(&db, |tx| {
        product(tx, 11, WS, None, None)?;
        run(
            tx,
            1,
            "succeeded",
            json!([
                { "productId": "11", "reason": "missingImage" },
                { "productId": "12", "reason": "missingStoreUrl" },
            ]),
        )
    })
    .await;
    let history = mc::list_history(&db, WS).await.expect("list");
    // Adapted: `productName: undefined` is an absent key in the stored JSON.
    assert_eq!(
        history[0].skipped_items,
        vec![
            json!({ "productId": "11", "reason": "missingImage", "productName": "Product 11" }),
            json!({ "productId": "12", "reason": "missingStoreUrl" }),
        ]
    );
}

#[tokio::test]
async fn meta_catalog_sync_run_service_list_skips_the_product_lookup_when_no_run_has_skipped_items()
{
    // Adapted: the lookup with no ids is not observable; the history is.
    let db = db_with(Some("9"), "idle").await;
    setup(&db, |tx| run(tx, 1, "succeeded", json!([]))).await;
    let history = mc::list_history(&db, WS).await.expect("list");
    assert_eq!(history.len(), 1);
    assert!(history[0].skipped_items.is_empty());
}

// ---- metaCatalogOperationService ---------------------------------------------

fn catalog(id: &str, name: Option<&str>) -> Catalog {
    Catalog {
        id: id.into(),
        name: name.map(str::to_string),
        business_id: None,
    }
}

#[tokio::test]
async fn meta_catalog_operation_service_reserves_an_import_run_before_queueing_the_connection_state()
 {
    let db = db_with(Some("catalog-old"), "idle").await;
    let (connection, run) = mc::start_import(&db, WS, catalog("catalog-new", Some("New catalog")))
        .await
        .expect("start");
    assert_eq!(connection.id, CONNECTION);
    assert_eq!(
        (
            connection.catalog_id.as_deref(),
            connection.catalog_name.as_deref(),
            connection.import_status.as_str()
        ),
        (Some("catalog-new"), Some("New catalog"), "queued")
    );
    assert_eq!(
        (
            run.integration_meta_catalog_id,
            run.direction.as_str(),
            run.catalog_id.as_deref(),
            run.scope.as_str()
        ),
        (CONNECTION, "import", Some("catalog-new"), "all")
    );
}

#[tokio::test]
async fn meta_catalog_operation_service_does_not_mutate_import_state_when_reserving_the_run_fails()
{
    let db = db_with(Some("catalog-old"), "idle").await;
    setup(&db, |tx| run(tx, 1, "queued", json!([]))).await;
    let error = mc::start_import(&db, WS, catalog("catalog-new", None))
        .await
        .expect_err("active run");
    assert_eq!(error.code(), Some("metaCatalogSyncAlreadyRunning"));
    assert_eq!(
        text(
            &db,
            r#"SELECT "catalogId" || ':' || "importStatus" FROM "IntegrationMetaCatalog""#
        )
        .await,
        Some("catalog-old:idle".into())
    );
}

#[tokio::test]
async fn meta_catalog_operation_service_reserves_a_push_run_before_rebinding_its_catalog() {
    let db = db_with(Some("catalog-old"), "idle").await;
    setup(&db, |tx| product(tx, 11, WS, None, None).map(|_| ())).await;
    let run = mc::start_push(
        &db,
        WS,
        catalog("catalog-new", Some("New catalog")),
        PushScope {
            scope: "selected".into(),
            category_id: None,
            selected_product_ids: vec![11],
        },
    )
    .await
    .expect("push");
    assert_eq!(
        (
            run.integration_meta_catalog_id,
            run.catalog_id.as_deref(),
            run.scope.as_str(),
            run.direction.as_str()
        ),
        (CONNECTION, Some("catalog-new"), "selected", "push")
    );
    assert_eq!(run.selected_product_ids.0, json!(["11"]));
    assert_eq!(
        text(
            &db,
            r#"SELECT "catalogId" || ':' || "catalogName" FROM "IntegrationMetaCatalog""#
        )
        .await,
        Some("catalog-new:New catalog".into())
    );
}

#[tokio::test]
async fn meta_catalog_operation_service_does_not_start_a_push_while_a_legacy_import_is_active() {
    let db = db_with(Some("catalog-old"), "queued").await;
    let error = mc::start_push(
        &db,
        WS,
        catalog("catalog-new", None),
        PushScope {
            scope: "all".into(),
            ..PushScope::default()
        },
    )
    .await
    .expect_err("legacy import");
    assert_eq!(error.code(), Some("metaCatalogSyncAlreadyRunning"));
    assert_eq!(
        count(&db, r#"SELECT count(*) FROM "MetaCatalogSyncRun""#).await,
        0
    );
    assert_eq!(
        text(&db, r#"SELECT "catalogId" FROM "IntegrationMetaCatalog""#).await,
        Some("catalog-old".into())
    );
}

// ---- meta-catalog-operations.ts ----------------------------------------------

fn all() -> PushScope {
    PushScope {
        scope: "all".into(),
        ..PushScope::default()
    }
}

#[tokio::test]
async fn select_meta_catalog_a_run_of_the_other_direction_is_a_409_with_the_caller_s_sentence() {
    let db = db_with(Some("9"), "idle").await;
    setup(&db, |tx| run(tx, 1, "running", json!([]))).await;
    let (graph, enc, jobs) = (FakeGraph::default(), encryption(), RecordingEnqueuer::new());
    let deps = Deps {
        db: &db,
        graph: &graph,
        encryption: &enc,
        jobs: Some(&*jobs),
    };
    let error = mc::select(
        &deps,
        WS,
        "9",
        &Reasons {
            already_running: "busy",
        },
    )
    .await
    .expect_err("busy");
    assert_eq!(
        (error.message(), error.status()),
        ("busy".to_string(), Some(409))
    );
}

#[tokio::test]
async fn select_meta_catalog_a_queue_failure_fails_the_run_and_the_import_so_they_cannot_block_later_syncs()
 {
    let db = db_with(Some("9"), "idle").await;
    let (graph, enc, jobs) = (FakeGraph::default(), encryption(), RecordingEnqueuer::new());
    jobs.fail_next("redis down");
    let deps = Deps {
        db: &db,
        graph: &graph,
        encryption: &enc,
        jobs: Some(&*jobs),
    };
    let error = mc::select(&deps, WS, "9", &ENGLISH_REASONS)
        .await
        .expect_err("queue");
    assert!(error.message().contains("redis down"), "{error}");
    let call = &jobs.calls()[0];
    assert_eq!(
        (call.queue, call.name.as_str()),
        (QueueName::Default, "importMetaCatalogProducts")
    );
    assert_eq!(call.data["data"]["workspaceId"], WS.to_string());
    // Adapted: the original passes `reasons.queueImport` as a plain string, so
    // `toPublicErrorMessage` stores the generic failure text.
    assert_eq!(
        text(
            &db,
            r#"SELECT "importStatus" || ':' || "importError" FROM "IntegrationMetaCatalog""#
        )
        .await,
        Some("failed:Meta Catalog import failed. Please try again or contact support.".into())
    );
    assert_eq!(
        text(&db, r#"SELECT "status" FROM "MetaCatalogSyncRun""#).await,
        Some("failed".into())
    );
}

#[tokio::test]
async fn sync_products_to_meta_catalog_verifies_a_new_destination_catalog_with_meta_the_stored_one_costs_no_roundtrip()
 {
    let db = db_with(Some("9"), "idle").await;
    let (graph, enc, jobs) = (FakeGraph::default(), encryption(), RecordingEnqueuer::new());
    let deps = Deps {
        db: &db,
        graph: &graph,
        encryption: &enc,
        jobs: Some(&*jobs),
    };
    mc::sync_products(&deps, WS, Some("9".into()), all(), &ENGLISH_REASONS)
        .await
        .expect("sync");
    assert!(graph.get_catalog.lock().unwrap().is_empty());
    // Adapted: the first run ends so the second may start (one active run).
    setup(&db, |tx| {
        tx.execute(
            r#"UPDATE "MetaCatalogSyncRun" SET "status" = 'succeeded'"#,
            [],
        )?;
        Ok(())
    })
    .await;
    mc::sync_products(&deps, WS, Some("10".into()), all(), &ENGLISH_REASONS)
        .await
        .expect("sync");
    assert_eq!(*graph.get_catalog.lock().unwrap(), vec!["10".to_string()]);
}

#[tokio::test]
async fn sync_products_to_meta_catalog_without_catalogid_pushes_to_the_bound_catalog() {
    let db = db_with(Some("9"), "idle").await;
    let (graph, enc, jobs) = (FakeGraph::default(), encryption(), RecordingEnqueuer::new());
    let deps = Deps {
        db: &db,
        graph: &graph,
        encryption: &enc,
        jobs: Some(&*jobs),
    };
    let run = mc::sync_products(&deps, WS, None, all(), &ENGLISH_REASONS)
        .await
        .expect("sync");
    assert!(graph.get_catalog.lock().unwrap().is_empty());
    assert_eq!(
        (run.workspace_id, run.catalog_id.as_deref()),
        (WS, Some("9"))
    );
    let call = &jobs.calls()[0];
    assert_eq!(call.name, "submitMetaCatalogSync");
    assert_eq!(call.opts.job_id, Some(format!("mc-submit-{}", run.id)));
}

#[tokio::test]
async fn sync_products_to_meta_catalog_without_catalogid_and_no_bound_catalog_asks_for_one_422() {
    let db = db_with(None, "idle").await;
    let (graph, enc, jobs) = (FakeGraph::default(), encryption(), RecordingEnqueuer::new());
    let deps = Deps {
        db: &db,
        graph: &graph,
        encryption: &enc,
        jobs: Some(&*jobs),
    };
    let error = mc::sync_products(&deps, WS, None, all(), &ENGLISH_REASONS)
        .await
        .expect_err("422");
    let BusinessError::Exception { code, field, .. } = &error else {
        panic!("{error}");
    };
    assert_eq!(
        (code.as_str(), field.as_deref()),
        ("validation", Some("catalogId"))
    );
    assert_eq!(
        count(&db, r#"SELECT count(*) FROM "MetaCatalogSyncRun""#).await,
        0
    );
}

#[tokio::test]
async fn sync_products_to_meta_catalog_a_queue_failure_marks_the_run_failed() {
    let db = db_with(Some("9"), "idle").await;
    let (graph, enc, jobs) = (FakeGraph::default(), encryption(), RecordingEnqueuer::new());
    jobs.fail_next("redis down");
    let deps = Deps {
        db: &db,
        graph: &graph,
        encryption: &enc,
        jobs: Some(&*jobs),
    };
    let error = mc::sync_products(&deps, WS, Some("9".into()), all(), &ENGLISH_REASONS)
        .await
        .expect_err("queue");
    assert!(error.message().contains("redis down"), "{error}");
    // Adapted: the stored text is the generic one (see the select test).
    assert_eq!(
        text(
            &db,
            r#"SELECT "status" || ':' || "error" FROM "MetaCatalogSyncRun""#
        )
        .await,
        Some("failed:Meta Catalog sync failed. Please try again or contact support.".into())
    );
}
