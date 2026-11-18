//! Port of `apps/builder/__tests__/triggers-public-api.test.ts` and the
//! trigger schema tests (`trigger-actions-public-api-schema`,
//! `trigger-pipeline-action-schema`, `trigger-send-google-ads-conversion-schema`,
//! `trigger-send-meta-capi-event-schema`, `trigger-set-custom-field-schema`,
//! `trigger-start-external-step-schema`) (Q.D5). Adapted: the mocked
//! `triggerService` becomes `handlers::dispatch` over real rows; a UI
//! `defaultFn()` is its literal value; the `z.toJSONSchema` and the
//! browser-timezone cases are n-a (no JSON schema export, no browser).

mod handlers_common;

use chatbotx_api::handlers::triggers::{
    send_google_ads_conversion_action, send_meta_capi_event_action, set_custom_field_action,
    start_external_step_action, update_trigger_schema,
};
use chatbotx_api::routes::{RouteAuth, route_by_id};
use chatbotx_db::rusqlite::params;
use handlers_common::{Fixture, OTHER_WS, WS, call, fixture, scalar};
use serde_json::{Value, json};

/// Trigger 51 (`WS`, condition 61 newContact, actions `[addTag 1]`),
/// trigger 59 (`OTHER_WS`); tag 1.
async fn seeded() -> Fixture {
    let f = fixture().await;
    f.state
        .db
        .write(|tx| {
            tx.execute(
                r#"INSERT INTO "Trigger" ("id", "createdAt", "name", "active", "actions", "workspaceId") VALUES
                   (51, 1000, 'T', 1, '[{"type":"addTag","tagIds":["1"]}]', ?1), (59, 2000, 'Far', 1, '[]', ?2)"#,
                params![WS, OTHER_WS],
            )?;
            tx.execute(
                r#"INSERT INTO "Condition" ("id", "triggerId", "type") VALUES (61, 51, 'newContact')"#,
                [],
            )?;
            tx.execute(
                r#"INSERT INTO "Tag" ("id", "name", "workspaceId") VALUES (1, 'a', ?1)"#,
                params![WS],
            )?;
            Ok(())
        })
        .await
        .expect("seed");
    f
}

fn put(actions: Value) -> chatbotx_core::flow_config::z::SafeParse {
    update_trigger_schema().safe_parse(&json!({ "conditions": [], "actions": actions }))
}

fn parsed_actions(actions: Value) -> Value {
    let result = put(actions);
    assert!(result.success, "{:?}", result.messages());
    result.data.expect("data")["actions"].clone()
}

#[test]
fn registers_the_triggers_public_router_under_the_automation_scope() {
    for route in [
        "triggers.list",
        "triggers.get",
        "triggers.create",
        "triggers.update",
        "triggers.updateSettings",
        "triggers.delete",
    ] {
        assert_eq!(
            route_by_id(route).expect("route").auth,
            RouteAuth::WorkspaceToken(&["automation"])
        );
    }
}

// describe("GET /v1/triggers")

#[tokio::test]
async fn returns_real_conditions_and_actions_via_a_single_paginated_query() {
    let f = seeded().await;
    let (status, body) = call(
        &f,
        "triggers.list",
        WS,
        &[],
        Some("page=1&perPage=50"),
        None,
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["data"].as_array().map(Vec::len), Some(1));
    assert_eq!(
        body["data"][0]["conditions"],
        json!([{ "id": "61", "type": "newContact", "sourceId": null, "operator": null, "value": null }])
    );
    assert_eq!(
        body["data"][0]["actions"],
        json!([{ "type": "addTag", "tagIds": ["1"] }])
    );
    assert_eq!(body["pageCount"], 1);
}

// describe("GET /v1/triggers/{id}")

#[tokio::test]
async fn get_delegates_to_find_with_conditions_and_throws_not_found() {
    let f = seeded().await;
    let (status, body) = call(&f, "triggers.get", WS, &[("id", "51")], None, None).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["id"], "51");
    assert_eq!(body["workspaceId"], WS.to_string());
    let (status, body) = call(&f, "triggers.get", WS, &[("id", "59")], None, None).await;
    assert_eq!(status, 404);
    assert_eq!(body["message"], "Trigger not found");
}

// describe("POST /v1/triggers")

