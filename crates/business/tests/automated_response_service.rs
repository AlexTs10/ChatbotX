//! Port of `packages/business/__tests__/automated-response.service.test.ts`
//! (P3.6-D3, slice keywords). Adapted: the mocked Drizzle chain becomes a
//! migrated database; the asserted inserts / updates / deletes become the
//! stored rows; `dispatchAuditRecord` becomes the returned audit decision
//! (`Updated::changed`, `StatusChange`, the deleted ids) and the `*_audit`
//! detail texts. The legacy `type: "keyword"` of the TS cases is `inbound`.
//! Not applicable: the caller-owned transaction (template install), the
//! delete guard and the template lock cases (no template locks / guards).

mod common;

use chatbotx_business::automated_response::{
    self as service, CreateAutomatedResponse, ResponseType, UpdateAutomatedResponse,
};
use chatbotx_db::rusqlite::params;
use common::{WS, rows, setup, two_workspaces};
use serde_json::{Value, json};

/// Rule 1 (inbound, keywords "hello"/"hi" as one contains condition, text
/// "Hi"); flow 10 of `WS`, flow 20 of the other workspace; folder 30
/// (`automatedResponse`).
async fn seeded() -> common::TestDb {
    let db = two_workspaces().await;
    setup(&db, |tx| {
        common::flow(tx, 10, WS)?;
        common::flow(tx, 20, common::OTHER_WS)?;
        common::folder(tx, 30, WS, "automatedResponse")?;
        tx.execute(
            r#"INSERT INTO "AutomatedResponse" ("id", "createdAt", "updatedAt", "workspaceId", "keywords", "conditions", "status", "text", "type")
               VALUES (1, 1000, 1000, ?1, '["hello","hi"]', '[{"operator":"contains","values":["hello","hi"]}]', 1, 'Hi', 'inbound')"#,
            params![WS],
        )?;
        Ok(())
    })
    .await;
    db
}

async fn rule(db: &common::TestDb, id: i64) -> serde_json::Map<String, Value> {
    rows(
        db,
        r#"SELECT "keywords", "conditions", "text", "flowId", "folderId", "status", "type" FROM "AutomatedResponse" WHERE "id" = ?1"#,
        id,
    )
    .await
    .into_iter()
    .next()
    .expect("rule row")
}

fn json_col(row: &serde_json::Map<String, Value>, key: &str) -> Value {
    match &row[key] {
        Value::String(text) => serde_json::from_str(text).expect("JSON column"),
        other => other.clone(),
    }
}

async fn count(db: &common::TestDb) -> i64 {
    common::scalar_i64(
        db,
        r#"SELECT count(*) FROM "AutomatedResponse" WHERE "workspaceId" = ?1"#,
        WS,
    )
    .await
    .unwrap_or(0)
}

fn text_update(text: &str) -> UpdateAutomatedResponse {
    UpdateAutomatedResponse {
        text: Some(Some(text.to_string())),
        ..UpdateAutomatedResponse::default()
    }
}

fn inbound(keywords: &[&str], text: &str) -> CreateAutomatedResponse {
    CreateAutomatedResponse {
        kind: ResponseType::Inbound,
        text: Some(text.to_string()),
        keywords: Some(keywords.iter().map(|k| (*k).to_string()).collect()),
        ..CreateAutomatedResponse::default()
    }
}

// describe("automatedResponseService audit side effects")

#[tokio::test]
async fn update_throws_not_found_and_never_audits_when_returning_no_row() {
    let db = seeded().await;
    let error = service::update(&db, WS, 999, ResponseType::Inbound, text_update("Hello"))
        .await
        .expect_err("no row");
    assert_eq!(error.code(), Some("notFound"));
    assert_eq!(error.message(), "Automated response not found");
}

#[tokio::test]
async fn does_not_audit_set_status_when_returning_no_row() {
    let db = seeded().await;
    let change = service::set_status(&db, WS, 999, ResponseType::Inbound, false)
        .await
        .expect("setStatus");
    assert_eq!(change.row, None);
    assert!(!change.changed);
}

