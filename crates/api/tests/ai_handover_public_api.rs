//! Ports of `apps/builder/__tests__/ai-handover-public-api.test.ts`,
//! `ai-handover-bulk-actions.test.ts` and
//! `save-ai-handover-settings-action.test.ts` (Q.D6).
//! Adapted: the captured oRPC handlers and the mocked services become
//! `handlers::dispatch` / `rpc_ai::call` over a real migrated database;
//! "the service was called with" becomes the response and the stored rows;
//! `safeParse(...).success` becomes a 200 / 422 answer. The translated
//! messages are the `en` copies (the TS tests use the identity translator).

mod handlers_common;

use chatbotx_api::routes::scopes_of_module;
use chatbotx_api::rpc_ai::{RpcAiContext, call as rpc};
use chatbotx_db::rusqlite::params;
use handlers_common::{Fixture, WS, call, fixture, scalar};
use serde_json::{Value, json};

const INBOX: i64 = 3001;
const FLOW: i64 = 4001;

async fn seeded() -> Fixture {
    let f = fixture().await;
    f.state
        .db
        .write(|tx| {
            tx.execute(
                r#"INSERT INTO "Inbox" ("id", "workspaceId", "name", "channel", "sourceId") VALUES (?1, ?2, 'Page', 'messenger', 'page-1')"#,
                params![INBOX, WS],
            )?;
            tx.execute(
                r#"INSERT INTO "Flow" ("id", "workspaceId", "name") VALUES (?1, ?2, 'Flow')"#,
                params![FLOW, WS],
            )?;
            Ok(())
        })
        .await
        .expect("seed");
    f
}

const PAGE: &[(&str, &str)] = &[("inboxId", "3001")];

fn settings_body() -> Value {
    json!({
        "enabled": true,
        "scheduleEnabled": false,
        "timeRanges": [],
        "gotoFlowId": null,
        "returnMessage": null,
        "pauseBotWaitingForStaff": false,
    })
}

async fn put(f: &Fixture, body: Value) -> (u16, Value) {
    call(f, "aiHandover.saveSettings", WS, PAGE, None, Some(body)).await
}

async fn apply(f: &Fixture, body: Value) -> (u16, Value) {
    call(f, "aiHandover.setApplyToAll", WS, PAGE, None, Some(body)).await
}

#[tokio::test]
async fn all_routes_use_the_integrations_scope() {
    assert_eq!(
        scopes_of_module("features/integration-ai-handover/api/public"),
        vec!["integrations"]
    );
}

#[tokio::test]
async fn get_settings_validates_the_page_and_returns_defaults_when_never_configured() {
    let f = seeded().await;
    let (status, body) = call(&f, "aiHandover.getSettings", WS, PAGE, None, None).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(
        body,
        json!({
            "enabled": false,
            "scheduleEnabled": false,
            "timeRanges": [],
            "gotoFlowId": null,
            "returnMessage": null,
            "pauseBotWaitingForStaff": false,
            "applyToAll": {
                "status": "idle",
                "applyToAllCustomers": false,
                "isAutomationActive": false,
                "run": null,
            },
        })
    );
    let (status, _) = call(
        &f,
        "aiHandover.getSettings",
        WS,
        &[("inboxId", "9")],
        None,
        None,
    )
    .await;
    assert_eq!(status, 404);
}

#[tokio::test]
async fn save_settings_goes_through_the_service_that_also_stops_a_running_enable() {
    let f = seeded().await;
    let mut body = settings_body();
    body["enabled"] = json!(false);
    body["returnMessage"] = json!("");
    let (status, body) = put(&f, body).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["enabled"], json!(false));
    assert_eq!(body["returnMessage"], Value::Null);
    assert_eq!(
        scalar(
            &f,
            r#"SELECT "workspaceId" FROM "AIHandoverSettings" WHERE "inboxId" = ?1"#,
            INBOX
        )
        .await,
        Some(json!(WS))
    );
}

