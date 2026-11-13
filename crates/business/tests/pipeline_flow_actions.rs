//! Port of `packages/business/__tests__/pipeline-flow-actions.test.ts`.
//!
//! Adapted (P3.6-D1p): the TS test mocks the item / content services and the
//! repositories. Here every test runs over real SQLite rows:
//! - workspace `WS`; contact 3001 has the open item 701 (pipeline 101,
//!   stage 202); contact 3002 has no item.
//! - pipeline 101 "Sales": stages 201 (first) and 202, fields 301 and 302;
//!   pipeline 102 "Support": stage 211; pipeline 150 is in `OTHER_WS`.
//! - member 401, inbox team 501; the flow id is 801.
//! - "the service was (not) called" assertions become row checks: the item
//!   row, its assignees, comments and `"PipelineItemActivity"` rows (the
//!   fixture writes no activity, so every activity row is a write of the
//!   action). "No lookup was made" has no row counterpart; the outcome is
//!   asserted.
//! - a mocked service rejection is a real one: a missing pipeline
//!   (`notFound`), the open-item limit (`conflict`), a SQLite trigger that
//!   raises "db down" (an unexpected failure).
//! - ids the mocks accepted without rows ("20", stage 201 of pipeline 102)
//!   are real ids (202, 211). Money is the stored x 10,000 integer, dates
//!   are unix ms.
//! - a `test.each` is one test that loops over its cases.

mod common;

use chatbotx_business::pipeline::flow_actions::{
    Interpolate, PIPELINE_FLOW_ACTION_HANDLERS, PipelineActionContext, PipelineFlowActionContext,
    PipelineFlowActionResult, PipelineFlowActionStatus, interpolate_fn, run_pipeline_action,
    run_pipeline_flow_action,
};
use chatbotx_business::pipeline::mutation::PipelineActor;
use chatbotx_core::flow_config::{
    pipeline_action_can_skip, pipeline_step_actions, pipeline_step_default_fn,
};
use chatbotx_db::rusqlite::{Connection, params};
use chatbotx_db::{Db, DbError};
use serde_json::{Map, Value, json};

use common::{
    OTHER_WS, TestDb, WS, contact, custom_field, pipeline_field_row, pipeline_row, rows, setup,
    two_workspaces,
};

use PipelineFlowActionStatus::{Error, Skip, Success};

const CONTACT: i64 = 3001;
const NO_ITEM_CONTACT: i64 = 3002;
const ITEM: i64 = 701;
const FLOW: i64 = 801;
/// 2026-01-02T03:04:05.000Z
const FIXED_AT_MS: i64 = 1_767_323_045_000;
/// 2026-03-04T05:06:07.000Z
const FIELD_AT_MS: i64 = 1_772_600_767_000;

fn stage(tx: &Connection, id: i64, pipeline_id: i64, position: i64) -> Result<(), DbError> {
    tx.execute(
        r#"INSERT INTO "PipelineStage" ("id", "workspaceId", "pipelineId", "name", "position") VALUES (?1, ?2, ?3, ?4, ?5)"#,
        params![id, WS, pipeline_id, format!("Stage {id}"), position],
    )?;
    Ok(())
}

/// An item row (number = 9000 + id, so it never meets the item counter).
fn item_row(
    tx: &Connection,
    id: i64,
    pipeline_id: i64,
    stage_id: i64,
    contact_id: i64,
    status: &str,
    created_at: i64,
) -> Result<(), DbError> {
    tx.execute(
        r#"INSERT INTO "PipelineItem" ("id", "createdAt", "updatedAt", "workspaceId", "pipelineId", "stageId", "contactId", "number",
             "title", "status", "priority", "fieldValues", "stageEnteredAt")
           VALUES (?1, ?2, ?2, ?3, ?4, ?5, ?6, ?7, 'Item', ?8, 'medium', '{}', ?2)"#,
        params![id, created_at, WS, pipeline_id, stage_id, contact_id, 9000 + id, status],
    )?;
    Ok(())
}

async fn board() -> TestDb {
    let db = two_workspaces().await;
    setup(&db, |tx| {
        contact(tx, CONTACT, WS)?;
        contact(tx, NO_ITEM_CONTACT, WS)?;
        pipeline_row(tx, 101, WS, "Sales", None)?;
        stage(tx, 201, 101, 0)?;
        stage(tx, 202, 101, 1)?;
        pipeline_field_row(tx, 301, WS, 101, "Source", false)?;
        pipeline_field_row(tx, 302, WS, 101, "Channel", false)?;
        pipeline_row(tx, 102, WS, "Support", None)?;
        stage(tx, 211, 102, 0)?;
        pipeline_row(tx, 150, OTHER_WS, "Elsewhere", None)?;
        tx.execute(
            r#"INSERT INTO "User" ("id", "email", "name") VALUES (1101, 'agent@example.test', 'Agent')"#,
            [],
        )?;
        tx.execute(
            r#"INSERT INTO "WorkspaceMember" ("id", "workspaceId", "userId", "role") VALUES (401, ?1, 1101, 'agent')"#,
            params![WS],
        )?;
        tx.execute(
            r#"INSERT INTO "InboxTeam" ("id", "workspaceId", "name") VALUES (501, ?1, 'Team')"#,
            params![WS],
        )?;
        item_row(tx, ITEM, 101, 202, CONTACT, "open", 1_000)
    })
    .await;
    db
}