#[tokio::test]
async fn does_not_audit_delete_many_when_delete_returning_finds_no_rows() {
    let db = seeded().await;
    let deleted = service::delete_many(&db, WS, vec![999], ResponseType::Inbound)
        .await
        .expect("deleteMany");
    assert!(deleted.is_empty());
    assert_eq!(service::delete_audit(&deleted), None);
    assert_eq!(count(&db).await, 1);
}

#[tokio::test]
async fn audits_normal_non_transaction_update_and_delete() {
    let db = seeded().await;
    let updated = service::update(&db, WS, 1, ResponseType::Inbound, text_update("Hello"))
        .await
        .expect("update");
    assert!(updated.changed);
    assert_eq!(
        service::update_audit(1),
        "updated a keyword automation (#1)"
    );
    let deleted = service::delete_many(&db, WS, vec![1], ResponseType::Inbound)
        .await
        .expect("deleteMany");
    assert_eq!(deleted, vec![1]);
    assert_eq!(
        service::delete_audit(&deleted).as_deref(),
        Some("deleted keyword automation (#1)")
    );
    assert_eq!(
        service::delete_audit(&[1, 2]).as_deref(),
        Some("deleted keyword automations (#1, #2)")
    );
    assert_eq!(
        service::create_audit(7),
        "created a new keyword automation (#7)"
    );
    assert_eq!(
        service::status_audit(7, false),
        "disabled a keyword automation (#7)"
    );
    assert_eq!(
        service::status_audit(7, true),
        "enabled a keyword automation (#7)"
    );
}

// describe("automatedResponseService — type scoping (Contact vs Page keywords)")

/// Adapted: the type predicate of the UPDATE is observed as "an inbound
/// row is not updated through `outbound`".
#[tokio::test]
async fn update_includes_an_explicit_type_predicate_in_the_where_clause() {
    let db = seeded().await;
    let error = service::update(&db, WS, 1, ResponseType::Outbound, text_update("Hello"))
        .await
        .expect_err("other type");
    assert_eq!(error.code(), Some("notFound"));
    assert_eq!(rule(&db, 1).await["text"], json!("Hi"));
    let error = service::update(
        &db,
        common::OTHER_WS,
        1,
        ResponseType::Inbound,
        text_update("Hello"),
    )
    .await
    .expect_err("other workspace");
    assert_eq!(error.code(), Some("notFound"));
}

#[tokio::test]
async fn set_status_scopes_the_read_and_the_write_by_type() {
    let db = seeded().await;
    let change = service::set_status(&db, WS, 1, ResponseType::Outbound, false)
        .await
        .expect("setStatus");
    assert_eq!(change.row, None);
    assert_eq!(rule(&db, 1).await["status"], json!(1));
    let change = service::set_status(&db, WS, 1, ResponseType::Inbound, false)
        .await
        .expect("setStatus");
    assert!(change.changed);
    assert_eq!(change.row.map(|r| r.status), Some(false));
    assert_eq!(rule(&db, 1).await["status"], json!(0));
    let again = service::set_status(&db, WS, 1, ResponseType::Inbound, false)
        .await
        .expect("setStatus");
    assert!(!again.changed);
}

#[tokio::test]
async fn delete_many_scopes_the_delete_by_type_so_it_never_removes_the_other_type_s_row() {
    let db = seeded().await;
    let deleted = service::delete_many(&db, WS, vec![1], ResponseType::Outbound)
        .await
        .expect("deleteMany");
    assert!(deleted.is_empty());
    let deleted = service::delete_many(&db, common::OTHER_WS, vec![1], ResponseType::Inbound)
        .await
        .expect("deleteMany");
    assert!(deleted.is_empty());
    assert_eq!(count(&db).await, 1);
}

#[tokio::test]
async fn find_or_fail_scopes_the_lookup_by_type() {
    let db = seeded().await;
    let error = service::find_or_fail(&db, WS, 1, ResponseType::Outbound)
        .await
        .expect_err("other type");
    assert_eq!(error.code(), Some("notFound"));
    assert_eq!(error.message(), "Automated response not found");
    let found = service::find_or_fail(&db, WS, 1, ResponseType::Inbound)
        .await
        .expect("inbound");
    assert_eq!(found.id, 1);
}

