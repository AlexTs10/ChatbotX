//! Port of `packages/business/__tests__/ai-function.service.test.ts` (Q.D6).
//! Adapted: the mocked drizzle client becomes a real migrated database; the
//! audit spy becomes a recording job enqueuer; "the where-clause has the
//! workspace" becomes "a row of another workspace is not changed".
//! Not applicable: "create skips the name-taken check when a transaction is
//! passed" asserts a mock call; the Rust `create_in` (the transaction form)
//! has no name check by construction (covered below).

mod common;
mod x1_support;

use std::sync::Arc;

use chatbotx_business::ai_function::{self as service, AiFunctionInput, DataCollect};
use chatbotx_business::audit::{AuditActor, AuditService, with_audit_context};
use chatbotx_business::tx;
use chatbotx_db::repo::utils::PaginationInput;
use chatbotx_db::rusqlite::params;
use common::{OTHER_WS, WS, scalar_i64, setup, two_workspaces};
use x1_support::RecordingEnqueuer;

fn request() -> AiFunctionInput {
    AiFunctionInput {
        name: "Collect email".into(),
        purpose: Some("Ask for email".into()),
        data_collect: vec![DataCollect {
            from: "email".into(),
            to: "email".into(),
        }],
        output_message: None,
        trigger_flow_id: None,
    }
}

fn audit() -> (AuditService, Arc<RecordingEnqueuer>) {
    let jobs = RecordingEnqueuer::new();
    (AuditService::new(jobs.clone()), jobs)
}

fn actor() -> AuditActor {
    AuditActor {
        user_id: Some("1001".into()),
        workspace_id: Some(WS.to_string()),
        ..AuditActor::default()
    }
}

fn last_audit_detail(jobs: &RecordingEnqueuer) -> Option<String> {
    jobs.calls()
        .last()
        .and_then(|call| call.data["data"]["detail"].as_str().map(str::to_string))
}

/// Function 7 "Collect email" of `WS`.
async fn seeded() -> common::TestDb {
    let db = two_workspaces().await;
    setup(&db, |tx| {
        tx.execute(
            r#"INSERT INTO "AIFunction" ("id", "name", "dataCollect", "workspaceId") VALUES (7, 'Collect email', '[]', ?1)"#,
            params![WS],
        )?;
        Ok(())
    })
    .await;
    db
}

// describe("aiFunctionService audit messages")

#[tokio::test]
async fn create_logs_by_id() {
    let db = two_workspaces().await;
    let (audit, jobs) = audit();
    let created = with_audit_context(actor(), service::create(&db, &audit, WS, request()))
        .await
        .expect("create");
    assert_eq!(
        last_audit_detail(&jobs).as_deref(),
        Some(format!("created a new AI Function (#{})", created.id).as_str())
    );
}

#[tokio::test]
async fn create_throws_when_the_name_is_already_taken() {
    let db = seeded().await;
    let (audit, _) = audit();
    let error = service::create(&db, &audit, WS, request())
        .await
        .expect_err("taken");
    assert_eq!(error.message(), "Name is already taken");
}

#[tokio::test]
async fn create_skips_the_name_taken_check_when_a_transaction_is_passed() {
    let db = seeded().await;
    let created = tx::write(&db, |tx| service::create_in(tx, WS, &request()))
        .await
        .expect("create in tx");
    assert_eq!(created.name, "Collect email");
}

#[tokio::test]
async fn update_ai_function_logs_by_id() {
    let db = seeded().await;
    let (audit, jobs) = audit();
    with_audit_context(actor(), service::update(&db, &audit, WS, 7, request()))
        .await
        .expect("update");
    assert_eq!(
        last_audit_detail(&jobs).as_deref(),
        Some("updated an AI Function (#7)")
    );
}

#[tokio::test]
async fn delete_ai_function_logs_by_id() {
    let db = seeded().await;
    let (audit, jobs) = audit();
    with_audit_context(actor(), service::delete(&db, &audit, WS, 7))
        .await
        .expect("delete");
    assert_eq!(
        last_audit_detail(&jobs).as_deref(),
        Some("deleted an AI Function (#7)")
    );
}

#[tokio::test]
async fn update_ai_function_throws_when_the_function_is_not_found() {
    let db = seeded().await;
    let (audit, jobs) = audit();
    assert!(
        with_audit_context(actor(), service::update(&db, &audit, WS, 99, request()))
            .await
            .is_err()
    );
    assert!(jobs.calls().is_empty());
}

#[tokio::test]
async fn delete_ai_function_throws_when_the_function_is_not_found() {
    let db = seeded().await;
    let (audit, jobs) = audit();
    assert!(
        with_audit_context(actor(), service::delete(&db, &audit, WS, 99))
            .await
            .is_err()
    );
    assert!(jobs.calls().is_empty());
}

#[tokio::test]
async fn list_ai_functions_returns_ai_functions_scoped_to_the_workspace() {
    let db = seeded().await;
    setup(&db, |tx| {
        tx.execute(
            r#"INSERT INTO "AIFunction" ("id", "name", "workspaceId") VALUES (8, 'Other', ?1)"#,
            params![OTHER_WS],
        )?;
        Ok(())
    })
    .await;
    let page = service::list(&db, WS, PaginationInput::default())
        .await
        .expect("list");
    assert_eq!(page.page_count, 1);
    assert_eq!(page.data.iter().map(|f| f.id).collect::<Vec<_>>(), [7]);
}

// describe("aiFunctionService plain-English errors (public API path)")

#[tokio::test]
async fn update_ai_function_throws_the_plain_english_fallback_when_missing() {
    let db = seeded().await;
    let (audit, _) = audit();
    let error = service::update(&db, &audit, WS, 99, request())
        .await
        .expect_err("missing");
    assert_eq!(error.message(), "AI Function not found");
}

#[tokio::test]
async fn delete_ai_function_throws_the_plain_english_fallback_when_missing() {
    let db = seeded().await;
    let (audit, _) = audit();
    let error = service::delete(&db, &audit, WS, 99)
        .await
        .expect_err("missing");
    assert_eq!(error.message(), "AI Function not found");
}

#[tokio::test]
async fn update_ai_function_resolves_the_updated_row() {
    let db = seeded().await;
    let (audit, _) = audit();
    let updated = service::update(
        &db,
        &audit,
        WS,
        7,
        AiFunctionInput {
            name: "Renamed".into(),
            ..request()
        },
    )
    .await
    .expect("update");
    assert_eq!((updated.id, updated.name.as_str()), (7, "Renamed"));
}

// describe("aiFunctionService cross-workspace isolation")

#[tokio::test]
async fn update_scopes_its_where_clause_to_the_requesting_workspace_not_just_the_id() {
    let db = seeded().await;
    let (audit, _) = audit();
    assert!(
        service::update(&db, &audit, OTHER_WS, 7, request())
            .await
            .is_err()
    );
    let name = scalar_i64(
        &db,
        r#"SELECT count(*) FROM "AIFunction" WHERE "id" = ?1 AND "purpose" IS NULL"#,
        7,
    )
    .await;
    assert_eq!(name, Some(1));
}

#[tokio::test]
async fn delete_scopes_its_where_clause_to_the_requesting_workspace_not_just_the_id() {
    let db = seeded().await;
    let (audit, _) = audit();
    assert!(service::delete(&db, &audit, OTHER_WS, 7).await.is_err());
    assert_eq!(
        scalar_i64(
            &db,
            r#"SELECT count(*) FROM "AIFunction" WHERE "id" = ?1"#,
            7
        )
        .await,
        Some(1)
    );
}