#[tokio::test]
async fn patch_settings_hands_only_the_sent_fields_to_the_service() {
    let f = seeded().await;
    let mut body = settings_body();
    body["gotoFlowId"] = json!(FLOW.to_string());
    body["pauseBotWaitingForStaff"] = json!(true);
    assert_eq!(put(&f, body).await.0, 200);
    let (status, body) = call(
        &f,
        "aiHandover.patchSettings",
        WS,
        PAGE,
        None,
        Some(json!({ "enabled": false })),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(
        (
            body["enabled"].clone(),
            body["gotoFlowId"].clone(),
            body["pauseBotWaitingForStaff"].clone()
        ),
        (json!(false), json!(FLOW.to_string()), json!(true))
    );
}

async fn switched_on(f: &Fixture) {
    assert_eq!(put(f, settings_body()).await.0, 200);
}

#[tokio::test]
async fn set_apply_to_all_dry_run_only_counts_and_never_changes_anything() {
    let f = seeded().await;
    switched_on(&f).await;
    let (status, body) = apply(
        &f,
        json!({ "applyToAllCustomers": true, "message": "", "dryRun": true }),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(
        body,
        json!({ "dryRun": true, "isChanged": true, "eligibleCount": 0, "run": null })
    );
    assert_eq!(
        scalar(
            &f,
            r#"SELECT "applyToAllRevision" FROM "AIHandoverSettings" WHERE "inboxId" = ?1"#,
            INBOX
        )
        .await,
        Some(json!(0))
    );
}

#[tokio::test]
async fn set_apply_to_all_with_a_confirmation_runs_without_a_user_and_bounded_by_confirm_count() {
    let f = seeded().await;
    switched_on(&f).await;
    let (status, body) = apply(
        &f,
        json!({ "applyToAllCustomers": true, "message": "", "confirmCount": 20 }),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(
        (body["dryRun"].clone(), body["isChanged"].clone()),
        (json!(false), json!(true))
    );
    assert_eq!(body["eligibleCount"], Value::Null);
    let run_id: i64 = body["run"]["id"]
        .as_str()
        .expect("run id")
        .parse()
        .expect("id");
    assert_eq!(
        scalar(
            &f,
            r#"SELECT "requestedByUserId" FROM "AIHandoverBulkRun" WHERE "id" = ?1"#,
            run_id
        )
        .await,
        Some(Value::Null)
    );
    assert_eq!(body["run"]["status"], json!("pending"));
}

#[tokio::test]
async fn a_real_change_without_confirm_count_is_rejected_by_the_schema() {
    let f = seeded().await;
    let (status, body) = apply(&f, json!({ "applyToAllCustomers": true, "message": "" })).await;
    assert_eq!(status, 422, "{body}");
}

#[tokio::test]
async fn dry_run_needs_no_confirm_count_an_off_still_needs_its_message() {
    let f = seeded().await;
    switched_on(&f).await;
    let (status, _) = apply(
        &f,
        json!({ "applyToAllCustomers": true, "message": "", "dryRun": true }),
    )
    .await;
    assert_eq!(status, 200);
    let (status, _) = apply(
        &f,
        json!({ "applyToAllCustomers": false, "message": " ", "confirmCount": 5 }),
    )
    .await;
    assert_eq!(status, 422);
}

#[tokio::test]
async fn retry_runs_without_a_user() {
    let f = seeded().await;
    switched_on(&f).await;
    // The Page is ON at revision 1 and its run failed.
    f.state
        .db
        .write(|tx| {
            tx.execute(
                r#"UPDATE "AIHandoverSettings" SET "applyToAllCustomers" = 1, "applyToAllRevision" = 1, "applyToAllRequestedByUserId" = 1001"#,
                [],
            )?;
            tx.execute(
                r#"INSERT INTO "AIHandoverBulkRun" ("id", "workspaceId", "inboxId", "channel", "revision", "action", "status",
                     "requestedAt", "attempts", "chunkSeq", "processedCount", "skippedCount", "failedCount")
                   VALUES (8001, ?1, ?2, 'messenger', 1, 'enable', 'failed', 0, 0, 0, 0, 0, 0)"#,
                params![WS, INBOX],
            )?;
            Ok(())
        })
        .await
        .expect("seed");
    let (status, body) = call(
        &f,
        "aiHandover.retryApplyToAll",
        WS,
        PAGE,
        None,
        Some(json!({})),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(
        scalar(&f, r#"SELECT "applyToAllRequestedByUserId" FROM "AIHandoverSettings" WHERE "inboxId" = ?1"#, INBOX).await,
        Some(Value::Null)
    );
    assert_eq!(body["run"]["status"], json!("pending"));
}

#[tokio::test]
async fn get_settings_also_reports_apply_to_all_as_idle_for_a_page_never_switched() {
    let f = seeded().await;
    switched_on(&f).await;
    let (_, body) = call(&f, "aiHandover.getSettings", WS, PAGE, None, None).await;
    assert_eq!(body["enabled"], json!(true));
    assert_eq!(
        body["applyToAll"],
        json!({ "status": "idle", "applyToAllCustomers": false, "isAutomationActive": true, "run": null })
    );
}

#[tokio::test]
async fn list_history_is_scoped_to_the_page_and_workspace() {
    let f = seeded().await;
    switched_on(&f).await;
    apply(
        &f,
        json!({ "applyToAllCustomers": true, "confirmCount": 5 }),
    )
    .await;
    let (status, body) = call(
        &f,
        "aiHandover.listHistory",
        WS,
        PAGE,
        Some("page=1&perPage=10"),
        None,
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["pageCount"], json!(1));
    assert_eq!(body["data"][0]["action"], json!("enable"));
    let (status, _) = call(
        &f,
        "aiHandover.listHistory",
        handlers_common::OTHER_WS,
        PAGE,
        None,
        None,
    )
    .await;
    assert_eq!(status, 404);
}

#[tokio::test]
async fn an_on_may_omit_message_including_a_dry_run() {
    let f = seeded().await;
    switched_on(&f).await;
    let (status, _) = apply(&f, json!({ "applyToAllCustomers": true, "dryRun": true })).await;
    assert_eq!(status, 200);
    let (status, body) = apply(
        &f,
        json!({ "applyToAllCustomers": true, "confirmCount": 3 }),
    )
    .await;
    assert_eq!(status, 200, "{body}");
}

#[tokio::test]
async fn an_off_without_message_is_rejected_even_as_a_dry_run() {
    let f = seeded().await;
    let (status, body) = apply(&f, json!({ "applyToAllCustomers": false, "dryRun": true })).await;
    assert_eq!(status, 422);
    assert_eq!(body["data"]["issues"][0]["path"], json!(["message"]));
}

#[tokio::test]
async fn a_non_numeric_goto_flow_id_is_a_validation_error_not_a_500() {
    let f = seeded().await;
    for flow in ["abc", ""] {
        let mut body = settings_body();
        body["gotoFlowId"] = json!(flow);
        assert_eq!(put(&f, body).await.0, 422);
    }
}

#[tokio::test]
async fn the_settings_get_result_can_be_sent_back_to_put() {
    let f = seeded().await;
    assert_eq!(put(&f, settings_body()).await.0, 200);
    let mut body = settings_body();
    body["scheduleEnabled"] = json!(true);
    let (status, body) = put(&f, body).await;
    assert_eq!(status, 422);
    assert_eq!(body["data"]["issues"][0]["path"], json!(["timeRanges"]));
}

// ---- ai-handover-bulk-actions.test.ts / save-ai-handover-settings-action.test.ts ----

fn admin() -> RpcAiContext {
    RpcAiContext::new(WS, 1001).with_member(true, false)
}

async fn action(
    f: &Fixture,
    path: &str,
    context: &RpcAiContext,
    input: Option<Value>,
) -> Result<Value, chatbotx_api::orpc::OrpcError> {
    let mut args = vec![json!(WS.to_string()), json!(INBOX.to_string())];
    args.extend(input);
    rpc(&f.state, path, context, &json!({ "args": args }))
        .await
        .expect("ported")
        .map(|out| out.json)
}

fn off() -> Value {
    json!({ "applyToAllCustomers": false, "message": "A person is here" })
}

#[tokio::test]
async fn moves_the_bound_pages_switch_as_the_signed_in_user() {
    let f = seeded().await;
    switched_on(&f).await;
    let on = json!({ "applyToAllCustomers": true, "message": "" });
    assert_eq!(
        action(&f, "actions.setApplyToAllAction", &admin(), Some(on))
            .await
            .expect("on"),
        json!({ "isChanged": true })
    );
    assert_eq!(
        scalar(&f, r#"SELECT "applyToAllRequestedByUserId" FROM "AIHandoverSettings" WHERE "inboxId" = ?1"#, INBOX).await,
        Some(json!(1001))
    );
    assert_eq!(
        action(&f, "actions.setApplyToAllAction", &admin(), Some(off()))
            .await
            .expect("off"),
        json!({ "isChanged": true })
    );
    assert_eq!(
        scalar(
            &f,
            r#"SELECT "applyToAllMessage" FROM "AIHandoverSettings" WHERE "inboxId" = ?1"#,
            INBOX
        )
        .await,
        Some(json!("A person is here"))
    );
}

#[tokio::test]
async fn reports_a_request_for_the_state_the_page_is_already_in_as_unchanged() {
    let f = seeded().await;
    let on = json!({ "applyToAllCustomers": false, "message": "hi" });
    assert_eq!(
        action(&f, "actions.setApplyToAllAction", &admin(), Some(on))
            .await
            .expect("noop"),
        json!({ "isChanged": false })
    );
}

#[tokio::test]
async fn refuses_a_member_who_is_not_a_super_admin_or_a_support_session_before_anything_is_requested()
 {
    let f = seeded().await;
    switched_on(&f).await;
    let on = json!({ "applyToAllCustomers": true, "message": "" });
    let member = RpcAiContext::new(WS, 1001).with_member(false, false);
    let error = action(&f, "actions.setApplyToAllAction", &member, Some(on.clone()))
        .await
        .expect_err("admin");
    assert_eq!(
        error.message,
        "You need to be a workspace super admin to do this."
    );
    let support = RpcAiContext::new(WS, 1001).with_member(true, true);
    let error = action(&f, "actions.setApplyToAllAction", &support, Some(on))
        .await
        .expect_err("support");
    assert_eq!(
        error.message,
        "Settings cannot be changed from a support session."
    );
    for context in [&member, &support] {
        assert!(
            action(&f, "actions.retryApplyToAllAction", context, None)
                .await
                .is_err()
        );
        assert!(
            action(
                &f,
                "actions.saveAiHandoverSettingsAction",
                context,
                Some(private_settings())
            )
            .await
            .is_err()
        );
    }
    assert_eq!(
        scalar(
            &f,
            r#"SELECT "applyToAllRevision" FROM "AIHandoverSettings" WHERE "inboxId" = ?1"#,
            INBOX
        )
        .await,
        Some(json!(0))
    );
}

#[tokio::test]
async fn shows_the_service_error_translated() {
    let f = seeded().await;
    let on = json!({ "applyToAllCustomers": true, "message": "" });
    let error = action(&f, "actions.setApplyToAllAction", &admin(), Some(on))
        .await
        .expect_err("inactive");
    assert_eq!(
        (error.code.as_str(), error.status),
        ("aiHandoverBulkAutomationNotActive", 422)
    );
    assert_eq!(
        error.message,
        "Save the automation and switch it on (inside its run time) before enabling it for all customers."
    );
    let error = action(&f, "actions.retryApplyToAllAction", &admin(), None)
        .await
        .expect_err("nothing");
    assert_eq!(error.message, "There is nothing to retry.");
}

fn private_settings() -> Value {
    json!({
        "enabled": true,
        "scheduleEnabled": false,
        "timeRanges": [],
        "gotoFlowId": null,
        "returnMessage": "",
        "pauseBotWaitingForStaff": false,
    })
}

#[tokio::test]
async fn saves_for_the_bound_workspace_and_page_and_returns_an_empty_string_for_a_cleared_return_message()
 {
    let f = seeded().await;
    let saved = action(
        &f,
        "actions.saveAiHandoverSettingsAction",
        &admin(),
        Some(private_settings()),
    )
    .await
    .expect("saved");
    assert_eq!(saved, private_settings());
}

#[tokio::test]
async fn maps_a_missing_or_inactive_flow_to_a_translated_error() {
    let f = seeded().await;
    let mut input = private_settings();
    input["gotoFlowId"] = json!("4999");
    let error = action(
        &f,
        "actions.saveAiHandoverSettingsAction",
        &admin(),
        Some(input),
    )
    .await
    .expect_err("flow");
    assert_eq!(
        error.message,
        "The selected flow was not found or is not active."
    );
}

#[tokio::test]
async fn rethrows_another_domain_error_unchanged() {
    let f = seeded().await;
    let mut input = private_settings();
    input["returnMessage"] = json!("x".repeat(2001));
    // Over the form limit: the request schema refuses it (400 validationErrors).
    let error = action(
        &f,
        "actions.saveAiHandoverSettingsAction",
        &admin(),
        Some(input),
    )
    .await
    .expect_err("long");
    assert_eq!(error.status, 400);
}

#[tokio::test]
async fn retries_the_bound_page_as_the_signed_in_user() {
    let f = seeded().await;
    switched_on(&f).await;
    f.state
        .db
        .write(|tx| {
            tx.execute(
                r#"UPDATE "AIHandoverSettings" SET "applyToAllCustomers" = 1, "applyToAllRevision" = 1"#,
                [],
            )?;
            tx.execute(
                r#"INSERT INTO "AIHandoverBulkRun" ("id", "workspaceId", "inboxId", "channel", "revision", "action", "status",
                     "requestedAt", "attempts", "chunkSeq", "processedCount", "skippedCount", "failedCount")
                   VALUES (8001, ?1, ?2, 'messenger', 1, 'enable', 'failed', 0, 0, 0, 0, 0, 0)"#,
                params![WS, INBOX],
            )?;
            Ok(())
        })
        .await
        .expect("seed");
    let out = action(&f, "actions.retryApplyToAllAction", &admin(), None)
        .await
        .expect("retried");
    assert_eq!(out, Value::Null);
    assert_eq!(
        scalar(
            &f,
            r#"SELECT "applyToAllRequestedByUserId" FROM "AIHandoverSettings" WHERE "inboxId" = ?1"#,
            INBOX
        )
        .await,
        Some(json!(1001))
    );
}