// describe("automatedResponseService.update — keywords and flowId/text invariants")

#[tokio::test]
async fn omitting_keywords_does_not_wipe_the_existing_keywords_column() {
    let db = seeded().await;
    service::update(&db, WS, 1, ResponseType::Inbound, text_update("hi"))
        .await
        .expect("update");
    let row = rule(&db, 1).await;
    assert_eq!(json_col(&row, "keywords"), json!(["hello", "hi"]));
    assert_eq!(row["text"], json!("hi"));
}

#[tokio::test]
async fn explicitly_supplied_keywords_are_still_applied() {
    let db = seeded().await;
    service::update(
        &db,
        WS,
        1,
        ResponseType::Inbound,
        UpdateAutomatedResponse {
            keywords: Some(vec!["new".into()]),
            ..UpdateAutomatedResponse::default()
        },
    )
    .await
    .expect("update");
    assert_eq!(json_col(&rule(&db, 1).await, "keywords"), json!(["new"]));
}

/// Adapted: "flowExists was not called" is observed as a foreign flow id
/// being ignored (no "Flow not found").
#[tokio::test]
async fn nulls_flow_id_when_text_is_set() {
    let db = seeded().await;
    service::update(
        &db,
        WS,
        1,
        ResponseType::Inbound,
        UpdateAutomatedResponse {
            text: Some(Some("hi".into())),
            flow_id: Some(Some(20)),
            ..UpdateAutomatedResponse::default()
        },
    )
    .await
    .expect("update");
    let row = rule(&db, 1).await;
    assert_eq!(row["flowId"], Value::Null);
    assert_eq!(row["text"], json!("hi"));
}

#[tokio::test]
async fn validates_flow_id_against_the_workspace_and_nulls_text_when_flow_id_is_set() {
    let db = seeded().await;
    let updated = service::update(
        &db,
        WS,
        1,
        ResponseType::Inbound,
        UpdateAutomatedResponse {
            flow_id: Some(Some(10)),
            ..UpdateAutomatedResponse::default()
        },
    )
    .await
    .expect("update");
    assert_eq!(updated.row.flow_id, Some(10));
    let row = rule(&db, 1).await;
    assert_eq!(row["flowId"], json!(10));
    assert_eq!(row["text"], Value::Null);
}

#[tokio::test]
async fn rejects_a_flow_id_that_does_not_belong_to_the_workspace() {
    let db = seeded().await;
    let error = service::update(
        &db,
        WS,
        1,
        ResponseType::Inbound,
        UpdateAutomatedResponse {
            flow_id: Some(Some(20)),
            ..UpdateAutomatedResponse::default()
        },
    )
    .await
    .expect_err("foreign flow");
    assert_eq!(error.code(), Some("validation"));
    assert_eq!(error.message(), "Flow not found");
    assert!(matches!(
        &error,
        chatbotx_business::BusinessError::Exception { field: Some(f), .. } if f == "flowId"
    ));
    let row = rule(&db, 1).await;
    assert_eq!(row["text"], json!("Hi"));
    assert_eq!(row["flowId"], Value::Null);
}

// describe("automatedResponseService.create — flowId XOR text")

#[tokio::test]
async fn verifies_flow_id_exists_and_inserts_it_when_only_flow_id_is_given() {
    let db = seeded().await;
    let created = service::create(
        &db,
        WS,
        CreateAutomatedResponse {
            flow_id: Some("10".into()),
            keywords: Some(vec!["hi".into()]),
            ..CreateAutomatedResponse::default()
        },
    )
    .await
    .expect("create");
    let row = rule(&db, created.id).await;
    assert_eq!(row["flowId"], json!(10));
    assert_eq!(row["text"], Value::Null);
    assert_eq!(row["status"], json!(1));
}