async fn exec(db: &Db, sql: &'static str) {
    setup(db, move |tx| {
        tx.execute_batch(sql)?;
        Ok(())
    })
    .await;
}

fn ada() -> Interpolate {
    interpolate_fn(|text| text.replacen("{{name}}", "Ada", 1))
}

fn ctx() -> PipelineFlowActionContext {
    PipelineFlowActionContext {
        workspace_id: WS,
        contact_id: CONTACT,
        flow_id: FLOW,
        flow_execution_key: Some("exec-1".to_string()),
        interpolate: ada(),
    }
}

fn ctx_for(contact_id: i64) -> PipelineFlowActionContext {
    PipelineFlowActionContext {
        contact_id,
        ..ctx()
    }
}

fn step(action: &str, patch: Value) -> Value {
    let mut out = pipeline_step_default_fn(action, &json!({ "id": "s-1", "pipelineId": "101" }));
    for (key, value) in patch.as_object().cloned().unwrap_or_default() {
        out[key.as_str()] = value;
    }
    out
}

async fn run_with(
    db: &Db,
    step: Value,
    ctx: PipelineFlowActionContext,
) -> PipelineFlowActionResult {
    run_pipeline_flow_action(db, &step, &ctx)
        .await
        .expect("no unexpected failure")
}

async fn run(db: &Db, step: Value) -> PipelineFlowActionResult {
    run_with(db, step, ctx()).await
}

async fn status(db: &Db, step: Value) -> PipelineFlowActionStatus {
    run(db, step).await.status
}