#[tokio::test]
async fn create_delegates_to_trigger_service_create() {
    let f = seeded().await;
    let (status, body) = call(
        &f,
        "triggers.create",
        WS,
        &[],
        None,
        Some(json!({ "name": "New trigger", "folderId": null })),
    )
    .await;
    assert_eq!(status, 201, "{body}");
    assert_eq!(body["name"], "New trigger");
    assert_eq!(body["actions"], json!([]));
    assert_eq!(body["conditions"], json!([]));
    let (status, _) = call(
        &f,
        "triggers.create",
        WS,
        &[],
        None,
        Some(json!({ "name": "" })),
    )
    .await;
    assert_eq!(status, 422);
}

// describe("PUT /v1/triggers/{id}")

#[tokio::test]
async fn update_delegates_to_update_with_conditions_and_returns_its_trigger_plus_conditions() {
    let f = seeded().await;
    let (status, body) = call(
        &f,
        "triggers.update",
        WS,
        &[("id", "51")],
        None,
        Some(json!({
            "conditions": [{ "type": "newContact" }],
            "actions": [{ "type": "startAnotherFlow", "flowId": "9", "junk": 1 }],
        })),
    )
    .await;
    // Flow 9 does not exist: a new action naming it is refused.
    assert_eq!(status, 422, "{body}");
    assert_eq!(body["data"]["field"], "actions.0.flowId");
    let (status, body) = call(
        &f,
        "triggers.update",
        WS,
        &[("id", "51")],
        None,
        Some(json!({
            "conditions": [{ "type": "newContact" }],
            "actions": [{ "type": "addTag", "tagIds": ["1"], "junk": 1 }],
        })),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(
        body["actions"],
        json!([{ "type": "addTag", "tagIds": ["1"] }])
    );
    assert_eq!(body["conditions"][0]["type"], "newContact");
    assert_eq!(
        scalar(
            &f,
            r#"SELECT count(*) FROM "Condition" WHERE "triggerId" = ?1"#,
            51
        )
        .await,
        Some(json!(1))
    );
}

#[tokio::test]
async fn throws_not_found_when_the_trigger_update_fails_to_match() {
    let f = seeded().await;
    let (status, body) = call(
        &f,
        "triggers.update",
        WS,
        &[("id", "59")],
        None,
        Some(json!({ "conditions": [], "actions": [] })),
    )
    .await;
    assert_eq!(status, 404);
    assert_eq!(body["message"], "Trigger not found");
}

// describe("PATCH /v1/triggers/{id}/settings")

#[tokio::test]
async fn update_settings_returns_its_result_and_propagates_not_found() {
    let f = seeded().await;
    let (status, body) = call(
        &f,
        "triggers.updateSettings",
        WS,
        &[("id", "51")],
        None,
        Some(json!({ "active": false })),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["active"], false);
    let (status, body) = call(
        &f,
        "triggers.updateSettings",
        WS,
        &[("id", "59")],
        None,
        Some(json!({ "active": false })),
    )
    .await;
    assert_eq!(status, 404);
    assert_eq!(body["message"], "Trigger not found");
}

// describe("DELETE /v1/triggers/{id}")

#[tokio::test]
async fn delete_delegates_to_trigger_service_delete_many() {
    let f = seeded().await;
    let (status, _) = call(&f, "triggers.delete", WS, &[("id", "51")], None, None).await;
    assert_eq!(status, 204);
    assert_eq!(
        scalar(&f, r#"SELECT count(*) FROM "Trigger" WHERE "id" = ?1"#, 51).await,
        Some(json!(0))
    );
    // Another workspace's trigger is not deleted.
    call(&f, "triggers.delete", WS, &[("id", "59")], None, None).await;
    assert_eq!(
        scalar(&f, r#"SELECT count(*) FROM "Trigger" WHERE "id" = ?1"#, 59).await,
        Some(json!(1))
    );
}

// `trigger-actions-public-api-schema.test.ts`

#[test]
fn unknown_extra_properties_on_a_known_action_are_stripped() {
    assert_eq!(
        parsed_actions(json!([{ "type": "addTag", "tagIds": ["1"], "legacyJunk": true }])),
        json!([{ "type": "addTag", "tagIds": ["1"] }])
    );
    assert_eq!(
        parsed_actions(json!([
            { "type": "pipeline", "pipelineId": "101", "action": "removeFromPipeline", "junk": 1 },
            { "type": "startExternalStep", "flowId": "11", "nodeId": "22", "junk": 1 },
        ])),
        json!([
            { "type": "pipeline", "pipelineId": "101", "action": "removeFromPipeline" },
            { "type": "startExternalStep", "flowId": "11", "nodeId": "22" },
        ])
    );
}

#[test]
fn an_unknown_action_type_and_start_external_step_without_a_node_id_are_rejected() {
    assert!(!put(json!([{ "type": "teleport", "flowId": "1" }])).success);
    assert!(!put(json!([{ "type": "startExternalStep", "flowId": "11" }])).success);
}

fn pipeline_variants() -> Vec<Value> {
    vec![
        json!({ "action": "addToPipeline", "title": "{{full_name}}", "description": "", "assigneeMemberIds": ["1"], "assigneeInboxTeamIds": ["2"], "status": "open", "stageId": "201", "dueDate": { "mode": "none" }, "fieldValues": [{ "pipelineFieldId": "301", "value": "{{x}}" }] }),
        json!({ "action": "moveToStage", "stageId": "201" }),
        json!({ "action": "changeValue", "value": "10" }),
        json!({ "action": "assign", "assigneeMemberIds": ["1"], "assigneeInboxTeamIds": [] }),
        json!({ "action": "changeStatus", "status": "won" }),
        json!({ "action": "changePriority", "priority": "high" }),
        json!({ "action": "changeDueDate", "dueDate": { "mode": "customField", "customFieldId": "5" } }),
        json!({ "action": "changeTitle", "title": "t" }),
        json!({ "action": "setCustomField", "pipelineFieldId": "301", "value": "v" }),
        json!({ "action": "clearCustomField", "pipelineFieldIds": ["301"] }),
        json!({ "action": "changeDescription", "description": "d" }),
        json!({ "action": "removeFromPipeline" }),
        json!({ "action": "addComment", "text": "hi" }),
        json!({ "action": "transfer", "destinationPipelineId": "102", "destinationStageId": "202" }),
    ]
}

#[test]
fn every_pipeline_variant_round_trips() {
    let variants = pipeline_variants();
    assert_eq!(variants.len(), 14);
    for fields in variants {
        let mut stored = json!({ "type": "pipeline", "pipelineId": "101" });
        for (k, v) in fields.as_object().expect("object") {
            stored[k] = v.clone();
        }
        assert_eq!(parsed_actions(json!([stored.clone()])), json!([stored]));
    }
}

#[test]
fn change_due_date_accepts_every_due_date_mode_and_rejects_relative() {
    for due in [
        json!({ "mode": "none" }),
        json!({ "mode": "customField", "customFieldId": "5" }),
        json!({ "mode": "customField", "customFieldId": "" }),
        json!({ "mode": "fixed", "at": "2030-01-01T10:00:00+07:00" }),
    ] {
        assert!(
            put(json!([{ "type": "pipeline", "pipelineId": "101", "action": "changeDueDate", "dueDate": due }])).success,
            "{due}"
        );
    }
    let mut add_to = pipeline_variants()[0].clone();
    add_to["type"] = json!("pipeline");
    add_to["pipelineId"] = json!("101");
    for due in [
        json!({ "mode": "none" }),
        json!({ "mode": "customField", "customFieldId": "5" }),
        json!({ "mode": "fixed", "at": "2030-01-01T10:00:00Z" }),
    ] {
        add_to["dueDate"] = due;
        assert!(put(json!([add_to.clone()])).success);
    }
    assert!(
        !put(json!([{ "type": "pipeline", "pipelineId": "101", "action": "changeDueDate", "dueDate": { "mode": "relative", "amount": 2, "unit": "days" } }]))
            .success
    );
}

#[test]
fn set_custom_field_with_operation_and_timezone_parses_and_keeps_both() {
    let stored = json!({ "type": "setCustomField", "customFieldId": "42", "operation": "O12", "value": "3", "timezone": "Asia/Bangkok" });
    assert_eq!(parsed_actions(json!([stored.clone()])), json!([stored]));
}

#[test]
fn old_stored_shapes_parse_unchanged() {
    let mut cases = vec![
        json!({ "type": "addTag", "tagIds": ["1", "2"] }),
        json!({ "type": "removeTag", "tagIds": ["1"] }),
        json!({ "type": "startAnotherFlow", "flowId": "9" }),
        json!({ "type": "clearCustomField", "customFieldId": "42" }),
    ];
    for op in ["O01", "O02", "O03", "O04", "O05"] {
        cases.push(json!({ "type": "setCustomField", "customFieldId": "42", "operation": op, "value": "" }));
    }
    for stored in cases {
        assert_eq!(parsed_actions(json!([stored.clone()])), json!([stored]));
    }
}

fn stored_field(patch: Value) -> Value {
    let mut out = json!({ "type": "setCustomField", "customFieldId": "42" });
    for (k, v) in patch.as_object().expect("object") {
        out[k] = v.clone();
    }
    out
}

#[test]
fn legacy_set_custom_field_operand_rules() {
    for patch in [
        json!({}),
        json!({ "value": "" }),
        json!({ "operation": "" }),
    ] {
        let result = set_custom_field_action().safe_parse(&stored_field(patch));
        assert!(!result.success);
        assert!(
            result.paths().iter().any(|p| p.starts_with("operation")),
            "{:?}",
            result.paths()
        );
    }
    for op in ["O01", "O02", "O03", "O04", "O05"] {
        assert!(
            set_custom_field_action()
                .safe_parse(&stored_field(json!({ "operation": op, "value": "" })))
                .success
        );
    }
    let result = set_custom_field_action().safe_parse(&stored_field(json!({ "operation": "O02" })));
    assert_eq!(result.data.expect("data")["value"], "");
    for op in ["O07", "O09", "O10", "O11", "O12"] {
        for value in ["", "   "] {
            let result = set_custom_field_action()
                .safe_parse(&stored_field(json!({ "operation": op, "value": value })));
            assert!(!result.success);
            assert_eq!(result.issues()[0].path_json(), json!(["value"]));
        }
    }
    // `trigger-set-custom-field-schema.test.ts`
    for op in ["O06", "O08"] {
        assert!(
            set_custom_field_action()
                .safe_parse(&stored_field(json!({ "operation": op })))
                .success
        );
    }
    let stored = stored_field(json!({ "operation": "O04", "value": "5" }));
    assert_eq!(
        set_custom_field_action().safe_parse(&stored).data,
        Some(stored)
    );
    let zoned =
        stored_field(json!({ "operation": "O01", "value": "x", "timezone": "Asia/Bangkok" }));
    assert_eq!(
        set_custom_field_action()
            .safe_parse(&zoned)
            .data
            .expect("data")["timezone"],
        "Asia/Bangkok"
    );
}

#[test]
fn add_sequence_and_remove_sequence_actions() {
    for kind in ["addSequence", "removeSequence"] {
        assert_eq!(
            parsed_actions(
                json!([{ "type": kind, "sequenceIds": ["11", "12"], "legacyJunk": true }])
            ),
            json!([{ "type": kind, "sequenceIds": ["11", "12"] }])
        );
        let many: Vec<String> = (1..=51).map(|n| n.to_string()).collect();
        for body in [
            json!({ "sequenceIds": [] }),
            json!({}),
            json!({ "sequenceIds": ["abc"] }),
            json!({ "sequenceIds": ["1234567890123456789"] }),
            json!({ "sequenceIds": [11] }),
            json!({ "sequenceIds": many }),
        ] {
            let mut action = body.clone();
            action["type"] = json!(kind);
            assert!(!put(json!([action])).success, "{kind} {body}");
        }
    }
}

// `trigger-pipeline-action-schema.test.ts`

#[test]
fn trigger_pipeline_action_schema() {
    assert!(
        !put(json!([{ "type": "pipeline", "action": "addToPipeline", "pipelineId": "" }])).success
    );
    assert!(put(json!([{ "type": "pipeline", "pipelineId": "101", "action": "moveToStage", "stageId": "201" }])).success);
    assert!(
        put(json!([
            { "type": "pipeline", "pipelineId": "101", "action": "removeFromPipeline" },
            { "type": "startExternalStep", "flowId": "11", "nodeId": "22" },
            { "type": "addTag", "tagIds": ["1"] },
        ]))
        .success
    );
    assert!(
        !put(json!([{ "type": "pipeline", "pipelineId": "101", "action": "moveToStage" }])).success
    );
}

// `trigger-send-google-ads-conversion-schema.test.ts`

fn google_default() -> Value {
    json!({ "type": "sendGoogleAdsConversion", "conversionActionId": "", "dedupMode": "id" })
}

fn with(base: Value, patch: Value) -> Value {
    let mut out = base;
    for (k, v) in patch.as_object().expect("object") {
        out[k] = v.clone();
    }
    out
}

#[test]
fn trigger_send_google_ads_conversion_action_schema() {
    let schema = send_google_ads_conversion_action();
    assert!(!schema.safe_parse(&google_default()).success);
    assert!(
        !schema
            .safe_parse(&with(
                google_default(),
                json!({ "conversionActionId": "abc" })
            ))
            .success
    );
    let base = with(google_default(), json!({ "conversionActionId": "123" }));
    assert!(
        schema
            .safe_parse(&with(base.clone(), json!({ "dedupMode": "click" })))
            .success
    );
    let result = schema.safe_parse(&base);
    assert!(!result.success);
    assert_eq!(result.issues()[0].path_json(), json!(["dedupId"]));
    assert_eq!(
        result.messages()[0],
        "googleAds.conversionFields.validation.dedupIdRequired"
    );
    let long = "x".repeat(65);
    let result = schema.safe_parse(&with(base.clone(), json!({ "dedupId": long })));
    assert!(!result.success);
    assert_eq!(
        result.messages()[0],
        "googleAds.conversionFields.validation.dedupIdTooLong"
    );
    assert!(
        schema
            .safe_parse(&with(base.clone(), json!({ "dedupId": "x".repeat(64) })))
            .success
    );
    assert!(
        schema
            .safe_parse(&with(
                base.clone(),
                json!({ "dedupMode": "click", "dedupId": long })
            ))
            .success
    );
    let mut without_mode = base.clone();
    without_mode
        .as_object_mut()
        .expect("object")
        .remove("dedupMode");
    assert!(!schema.safe_parse(&without_mode).success);
    let click = with(base, json!({ "dedupMode": "click" }));
    assert!(
        !schema
            .safe_parse(&with(click.clone(), json!({ "value": "10" })))
            .success
    );
    assert!(
        !schema
            .safe_parse(&with(click.clone(), json!({ "currency": "USD" })))
            .success
    );
    assert!(
        schema
            .safe_parse(&with(
                click,
                json!({ "value": "10", "currency": "USD", "dedupMode": "id", "dedupId": "ORDER-1" })
            ))
            .success
    );
    // Registered in the trigger action union.
    assert!(put(json!([{ "type": "sendGoogleAdsConversion", "conversionActionId": "1", "dedupMode": "click" }])).success);
}

// `trigger-send-meta-capi-event-schema.test.ts`

fn meta_default() -> Value {
    json!({ "type": "sendMetaCapiEvent", "eventName": "LeadSubmitted", "actionSource": "business_messaging" })
}

#[test]
fn trigger_send_meta_capi_event_action_schema() {
    let schema = send_meta_capi_event_action();
    assert!(
        !schema
            .safe_parse(&with(meta_default(), json!({ "eventName": "Purchase" })))
            .success
    );
    assert!(
        schema
            .safe_parse(&with(
                meta_default(),
                json!({ "eventName": "Purchase", "value": "10", "currency": "USD" })
            ))
            .success
    );
    let result = schema.safe_parse(&json!({
        "type": "sendMetaCapiEvent", "eventName": "LeadSubmitted", "value": "10", "currency": "USD",
        "contentCategory": "Education", "contentName": "Course",
    }));
    assert!(result.success, "{:?}", result.messages());
    let data = result.data.expect("data");
    assert_eq!(data["actionSource"], "business_messaging");
    assert_eq!(data["contentName"], "Course");
    assert!(
        schema
            .safe_parse(&with(
                meta_default(),
                json!({ "actionSource": "email", "eventName": "my-custom-event" })
            ))
            .success
    );
    assert!(
        !schema
            .safe_parse(&with(meta_default(), json!({ "eventName": "Lead" })))
            .success
    );
    assert!(schema.safe_parse(&meta_default()).success);
}

// `trigger-start-external-step-schema.test.ts`

#[test]
fn trigger_start_external_step_action_schema() {
    let schema = start_external_step_action();
    assert!(
        schema
            .safe_parse(&json!({ "type": "startExternalStep", "flowId": "11", "nodeId": "22" }))
            .success
    );
    for action in [
        json!({ "type": "startExternalStep", "flowId": "", "nodeId": "" }),
        json!({ "type": "startExternalStep", "flowId": "11" }),
        json!({ "type": "startExternalStep", "flowId": "a", "nodeId": "b" }),
        json!({ "type": "startAnotherFlow", "flowId": "1", "nodeId": "2" }),
    ] {
        assert!(!schema.safe_parse(&action).success, "{action}");
    }
}

// `update-trigger-action.test.ts`: catalog and call conditions as validated.

#[test]
fn catalog_and_call_conditions_are_normalized_by_the_schema() {
    let result = update_trigger_schema().safe_parse(&json!({
        "actions": [],
        "conditions": [
            { "type": "shoppingCartSent" },
            { "type": "shoppingCartSent", "sourceId": "" },
            { "type": "userAskedAboutProduct", "sourceId": "123456789012345678" },
            { "type": "incomingCall" },
        ],
    }));
    assert!(result.success, "{:?}", result.messages());
    assert_eq!(
        result.data.expect("data")["conditions"],
        json!([
            { "type": "shoppingCartSent", "sourceId": null },
            { "type": "shoppingCartSent", "sourceId": null },
            { "type": "userAskedAboutProduct", "sourceId": "123456789012345678" },
            { "type": "incomingCall" },
        ])
    );
}