#[tokio::test]
async fn throws_instead_of_dropping_text_when_both_flow_id_and_text_are_given() {
    let db = seeded().await;
    let error = service::create(
        &db,
        WS,
        CreateAutomatedResponse {
            text: Some("do not drop me".into()),
            flow_id: Some("10".into()),
            keywords: Some(vec!["hi".into()]),
            ..CreateAutomatedResponse::default()
        },
    )
    .await
    .expect_err("both");
    assert_eq!(
        error.message(),
        "A keyword replies with either text or a flow, not both"
    );
    assert_eq!(count(&db).await, 1);
}

/// Adapted: the real create checks the reply flow with `assertReferencable`
/// before its own `flowService.exists` check, so a missing flow is the
/// guard's 404 `notFound` (parity p1-10 `40-keywords-crud`), not the
/// mocked 422.
#[tokio::test]
async fn throws_a_field_scoped_validation_error_when_flow_id_does_not_exist() {
    let db = seeded().await;
    for flow in ["999", "20"] {
        let error = service::create(
            &db,
            WS,
            CreateAutomatedResponse {
                flow_id: Some(flow.into()),
                keywords: Some(vec!["hi".into()]),
                ..CreateAutomatedResponse::default()
            },
        )
        .await
        .expect_err("missing flow");
        assert_eq!(error.code(), Some("notFound"));
        assert_eq!(error.message(), "Flow not found");
    }
    assert_eq!(count(&db).await, 1);
}

#[tokio::test]
async fn nulls_out_flow_id_when_only_text_is_given() {
    let db = seeded().await;
    let created = service::create(&db, WS, inbound(&["hi"], "Hello there"))
        .await
        .expect("create");
    let row = rule(&db, created.id).await;
    assert_eq!(row["flowId"], Value::Null);
    assert_eq!(row["text"], json!("Hello there"));
}

#[tokio::test]
async fn treats_a_blank_flow_id_from_the_form_as_not_set() {
    let db = seeded().await;
    let created = service::create(
        &db,
        WS,
        CreateAutomatedResponse {
            flow_id: Some(String::new()),
            ..inbound(&["hi"], "this is a link")
        },
    )
    .await
    .expect("create");
    let row = rule(&db, created.id).await;
    assert_eq!(row["flowId"], Value::Null);
    assert_eq!(row["text"], json!("this is a link"));
}

/// Adapted: `folderService.ensureExists` is observed through its 404 for a
/// folder of another type or an unknown id.
#[tokio::test]
async fn ensures_the_folder_exists_before_creating_when_a_folder_id_is_given() {
    let db = seeded().await;
    let created = service::create(
        &db,
        WS,
        CreateAutomatedResponse {
            folder_id: Some(30),
            ..inbound(&["hi"], "Hello there")
        },
    )
    .await
    .expect("create");
    assert_eq!(rule(&db, created.id).await["folderId"], json!(30));
    for (kind, folder) in [(ResponseType::Inbound, 999), (ResponseType::Outbound, 30)] {
        let error = service::create(
            &db,
            WS,
            CreateAutomatedResponse {
                kind,
                folder_id: Some(folder),
                ..inbound(&["hi"], "Hello there")
            },
        )
        .await
        .expect_err("missing folder");
        assert_eq!(error.code(), Some("notFound"));
        assert_eq!(error.message(), "Folder not found");
    }
    assert_eq!(count(&db).await, 2);
}

#[tokio::test]
async fn does_not_check_folder_existence_when_no_folder_id_is_given() {
    let db = seeded().await;
    let created = service::create(&db, WS, inbound(&["hi"], "Hello there"))
        .await
        .expect("create");
    assert_eq!(rule(&db, created.id).await["folderId"], Value::Null);
}

// describe("automatedResponseService keyword conditions")

#[tokio::test]
async fn stores_plain_inbound_keywords_as_one_trimmed_contains_condition() {
    let db = seeded().await;
    let created = service::create(&db, WS, inbound(&[" price ", "giá", " "], "Hi"))
        .await
        .expect("create");
    let row = rule(&db, created.id).await;
    assert_eq!(json_col(&row, "keywords"), json!(["price", "giá"]));
    assert_eq!(
        json_col(&row, "conditions"),
        json!([{ "operator": "contains", "values": ["price", "giá"] }])
    );
}