async fn item(db: &Db, id: i64) -> Option<Map<String, Value>> {
    rows(db, r#"SELECT * FROM "PipelineItem" WHERE "id" = ?1"#, id)
        .await
        .into_iter()
        .next()
}

async fn item_field(db: &Db, id: i64, column: &str) -> Value {
    item(db, id).await.expect("item exists")[column].clone()
}

async fn field_values(db: &Db, id: i64) -> Value {
    let raw = item_field(db, id, "fieldValues").await;
    serde_json::from_str(raw.as_str().expect("fieldValues text")).expect("fieldValues JSON")
}

/// The items of the workspace created by the action (not the fixture rows).
async fn new_items(db: &Db) -> Vec<Map<String, Value>> {
    rows(
        db,
        r#"SELECT * FROM "PipelineItem" WHERE "workspaceId" = ?1 AND "id" NOT IN (701, 702, 703, 704, 705, 706) ORDER BY "id""#,
        WS,
    )
    .await
}

async fn only_new_item(db: &Db) -> Map<String, Value> {
    let mut items = new_items(db).await;
    assert_eq!(items.len(), 1, "one item was created");
    items.remove(0)
}

/// `(kind, actorKind, actorUserId, actorSourceId, itemId)` of every activity row.
async fn activity(db: &Db) -> Vec<Map<String, Value>> {
    rows(
        db,
        r#"SELECT "kind", "actorKind", "actorUserId", "actorSourceId", "itemId", "toTitle" FROM "PipelineItemActivity" WHERE "workspaceId" = ?1 ORDER BY "id""#,
        WS,
    )
    .await
}

async fn activity_kinds(db: &Db) -> Vec<String> {
    activity(db)
        .await
        .iter()
        .map(|row| row["kind"].as_str().unwrap_or_default().to_string())
        .collect()
}

/// `(workspaceMemberId, inboxTeamId)` of an item's assignees.
async fn assignees(db: &Db, item_id: i64) -> Vec<(Option<i64>, Option<i64>)> {
    rows(
        db,
        r#"SELECT "workspaceMemberId", "inboxTeamId" FROM "PipelineItemAssignee" WHERE "itemId" = ?1 ORDER BY "id""#,
        item_id,
    )
    .await
    .iter()
    .map(|row| (row["workspaceMemberId"].as_i64(), row["inboxTeamId"].as_i64()))
    .collect()
}

async fn comments(db: &Db) -> Vec<Map<String, Value>> {
    rows(
        db,
        r#"SELECT "itemId", "text", "idempotencyKey" FROM "PipelineItemComment" WHERE "workspaceId" = ?1 ORDER BY "id""#,
        WS,
    )
    .await
}

fn sorted(mut values: Vec<String>) -> Vec<String> {
    values.sort();
    values
}

#[tokio::test]
async fn a_malformed_destination_stage_id_routes_to_error_before_any_lookup() {
    let db = board().await;
    let result = run(
        &db,
        step(
            "transfer",
            json!({ "destinationPipelineId": "102", "destinationStageId": "not-a-number" }),
        ),
    )
    .await;
    assert_eq!(result.status, Error);
    assert_eq!(item_field(&db, ITEM, "pipelineId").await, json!(101));
    assert!(activity(&db).await.is_empty());
}

#[tokio::test]
async fn a_malformed_or_empty_due_date_custom_field_routes_to_error_before_any_query() {
    let db = board().await;
    for custom_field_id in ["abc".to_string(), String::new(), "9".repeat(25)] {
        let result = run(
            &db,
            step(
                "changeDueDate",
                json!({ "dueDate": { "mode": "customField", "customFieldId": custom_field_id } }),
            ),
        )
        .await;
        assert_eq!(result.status, Error);
    }
    assert!(activity(&db).await.is_empty());
}

#[tokio::test]
async fn malformed_reference_ids_route_to_error_without_touching_the_database() {
    let db = board().await;
    assert_eq!(
        status(&db, step("moveToStage", json!({ "stageId": "201" }))).await,
        Success
    );

    let results = [
        run(&db, step("moveToStage", json!({ "pipelineId": "abc" }))).await,
        run(&db, step("moveToStage", json!({ "stageId": "x1" }))).await,
        run(&db, step("assign", json!({ "assigneeMemberIds": ["bad"] }))).await,
        run(
            &db,
            step("transfer", json!({ "destinationPipelineId": "1; drop" })),
        )
        .await,
        run(&db, step("addToPipeline", json!({ "stageId": "nope" }))).await,
    ];

    for result in results {
        assert_eq!(result.status, Error);
    }
    assert_eq!(
        status(&db, step("moveToStage", json!({ "pipelineId": "abc" }))).await,
        Error
    );
    assert_eq!(activity_kinds(&db).await, vec!["stageChanged"]);
    assert!(new_items(&db).await.is_empty());
}

#[test]
fn has_a_handler_for_every_action() {
    assert_eq!(
        sorted(
            PIPELINE_FLOW_ACTION_HANDLERS
                .iter()
                .map(|name| name.to_string())
                .collect()
        ),
        sorted(pipeline_step_actions().enum_values()),
    );
}

#[tokio::test]
async fn errors_without_a_pipeline_or_when_it_is_not_in_the_workspace() {
    let db = board().await;
    assert_eq!(
        status(&db, step("changeTitle", json!({ "pipelineId": "" }))).await,
        Error
    );
    // The pipeline is only looked up when the contact has no target item, so a
    // pipeline of another workspace routes to error.
    let result = run(&db, step("changeTitle", json!({ "pipelineId": "150" }))).await;
    assert_eq!(result.status, Error);
    assert!(
        result
            .message
            .as_deref()
            .unwrap_or_default()
            .contains("does not exist")
    );
}

#[tokio::test]
async fn a_contact_without_an_item_in_an_existing_pipeline_skips() {
    let db = board().await;
    let result = run_with(
        &db,
        step("changeTitle", json!({})),
        ctx_for(NO_ITEM_CONTACT),
    )
    .await;
    assert_eq!(result.status, Skip);
}

#[tokio::test]
async fn a_missing_pipeline_on_add_to_pipeline_errors_from_the_service_without_a_pre_lookup() {
    let db = board().await;
    let result = run(&db, step("addToPipeline", json!({ "pipelineId": "109" }))).await;
    assert_eq!(
        (result.status, result.message.as_deref()),
        (Error, Some("Pipeline not found"))
    );
    assert!(new_items(&db).await.is_empty());
}

#[tokio::test]
async fn add_to_pipeline_creates_an_interpolated_item_with_a_dedup_key() {
    let db = board().await;
    let result = run(
        &db,
        step(
            "addToPipeline",
            json!({
                "title": "Deal {{name}}",
                "description": "  note  ",
                "stageId": "201",
                "status": "open",
                "assigneeMemberIds": ["401"],
                "fieldValues": [{ "pipelineFieldId": "301", "value": "{{name}}" }],
            }),
        ),
    )
    .await;
    assert_eq!(result.status, Success);
    let created = only_new_item(&db).await;
    assert_eq!(created["idempotencyKey"], json!("exec-1:s-1"));
    let id = created["id"].as_i64().expect("id");
    let log = activity(&db).await;
    assert_eq!(log.len(), 1);
    assert_eq!(log[0]["kind"], json!("created"));
    assert_eq!(log[0]["actorKind"], json!("flow"));
    assert_eq!(log[0]["actorUserId"], Value::Null);
    assert_eq!(log[0]["actorSourceId"], json!(FLOW.to_string()));
    assert_eq!(created["title"], json!("Deal Ada"));
    assert_eq!(created["description"], json!("note"));
    assert_eq!(created["stageId"], json!(201));
    assert_eq!(assignees(&db, id).await, vec![(Some(401), None)]);
    assert_eq!(created["dueDate"], Value::Null);
    assert_eq!(field_values(&db, id).await, json!({ "301": "Ada" }));
}

#[tokio::test]
async fn add_to_pipeline_and_add_comment_send_no_idempotency_key_without_flow_execution_key() {
    let db = board().await;
    let no_key = || PipelineFlowActionContext {
        flow_execution_key: None,
        ..ctx()
    };
    run_with(
        &db,
        step("addToPipeline", json!({ "title": "x" })),
        no_key(),
    )
    .await;
    run_with(&db, step("addComment", json!({ "text": "hi" })), no_key()).await;
    assert_eq!(only_new_item(&db).await["idempotencyKey"], Value::Null);
    let comments = comments(&db).await;
    assert_eq!(comments.len(), 1);
    assert_eq!(comments[0]["idempotencyKey"], Value::Null);
}

#[tokio::test]
async fn other_actions_never_send_an_idempotency_key() {
    let db = board().await;
    run(&db, step("changeTitle", json!({ "title": "x" }))).await;
    assert_eq!(item_field(&db, ITEM, "title").await, json!("x"));
    assert_eq!(item_field(&db, ITEM, "idempotencyKey").await, Value::Null);
}

#[tokio::test]
async fn add_to_pipeline_falls_back_to_a_title_and_rejects_a_malformed_stage() {
    let db = board().await;
    run(&db, step("addToPipeline", json!({ "title": "  " }))).await;
    assert_eq!(only_new_item(&db).await["title"], json!("Untitled"));
    let result = run(&db, step("addToPipeline", json!({ "stageId": "bad" }))).await;
    assert_eq!(result.status, Error);
    assert_eq!(new_items(&db).await.len(), 1);
}

#[tokio::test]
async fn a_service_conflict_routes_to_error_with_its_message() {
    let db = board().await;
    exec(
        &db,
        r#"UPDATE "Pipeline" SET "settings" = '{"version":1,"maxOpenItemsPerContact":1}' WHERE "id" = 101"#,
    )
    .await;
    let result = run(&db, step("addToPipeline", json!({ "title": "x" }))).await;
    assert_eq!(
        (result.status, result.message.as_deref()),
        (
            Error,
            Some("This contact already has 1 open item(s) in this pipeline")
        )
    );
}

#[tokio::test]
async fn an_unexpected_failure_propagates_so_the_job_can_retry() {
    let db = board().await;
    exec(
        &db,
        r#"CREATE TRIGGER "db_down" BEFORE INSERT ON "PipelineItem" BEGIN SELECT RAISE(ABORT, 'db down'); END;"#,
    )
    .await;
    let error =
        run_pipeline_flow_action(&db, &step("addToPipeline", json!({ "title": "x" })), &ctx())
            .await
            .expect_err("rejects");
    assert_eq!(error.code(), None);
    assert!(error.message().contains("db down"), "{}", error.message());
}

const SKIPPING_ACTIONS: &[(&str, &str)] = &[
    ("moveToStage", r#"{ "stageId": "201" }"#),
    ("changeValue", r#"{ "value": "10" }"#),
    ("assign", r#"{ "assigneeMemberIds": ["401"] }"#),
    ("changeStatus", r#"{ "status": "won" }"#),
    ("changePriority", r#"{ "priority": "high" }"#),
    ("changeDueDate", r#"{ "dueDate": { "mode": "none" } }"#),
    ("changeTitle", r#"{ "title": "T" }"#),
    (
        "setCustomField",
        r#"{ "pipelineFieldId": "301", "value": "v" }"#,
    ),
    ("clearCustomField", r#"{ "pipelineFieldIds": ["301"] }"#),
    ("changeDescription", r#"{ "description": "d" }"#),
    ("removeFromPipeline", "{}"),
    ("addComment", r#"{ "text": "c" }"#),
    ("transfer", r#"{ "destinationPipelineId": "102" }"#),
];

// The builder hides the skip handle of actions that cannot skip; this keeps
// that rule (flow-config) and the handlers here in step.
#[test]
fn the_actions_that_can_skip_are_exactly_the_ones_the_builder_shows_a_skip_handle_for() {
    assert_eq!(
        sorted(
            SKIPPING_ACTIONS
                .iter()
                .map(|(action, _)| action.to_string())
                .collect()
        ),
        sorted(
            pipeline_step_actions()
                .enum_values()
                .into_iter()
                .filter(|action| pipeline_action_can_skip(action))
                .collect()
        ),
    );
}

#[tokio::test]
async fn add_to_pipeline_never_skips_with_no_open_item_it_creates_one() {
    let db = board().await;
    let result = run_with(
        &db,
        step("addToPipeline", json!({ "title": "x" })),
        ctx_for(NO_ITEM_CONTACT),
    )
    .await;
    assert_eq!(result.status, Success);
    assert_eq!(
        only_new_item(&db).await["contactId"],
        json!(NO_ITEM_CONTACT)
    );
}

#[tokio::test]
async fn skips_when_the_contact_has_no_open_item() {
    let db = board().await;
    // The contact has a closed item only.
    setup(&db, |tx| {
        item_row(tx, 702, 101, 201, NO_ITEM_CONTACT, "won", 2_000)
    })
    .await;
    for (action, patch) in SKIPPING_ACTIONS {
        let patch: Value = serde_json::from_str(patch).expect("patch JSON");
        let result = run_with(&db, step(action, patch), ctx_for(NO_ITEM_CONTACT)).await;
        assert_eq!(result.status, Skip, "{action}");
        assert!(activity(&db).await.is_empty(), "{action}");
        assert!(comments(&db).await.is_empty(), "{action}");
        let closed = item(&db, 702).await.expect("item 702 kept");
        assert_eq!(closed["pipelineId"], json!(101), "{action}");
        assert_eq!(closed["title"], json!("Item"), "{action}");
    }
}

#[tokio::test]
async fn targets_the_latest_open_item_of_the_contact_in_the_pipeline() {
    let db = board().await;
    setup(&db, |tx| {
        item_row(tx, 703, 101, 201, CONTACT, "open", 3_000)?;
        item_row(tx, 704, 101, 201, CONTACT, "won", 4_000)?;
        item_row(tx, 705, 102, 211, CONTACT, "open", 5_000)
    })
    .await;
    run(&db, step("changePriority", json!({ "priority": "high" }))).await;
    assert_eq!(item_field(&db, 703, "priority").await, json!("high"));
    for other in [ITEM, 704, 705] {
        assert_eq!(item_field(&db, other, "priority").await, json!("medium"));
    }
}

#[tokio::test]
async fn move_to_stage_requires_a_well_formed_stage_id() {
    let db = board().await;
    assert_eq!(
        status(&db, step("moveToStage", json!({ "stageId": "201" }))).await,
        Success
    );
    assert_eq!(item_field(&db, ITEM, "stageId").await, json!(201));
    assert_eq!(
        status(&db, step("moveToStage", json!({ "stageId": "x" }))).await,
        Error
    );
    assert_eq!(
        status(&db, step("moveToStage", json!({ "stageId": "" }))).await,
        Error
    );
}

#[tokio::test]
async fn change_value_accepts_a_finite_number_gte_0_and_rejects_the_rest() {
    let db = board().await;
    let change = |value: &str| step("changeValue", json!({ "value": value }));
    assert_eq!(status(&db, change("12.5")).await, Success);
    assert_eq!(item_field(&db, ITEM, "value").await, json!(125_000));
    assert_eq!(status(&db, change("0")).await, Success);
    for bad in ["", "abc", "-1", "Infinity", "1e30"] {
        assert_eq!(status(&db, change(bad)).await, Error, "{bad}");
    }
    assert_eq!(
        activity_kinds(&db).await,
        vec!["valueChanged", "valueChanged"]
    );
    assert_eq!(item_field(&db, ITEM, "value").await, json!(0));
}

#[tokio::test]
async fn change_value_interpolates_variables_first() {
    let db = board().await;
    let result = run_with(
        &db,
        step("changeValue", json!({ "value": "{{name}}" })),
        PipelineFlowActionContext {
            interpolate: interpolate_fn(|_| "42".to_string()),
            ..ctx()
        },
    )
    .await;
    assert_eq!(result.status, Success);
    assert_eq!(item_field(&db, ITEM, "value").await, json!(420_000));
}

#[tokio::test]
async fn set_custom_field_clear_custom_field_write_and_null_the_field_values() {
    let db = board().await;
    exec(
        &db,
        r#"UPDATE "PipelineItem" SET "fieldValues" = '{"302":"old"}' WHERE "id" = 701"#,
    )
    .await;
    run(
        &db,
        step(
            "setCustomField",
            json!({ "pipelineFieldId": "301", "value": "{{name}}" }),
        ),
    )
    .await;
    assert_eq!(
        field_values(&db, ITEM).await,
        json!({ "301": "Ada", "302": "old" })
    );
    run(
        &db,
        step(
            "clearCustomField",
            json!({ "pipelineFieldIds": ["301", "302"] }),
        ),
    )
    .await;
    assert_eq!(field_values(&db, ITEM).await, json!({}));
    assert_eq!(
        status(
            &db,
            step("clearCustomField", json!({ "pipelineFieldIds": [] }))
        )
        .await,
        Error
    );
    assert_eq!(
        status(
            &db,
            step("setCustomField", json!({ "pipelineFieldId": "" }))
        )
        .await,
        Error
    );
}

#[tokio::test]
async fn change_title_rejects_an_empty_interpolated_title_change_description_clears_with_null() {
    let db = board().await;
    exec(
        &db,
        r#"UPDATE "PipelineItem" SET "description" = 'old' WHERE "id" = 701"#,
    )
    .await;
    assert_eq!(
        status(&db, step("changeTitle", json!({ "title": " " }))).await,
        Error
    );
    run(
        &db,
        step("changeDescription", json!({ "description": " " })),
    )
    .await;
    assert_eq!(item_field(&db, ITEM, "description").await, Value::Null);
}

#[tokio::test]
async fn assign_replaces_members_and_teams() {
    let db = board().await;
    run(
        &db,
        step(
            "assign",
            json!({ "assigneeMemberIds": ["401"], "assigneeInboxTeamIds": ["501"] }),
        ),
    )
    .await;
    assert_eq!(
        assignees(&db, ITEM).await,
        vec![(Some(401), None), (None, Some(501))]
    );
}

#[tokio::test]
async fn assign_with_no_assignees_errors_instead_of_clearing_them() {
    let db = board().await;
    exec(
        &db,
        r#"INSERT INTO "PipelineItemAssignee" ("workspaceId", "itemId", "workspaceMemberId") VALUES (2001, 701, 401)"#,
    )
    .await;
    let result = run(
        &db,
        step(
            "assign",
            json!({ "assigneeMemberIds": [], "assigneeInboxTeamIds": [] }),
        ),
    )
    .await;
    assert_eq!(
        (result.status, result.message.as_deref()),
        (Error, Some("The assign step has no assignees"))
    );
    assert_eq!(assignees(&db, ITEM).await, vec![(Some(401), None)]);
}

#[tokio::test]
async fn assign_with_only_teams_is_accepted() {
    let db = board().await;
    let result = run(
        &db,
        step(
            "assign",
            json!({ "assigneeMemberIds": [], "assigneeInboxTeamIds": ["501"] }),
        ),
    )
    .await;
    assert_eq!(result.status, Success);
    assert_eq!(assignees(&db, ITEM).await, vec![(None, Some(501))]);
}

#[tokio::test]
async fn add_to_pipeline_with_empty_assignee_lists_succeeds_and_leaves_them_empty() {
    let db = board().await;
    let result = run(
        &db,
        step(
            "addToPipeline",
            json!({ "title": "x", "assigneeMemberIds": [], "assigneeInboxTeamIds": [] }),
        ),
    )
    .await;
    assert_eq!(result.status, Success);
    let id = only_new_item(&db).await["id"].as_i64().expect("id");
    assert!(assignees(&db, id).await.is_empty());
}

#[tokio::test]
async fn due_date_none_clears_fixed_sets_an_instant() {
    let db = board().await;
    exec(
        &db,
        r#"UPDATE "PipelineItem" SET "dueDate" = 5000 WHERE "id" = 701"#,
    )
    .await;
    run(
        &db,
        step("changeDueDate", json!({ "dueDate": { "mode": "none" } })),
    )
    .await;
    assert_eq!(item_field(&db, ITEM, "dueDate").await, Value::Null);
    run(
        &db,
        step(
            "changeDueDate",
            json!({ "dueDate": { "mode": "fixed", "at": "2026-01-02T03:04:05.000Z" } }),
        ),
    )
    .await;
    assert_eq!(item_field(&db, ITEM, "dueDate").await, json!(FIXED_AT_MS));
}

#[tokio::test]
async fn due_date_custom_field_mode_reads_the_contacts_date_value_blank_clears() {
    let db = board().await;
    setup(&db, |tx| {
        custom_field(tx, 601, WS, "Follow up", "datetime")?;
        tx.execute(
            r#"INSERT INTO "ContactCustomField" ("value", "contactId", "customFieldId") VALUES ('2026-03-04T05:06:07.000Z', ?1, 601)"#,
            params![CONTACT],
        )?;
        Ok(())
    })
    .await;
    let due_date = || {
        step(
            "changeDueDate",
            json!({ "dueDate": { "mode": "customField", "customFieldId": "601" } }),
        )
    };
    run(&db, due_date()).await;
    assert_eq!(item_field(&db, ITEM, "dueDate").await, json!(FIELD_AT_MS));
    exec(
        &db,
        r#"UPDATE "ContactCustomField" SET "value" = '  ' WHERE "customFieldId" = 601"#,
    )
    .await;
    run(&db, due_date()).await;
    assert_eq!(item_field(&db, ITEM, "dueDate").await, Value::Null);
    exec(
        &db,
        r#"UPDATE "PipelineItem" SET "dueDate" = 5000 WHERE "id" = 701;
           DELETE FROM "ContactCustomField" WHERE "customFieldId" = 601"#,
    )
    .await;
    run(&db, due_date()).await;
    assert_eq!(item_field(&db, ITEM, "dueDate").await, Value::Null);
}

#[tokio::test]
async fn due_date_custom_field_mode_rejects_non_date_fields_and_unparsable_values() {
    let db = board().await;
    let due_date = || {
        step(
            "changeDueDate",
            json!({ "dueDate": { "mode": "customField", "customFieldId": "601" } }),
        )
    };
    setup(&db, |tx| custom_field(tx, 601, WS, "Note", "shortText")).await;
    assert_eq!(status(&db, due_date()).await, Error);
    exec(&db, r#"DELETE FROM "CustomField" WHERE "id" = 601"#).await;
    assert_eq!(status(&db, due_date()).await, Error);
    setup(&db, |tx| {
        custom_field(tx, 601, WS, "Follow up", "date")?;
        tx.execute(
            r#"INSERT INTO "ContactCustomField" ("value", "contactId", "customFieldId") VALUES ('not a date', ?1, 601)"#,
            params![CONTACT],
        )?;
        Ok(())
    })
    .await;
    assert_eq!(status(&db, due_date()).await, Error);
    assert!(activity(&db).await.is_empty());
}

#[tokio::test]
async fn remove_from_pipeline_deletes_the_target_item() {
    let db = board().await;
    assert_eq!(
        status(&db, step("removeFromPipeline", json!({}))).await,
        Success
    );
    assert!(item(&db, ITEM).await.is_none());
    let log = activity(&db).await;
    assert_eq!(log.len(), 1);
    assert_eq!(log[0]["kind"], json!("deleted"));
    assert_eq!(log[0]["itemId"], json!(ITEM));
}

#[tokio::test]
async fn add_comment_trims_rejects_empty_and_over_long_text() {
    let db = board().await;
    assert_eq!(
        status(&db, step("addComment", json!({ "text": "  {{name}}  " }))).await,
        Success
    );
    let first = comments(&db).await;
    assert_eq!(first[0]["text"], json!("Ada"));
    assert_eq!(first[0]["idempotencyKey"], json!("exec-1:s-1"));
    assert_eq!(
        status(&db, step("addComment", json!({ "text": " " }))).await,
        Error
    );
    assert_eq!(
        status(&db, step("addComment", json!({ "text": "x".repeat(2001) }))).await,
        Error
    );
    assert_eq!(comments(&db).await.len(), 1);
}

#[tokio::test]
async fn transfer_validates_the_destination_pipeline_and_stage() {
    let db = board().await;
    // A second open item, so the contact still has one in pipeline 101 after the first transfer.
    setup(&db, |tx| {
        item_row(tx, 706, 101, 201, CONTACT, "open", 6_000)
    })
    .await;
    let ok = run(
        &db,
        step(
            "transfer",
            json!({ "destinationPipelineId": "102", "destinationStageId": "211" }),
        ),
    )
    .await;
    assert_eq!(ok.status, Success);
    assert_eq!(item_field(&db, 706, "pipelineId").await, json!(102));
    assert_eq!(item_field(&db, 706, "stageId").await, json!(211));
    let missing = run(
        &db,
        step("transfer", json!({ "destinationPipelineId": "109" })),
    )
    .await;
    assert_eq!(
        (missing.status, missing.message.as_deref()),
        (Error, Some("Pipeline not found"))
    );
    assert_eq!(
        status(
            &db,
            step("transfer", json!({ "destinationPipelineId": "" }))
        )
        .await,
        Error
    );
    assert_eq!(
        status(
            &db,
            step(
                "transfer",
                json!({ "destinationPipelineId": "102", "destinationStageId": "bad" }),
            ),
        )
        .await,
        Error
    );
    assert_eq!(activity_kinds(&db).await, vec!["transferred"]);
    assert_eq!(item_field(&db, ITEM, "pipelineId").await, json!(101));
}

#[tokio::test]
async fn add_to_pipeline_still_creates_the_item_without_the_deleted_stage_admins_teams_and_fields()
{
    let db = board().await;
    exec(
        &db,
        r#"DELETE FROM "PipelineStage" WHERE "id" = 201;
           DELETE FROM "InboxTeam" WHERE "id" = 501;
           DELETE FROM "PipelineField" WHERE "id" = 302"#,
    )
    .await;

    let result = run(
        &db,
        step(
            "addToPipeline",
            json!({
                "title": "Deal",
                "stageId": "201",
                "assigneeMemberIds": ["401", "402"],
                "assigneeInboxTeamIds": ["501"],
                "fieldValues": [
                    { "pipelineFieldId": "301", "value": "kept" },
                    { "pipelineFieldId": "302", "value": "gone" },
                ],
            }),
        ),
    )
    .await;

    assert_eq!(result.status, Success);
    let created = only_new_item(&db).await;
    let id = created["id"].as_i64().expect("id");
    // The first remaining stage.
    assert_eq!(created["stageId"], json!(202));
    assert_eq!(assignees(&db, id).await, vec![(Some(401), None)]);
    assert_eq!(field_values(&db, id).await, json!({ "301": "kept" }));
}

#[tokio::test]
async fn add_to_pipeline_adds_the_item_without_a_due_date_when_the_due_date_field_was_deleted() {
    let db = board().await;

    let result = run(
        &db,
        step(
            "addToPipeline",
            json!({
                "title": "Deal",
                "dueDate": { "mode": "customField", "customFieldId": "601" },
            }),
        ),
    )
    .await;

    assert_eq!(result.status, Success);
    assert_eq!(only_new_item(&db).await["dueDate"], Value::Null);
}

#[tokio::test]
async fn change_due_date_errors_instead_of_clearing_the_date_when_its_field_was_deleted() {
    let db = board().await;
    exec(
        &db,
        r#"UPDATE "PipelineItem" SET "dueDate" = 5000 WHERE "id" = 701"#,
    )
    .await;

    let result = run(
        &db,
        step(
            "changeDueDate",
            json!({ "dueDate": { "mode": "customField", "customFieldId": "601" } }),
        ),
    )
    .await;

    assert_eq!(result.status, Error);
    assert_eq!(item_field(&db, ITEM, "dueDate").await, json!(5000));
    assert!(activity(&db).await.is_empty());
}

#[tokio::test]
async fn assign_keeps_the_remaining_assignees_and_errors_only_when_none_is_left() {
    let db = board().await;

    let kept = run(
        &db,
        step("assign", json!({ "assigneeMemberIds": ["401", "402"] })),
    )
    .await;
    assert_eq!(kept.status, Success);
    assert_eq!(assignees(&db, ITEM).await, vec![(Some(401), None)]);

    let none = run(&db, step("assign", json!({ "assigneeMemberIds": ["402"] }))).await;
    assert_eq!(none.status, Error);
    assert_eq!(assignees(&db, ITEM).await, vec![(Some(401), None)]);
}

#[tokio::test]
async fn clear_custom_field_skips_deleted_fields_and_is_a_no_op_when_all_are_gone() {
    let db = board().await;
    exec(
        &db,
        r#"UPDATE "PipelineItem" SET "fieldValues" = '{"301":"a"}' WHERE "id" = 701;
           DELETE FROM "PipelineField" WHERE "id" = 302"#,
    )
    .await;
    run(
        &db,
        step(
            "clearCustomField",
            json!({ "pipelineFieldIds": ["301", "302"] }),
        ),
    )
    .await;
    assert_eq!(field_values(&db, ITEM).await, json!({}));
    assert_eq!(activity_kinds(&db).await, vec!["fieldValuesChanged"]);

    exec(&db, r#"DELETE FROM "PipelineField" WHERE "id" = 301"#).await;
    let result = run(
        &db,
        step("clearCustomField", json!({ "pipelineFieldIds": ["302"] })),
    )
    .await;
    assert_eq!(result.status, Success);
    assert_eq!(activity_kinds(&db).await, vec!["fieldValuesChanged"]);
}

// runPipelineAction (trigger)

fn trigger_ctx() -> PipelineActionContext {
    PipelineActionContext {
        workspace_id: WS,
        contact_id: CONTACT,
        actor: PipelineActor {
            kind: "trigger".to_string(),
            user_id: None,
            source_id: Some("trigger-1".to_string()),
        },
        base_idempotency_key: Some("trigger:job-1:trigger-1:0".to_string()),
        interpolate: ada(),
    }
}

/// A trigger action is the step's fields without the step envelope.
fn action(name: &str, patch: Value) -> Value {
    let mut fields = step(name, patch);
    let object = fields.as_object_mut().expect("step object");
    object.remove("id");
    object.remove("stepType");
    object.remove("states");
    object.insert("type".to_string(), json!("pipeline"));
    fields
}

async fn run_action(
    db: &Db,
    action: Value,
    ctx: PipelineActionContext,
) -> PipelineFlowActionResult {
    run_pipeline_action(db, &action, &ctx)
        .await
        .expect("no unexpected failure")
}

#[tokio::test]
async fn attributes_the_change_to_the_trigger() {
    let db = board().await;
    run_action(
        &db,
        action("addToPipeline", json!({ "title": "{{name}}" })),
        trigger_ctx(),
    )
    .await;

    let log = activity(&db).await;
    assert_eq!(log.len(), 1);
    assert_eq!(log[0]["actorKind"], json!("trigger"));
    assert_eq!(log[0]["actorUserId"], Value::Null);
    assert_eq!(log[0]["actorSourceId"], json!("trigger-1"));
    assert_eq!(only_new_item(&db).await["title"], json!("Ada"));
}

#[tokio::test]
async fn is_deduplicated_by_the_occurrence_key() {
    let cases = [
        ("addToPipeline", json!({ "title": "x" })),
        ("addComment", json!({ "text": "hi" })),
    ];
    for (name, patch) in cases {
        let db = board().await;
        run_action(&db, action(name, patch), trigger_ctx()).await;

        let key = if name == "addToPipeline" {
            only_new_item(&db).await["idempotencyKey"].clone()
        } else {
            comments(&db).await[0]["idempotencyKey"].clone()
        };
        assert_eq!(key, json!("trigger:job-1:trigger-1:0"), "{name}");
    }
}

#[tokio::test]
async fn identical_add_comment_actions_at_different_indexes_forward_different_keys() {
    let db = board().await;
    let comment = action("addComment", json!({ "text": "same" }));
    for base in [
        "trigger:job-1:trigger-1:0",
        "trigger:job-1:trigger-1:1",
        "trigger:job-1:trigger-1:0",
        "trigger:job-2:trigger-1:0",
    ] {
        run_action(
            &db,
            comment.clone(),
            PipelineActionContext {
                base_idempotency_key: Some(base.to_string()),
                ..trigger_ctx()
            },
        )
        .await;
    }

    // The replay of the first key adds nothing; every other key adds one comment.
    let keys: Vec<Value> = comments(&db)
        .await
        .iter()
        .map(|row| row["idempotencyKey"].clone())
        .collect();
    assert_eq!(
        keys,
        vec![
            json!("trigger:job-1:trigger-1:0"),
            json!("trigger:job-1:trigger-1:1"),
            json!("trigger:job-2:trigger-1:0"),
        ]
    );
}

#[tokio::test]
async fn without_an_occurrence_key_an_add_is_not_deduplicated() {
    let db = board().await;
    run_action(
        &db,
        action("addToPipeline", json!({ "title": "x" })),
        PipelineActionContext {
            base_idempotency_key: None,
            ..trigger_ctx()
        },
    )
    .await;

    assert_eq!(only_new_item(&db).await["idempotencyKey"], Value::Null);
}

#[tokio::test]
async fn actions_that_are_already_idempotent_never_send_the_key() {
    let db = board().await;
    run_action(
        &db,
        action("changeTitle", json!({ "title": "x" })),
        trigger_ctx(),
    )
    .await;

    assert_eq!(item_field(&db, ITEM, "title").await, json!("x"));
    assert_eq!(item_field(&db, ITEM, "idempotencyKey").await, Value::Null);
}

#[tokio::test]
async fn a_contact_with_no_open_item_is_a_skip_a_deleted_pipeline_an_error() {
    let db = board().await;
    let no_item = || PipelineActionContext {
        contact_id: NO_ITEM_CONTACT,
        ..trigger_ctx()
    };
    assert_eq!(
        run_action(
            &db,
            action("changeTitle", json!({ "title": "x" })),
            no_item()
        )
        .await
        .status,
        Skip
    );

    exec(&db, r#"DELETE FROM "Pipeline" WHERE "id" = 101"#).await;
    assert_eq!(
        run_action(
            &db,
            action("changeTitle", json!({ "title": "x" })),
            no_item()
        )
        .await
        .status,
        Error
    );
}

#[tokio::test]
async fn a_missing_or_malformed_pipeline_is_an_error_before_any_lookup() {
    let db = board().await;
    for pipeline_id in ["", "abc"] {
        assert_eq!(
            run_action(
                &db,
                action(
                    "changeTitle",
                    json!({ "title": "x", "pipelineId": pipeline_id })
                ),
                trigger_ctx(),
            )
            .await
            .status,
            Error,
            "{pipeline_id:?}"
        );
    }
    assert_eq!(item_field(&db, ITEM, "title").await, json!("Item"));
    assert!(activity(&db).await.is_empty());
}