#[tokio::test]
async fn mirrors_every_condition_value_into_the_flat_keywords_column() {
    let db = seeded().await;
    let conditions = json!([
        { "operator": "contains", "values": ["muốn", "quan tâm"] },
        { "operator": "containsImage", "values": [] },
        { "operator": "notContains", "values": ["test", "muốn"] },
    ]);
    let created = service::create(
        &db,
        WS,
        CreateAutomatedResponse {
            text: Some("Hi".into()),
            conditions: Some(conditions.clone()),
            ..CreateAutomatedResponse::default()
        },
    )
    .await
    .expect("create");
    let row = rule(&db, created.id).await;
    assert_eq!(json_col(&row, "keywords"), json!(["muốn", "quan tâm"]));
    assert_eq!(json_col(&row, "conditions"), conditions);
}

#[tokio::test]
async fn rejects_more_plain_inbound_keywords_than_one_condition_allows() {
    let db = seeded().await;
    let keywords: Vec<String> = (0..51).map(|i| format!("k{i}")).collect();
    let error = service::create(
        &db,
        WS,
        CreateAutomatedResponse {
            text: Some("Hi".into()),
            keywords: Some(keywords),
            ..CreateAutomatedResponse::default()
        },
    )
    .await
    .expect_err("too many");
    assert!(matches!(
        &error,
        chatbotx_business::BusinessError::Exception { field: Some(f), .. } if f == "keywords"
    ));
    assert_eq!(count(&db).await, 1);
}

/// Adapted: the stored conditions are JSON text; an update that sends the
/// same conditions and text reports no change (no audit record).
#[tokio::test]
async fn saving_unchanged_conditions_writes_no_audit_entry() {
    let db = seeded().await;
    let updated = service::update(
        &db,
        WS,
        1,
        ResponseType::Inbound,
        UpdateAutomatedResponse {
            text: Some(Some("Hi".into())),
            conditions: Some(json!([{ "operator": "contains", "values": ["hello", "hi"] }])),
            ..UpdateAutomatedResponse::default()
        },
    )
    .await
    .expect("update");
    assert!(!updated.changed);
}

#[tokio::test]
async fn keeps_outbound_keywords_exact_with_no_conditions() {
    let db = seeded().await;
    let created = service::create(
        &db,
        WS,
        CreateAutomatedResponse {
            kind: ResponseType::Outbound,
            ..inbound(&["bắt đầu tư vấn"], "Hi")
        },
    )
    .await
    .expect("create");
    let row = rule(&db, created.id).await;
    assert_eq!(json_col(&row, "keywords"), json!(["bắt đầu tư vấn"]));
    assert_eq!(row["conditions"], Value::Null);
    assert_eq!(row["type"], json!("outbound"));
}

#[tokio::test]
async fn rejects_conditions_on_an_outbound_rule() {
    let db = seeded().await;
    let error = service::create(
        &db,
        WS,
        CreateAutomatedResponse {
            kind: ResponseType::Outbound,
            text: Some("Hi".into()),
            conditions: Some(json!([{ "operator": "contains", "values": ["hi"] }])),
            ..CreateAutomatedResponse::default()
        },
    )
    .await
    .expect_err("outbound conditions");
    assert_eq!(
        error.message(),
        "Conditions are only supported for contact (inbound) keywords"
    );
    assert!(matches!(
        &error,
        chatbotx_business::BusinessError::Exception { field: Some(f), .. } if f == "conditions"
    ));
    assert_eq!(count(&db).await, 1);
}

#[tokio::test]
async fn rejects_a_request_carrying_both_keywords_and_conditions() {
    let db = seeded().await;
    let error = service::create(
        &db,
        WS,
        CreateAutomatedResponse {
            conditions: Some(json!([{ "operator": "contains", "values": ["hi"] }])),
            ..inbound(&["hi"], "Hi")
        },
    )
    .await
    .expect_err("both");
    assert!(matches!(
        &error,
        chatbotx_business::BusinessError::Exception { field: Some(f), .. } if f == "conditions"
    ));
}

#[tokio::test]
async fn rejects_a_rule_with_neither_keywords_nor_conditions() {
    let db = seeded().await;
    let error = service::create(&db, WS, inbound(&["  "], "Hi"))
        .await
        .expect_err("blank");
    assert_eq!(error.message(), "At least one keyword is required");
    assert!(matches!(
        &error,
        chatbotx_business::BusinessError::Exception { field: Some(f), .. } if f == "keywords"
    ));
}

#[tokio::test]
async fn update_replaces_conditions_and_the_mirrored_keywords_together() {
    let db = seeded().await;
    let updated = service::update(
        &db,
        WS,
        1,
        ResponseType::Inbound,
        UpdateAutomatedResponse {
            conditions: Some(json!([{ "operator": "beginsWith", "values": ["chào"] }])),
            ..UpdateAutomatedResponse::default()
        },
    )
    .await
    .expect("update");
    assert!(updated.changed);
    let row = rule(&db, 1).await;
    assert_eq!(json_col(&row, "keywords"), json!(["chào"]));
    assert_eq!(
        json_col(&row, "conditions"),
        json!([{ "operator": "beginsWith", "values": ["chào"] }])
    );
}

#[tokio::test]
async fn update_without_keywords_or_conditions_leaves_what_the_rule_matches_on() {
    let db = seeded().await;
    service::update(
        &db,
        WS,
        1,
        ResponseType::Inbound,
        text_update("Hello again"),
    )
    .await
    .expect("update");
    let row = rule(&db, 1).await;
    assert_eq!(json_col(&row, "keywords"), json!(["hello", "hi"]));
    assert_eq!(
        json_col(&row, "conditions"),
        json!([{ "operator": "contains", "values": ["hello", "hi"] }])
    );
}

// Own cases (no TS test): `findByInboundKeyword` and `list` (used by the
// contact auto-reply route and `/v1/keywords`).

#[tokio::test]
async fn find_by_inbound_keyword_matches_one_whole_keyword_of_an_inbound_rule() {
    let db = seeded().await;
    let found = service::find_by_inbound_keyword(&db, WS, "hello")
        .await
        .expect("find");
    assert_eq!(found.map(|r| r.id), Some(1));
    for missing in ["hell", "HELLO"] {
        let found = service::find_by_inbound_keyword(&db, WS, missing)
            .await
            .expect("find");
        assert_eq!(found, None, "{missing}");
    }
    let found = service::find_by_inbound_keyword(&db, common::OTHER_WS, "hello")
        .await
        .expect("find");
    assert_eq!(found, None);
}

#[tokio::test]
async fn list_pages_one_type_newest_first() {
    let db = seeded().await;
    let mut ids = vec![1];
    for keyword in ["a", "b"] {
        ids.push(
            service::create(&db, WS, inbound(&[keyword], "x"))
                .await
                .expect("create")
                .id,
        );
    }
    service::create(
        &db,
        WS,
        CreateAutomatedResponse {
            kind: ResponseType::Outbound,
            ..inbound(&["o"], "x")
        },
    )
    .await
    .expect("create outbound");
    let page = service::list(
        &db,
        service::ListAutomatedResponses {
            workspace_id: WS,
            kind: ResponseType::Inbound,
            page: 1,
            per_page: 2,
            ..service::ListAutomatedResponses::default()
        },
    )
    .await
    .expect("list");
    assert_eq!(page.page_count, 2);
    assert_eq!(
        page.data.iter().map(|r| r.id).collect::<Vec<_>>(),
        vec![ids[2], ids[1]]
    );
    let outbound = service::list(
        &db,
        service::ListAutomatedResponses {
            workspace_id: WS,
            kind: ResponseType::Outbound,
            page: 1,
            per_page: 50,
            ..service::ListAutomatedResponses::default()
        },
    )
    .await
    .expect("list");
    assert_eq!(outbound.data.len(), 1);
    assert_eq!(outbound.page_count, 1);
}
