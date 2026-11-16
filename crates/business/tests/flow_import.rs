//! Ports of the flow import tests (P3.6-D2 slice flow import):
//! `packages/business/__tests__/import.service.test.ts` (`startFlowImport`, 5),
//! `flow-import-flow-export.test.ts` (`importFlowExport`, 10) and
//! `apps/worker/__tests__/flow-import-handler.test.ts` (`runFlowImport`).
//! Adapted: the mocked db, services and uploader become real rows and an
//! in-memory store; "the update calls" = the stored `Import` row after the
//! run; a mocked `importFlowExport` result becomes the real resolution of
//! seeded fields. Not applicable: the two custom-field / bot-field cache
//! invalidation cases (no field caches in Rust) and the stored `errorCode`
//! (the `Import` row has no code column; the message is kept).

mod common;

use chatbotx_business::flow::import::{
    BindingInput, FieldManifestEntry, ImportFlowExport, ResolvedField, StartFlowImportRefusal,
    compute_custom_field_name_bindings, flow_import_reference_kinds, import_flow_export,
    rewrite_bot_field_tokens_deep, rewrite_field_tokens_deep, run_flow_import, start_flow_import,
    summarize_schema_error,
};
use chatbotx_core::flow_config::authoring::FlowAuthoringError;
use chatbotx_core::flow_config::import_export::reference_kinds_with_policy;
use chatbotx_core::utils::custom_field::CustomFieldType;
use chatbotx_db::rusqlite::params;
use chatbotx_files::{PutOptions, Uploader};
use common::{TestDb, WS, bot_field, custom_field, folder, rows, setup, two_workspaces};
use serde_json::{Value, json};

const PATH: &str = "workspaces/2001/imports/flows/flow.json";

async fn file(db: &TestDb, id: i64, context: &'static str, sub: &'static str) {
    setup(db, move |tx| {
        tx.execute(
            r#"INSERT INTO "File" ("id", "workspaceId", "contextType", "subType", "path", "fileName", "mimeType", "status") VALUES (?1, ?2, ?3, ?4, ?5, 'flow.json', 'application/json', 'pending')"#,
            params![id, WS, context, sub, PATH],
        )?;
        Ok(())
    })
    .await;
}

// --- importService.startFlowImport ------------------------------------

#[tokio::test]
async fn start_flow_import_returns_file_not_found_when_the_file_does_not_exist_in_the_workspace() {
    let db = two_workspaces().await;
    let out = start_flow_import(&db, WS, Some(1001), 1, None)
        .await
        .unwrap();
    assert_eq!(out, Err(StartFlowImportRefusal::FileNotFound));
    let imports = rows(&db, r#"SELECT "id" FROM "Import" WHERE ?1 = ?1"#, 0).await;
    assert!(imports.is_empty());
}

#[tokio::test]
async fn start_flow_import_returns_not_a_flow_import_when_the_file_is_not_a_flow_import_upload() {
    let db = two_workspaces().await;
    file(&db, 1, "generic", "flow").await;
    let out = start_flow_import(&db, WS, Some(1001), 1, None)
        .await
        .unwrap();
    assert_eq!(out, Err(StartFlowImportRefusal::NotAFlowImport));
    let imports = rows(&db, r#"SELECT "id" FROM "Import" WHERE ?1 = ?1"#, 0).await;
    assert!(imports.is_empty());
}

#[tokio::test]
async fn start_flow_import_returns_not_a_flow_import_when_the_file_s_sub_type_is_not_flow() {
    let db = two_workspaces().await;
    file(&db, 1, "import", "contacts").await;
    let out = start_flow_import(&db, WS, Some(1001), 1, None)
        .await
        .unwrap();
    assert_eq!(out, Err(StartFlowImportRefusal::NotAFlowImport));
}

#[tokio::test]
async fn start_flow_import_marks_the_file_uploaded_and_creates_the_import_row_atomically() {
    let db = two_workspaces().await;
    file(&db, 1, "import", "flow").await;
    let import_id = start_flow_import(&db, WS, Some(1001), 1, None)
        .await
        .unwrap()
        .expect("started");
    let import = rows(
        &db,
        r#"SELECT "workspaceId", "userId", "fileId", "type", "format", "status", "meta" FROM "Import" WHERE "id" = ?1"#,
        import_id,
    )
    .await;
    assert_eq!(
        Value::Object(import[0].clone()),
        json!({
            "workspaceId": WS, "userId": 1001, "fileId": 1, "type": "flow",
            "format": "json", "status": "pending", "meta": r#"{"folderId":null}"#,
        })
    );
    let file = rows(
        &db,
        r#"SELECT "status", "uploadedAt" FROM "File" WHERE "id" = ?1"#,
        1,
    )
    .await;
    assert_eq!(file[0]["status"], json!("uploaded"));
    assert!(file[0]["uploadedAt"].is_i64());
}

#[tokio::test]
async fn start_flow_import_persists_the_requested_folder_id_into_the_import_row_s_meta() {
    let db = two_workspaces().await;
    file(&db, 1, "import", "flow").await;
    let import_id = start_flow_import(&db, WS, Some(1001), 1, Some(77))
        .await
        .unwrap()
        .expect("started");
    let import = rows(
        &db,
        r#"SELECT "meta" FROM "Import" WHERE "id" = ?1"#,
        import_id,
    )
    .await;
    let meta: Value = serde_json::from_str(import[0]["meta"].as_str().unwrap()).unwrap();
    assert_eq!(meta, json!({ "folderId": "77" }));
}

// --- flowService.importFlowExport -------------------------------------

/// A `sendMessage` start node (id `1`) with these steps.
fn node(steps: Value) -> Value {
    json!({
        "id": "1",
        "position": { "x": 0, "y": 0 },
        "measured": { "width": 288, "height": 100 },
        "type": "sendMessage",
        "data": {
            "name": "Send Message",
            "isStartNode": true,
            "details": {
                "beforeStep": { "id": "2", "stepType": "chooseChannel", "channel": "omnichannel" },
                "steps": steps,
                "quickReplies": [],
            },
        },
    })
}

fn set_field_step(field_id: &str, value: &str) -> Value {
    json!({ "id": "3", "stepType": "setCustomField", "inputFieldId": field_id, "operation": "O01", "value": value })
}

fn entry(name: &str, kind: CustomFieldType) -> FieldManifestEntry {
    FieldManifestEntry {
        name: name.into(),
        kind,
    }
}

fn input(nodes: Vec<Value>) -> ImportFlowExport {
    ImportFlowExport {
        workspace_id: WS,
        name: "Onboarding".into(),
        active: true,
        enable_in_inbox: true,
        start_node_id: "1".into(),
        nodes,
        edges: Vec::new(),
        custom_fields: Vec::new(),
        bot_fields: Vec::new(),
        folder_id: None,
    }
}

/// The stored draft nodes and folder of a flow.
async fn stored(db: &TestDb, flow_id: i64) -> (Value, Value) {
    let flow = rows(db, r#"SELECT f."folderId", v."nodes" FROM "Flow" f JOIN "FlowVersion" v ON v."id" = f."draftVersionId" WHERE f."id" = ?1"#, flow_id).await;
    let nodes: Value = serde_json::from_str(flow[0]["nodes"].as_str().unwrap()).unwrap();
    (nodes, flow[0]["folderId"].clone())
}

#[test]
fn import_flow_export_never_trusts_the_file_s_own_source_workspace_to_keep_member_and_team_ids() {
    // Adapted: `referenceKindsWithPolicy` is mocked to `[]` in the original,
    // so the real verifyInTarget kinds follow the eight listed ones.
    let kinds = flow_import_reference_kinds();
    assert_eq!(
        kinds[..8],
        [
            "customField",
            "botField",
            "tag",
            "pipeline",
            "folder",
            "productCategory",
            "workspaceMember",
            "inboxTeam",
        ]
    );
    assert_eq!(
        kinds[8..],
        reference_kinds_with_policy("verifyInTarget")[..]
    );
}

#[tokio::test]
async fn import_flow_export_resolves_the_manifest_remaps_references_and_inserts_the_flow_in_one_transaction()
 {
    let db = two_workspaces().await;
    let mut request = input(vec![node(json!([set_field_step("42", "1990-01-01")]))]);
    request.custom_fields = vec![("42".into(), entry("Birthday", CustomFieldType::Date))];
    let out = import_flow_export(&db, request).await.expect("imported");
    assert_eq!(out.created_custom_field_ids.len(), 1);
    assert!(out.created_bot_field_ids.is_empty());
    assert!(out.renamed_custom_fields.is_empty());
    assert!(out.renamed_bot_fields.is_empty());
    let target = &out.created_custom_field_ids[0];
    let field = rows(
        &db,
        r#"SELECT "name", "type", "showInInbox", "workspaceId" FROM "CustomField" WHERE "id" = ?1"#,
        target.parse().unwrap(),
    )
    .await;
    assert_eq!(
        Value::Object(field[0].clone()),
        json!({ "name": "Birthday", "type": "date", "showInInbox": 1, "workspaceId": WS })
    );
    // The remapped graph reaches the draft version row.
    let (nodes, _) = stored(&db, out.flow_id).await;
    assert_eq!(
        nodes[0]["data"]["details"]["steps"][0]["inputFieldId"],
        json!(target)
    );
    let flow = rows(&db, r#"SELECT "name", "active", "enableInInbox", "currentVersionId" FROM "Flow" WHERE "id" = ?1"#, out.flow_id).await;
    assert_eq!(
        Value::Object(flow[0].clone()),
        json!({ "name": "Onboarding", "active": 1, "enableInInbox": 1, "currentVersionId": null })
    );
    let sessions = rows(
        &db,
        r#"SELECT "id" FROM "FlowAnalyticsSession" WHERE "flowId" = ?1"#,
        out.flow_id,
    )
    .await;
    assert_eq!(sessions.len(), 1);
}

#[tokio::test]
async fn import_flow_export_rewrites_name_tokens_before_remapping_and_reports_a_renamed_custom_field()
 {
    let db = two_workspaces().await;
    // `size` is taken by a text field, so the number field is created as `size_1`.
    setup(&db, |tx| {
        custom_field(tx, 900, WS, "size", "shortText")?;
        Ok(())
    })
    .await;
    let mut request = input(vec![node(json!([set_field_step(
        "9",
        "{{size}} and {{raw:size}}"
    )]))]);
    request.custom_fields = vec![("9".into(), entry("size", CustomFieldType::Number))];
    let out = import_flow_export(&db, request).await.expect("imported");
    assert_eq!(out.renamed_custom_fields.len(), 1);
    assert_eq!(
        (
            out.renamed_custom_fields[0].from.as_str(),
            out.renamed_custom_fields[0].to.as_str(),
            out.renamed_custom_fields[0].kind.as_str()
        ),
        ("size", "size_1", "number")
    );
    let (nodes, _) = stored(&db, out.flow_id).await;
    assert_eq!(
        nodes[0]["data"]["details"]["steps"][0]["value"],
        json!("{{size_1}} and {{raw:size_1}}")
    );
}

#[tokio::test]
async fn import_flow_export_resolves_a_bot_field_manifest_entry_into_id_maps_bot_field() {
    let db = two_workspaces().await;
    let mut request = input(vec![node(json!([set_field_step(
        "bot_field:7",
        "{{bot_field:7}}"
    )]))]);
    request.bot_fields = vec![("7".into(), entry("Loyalty Points", CustomFieldType::Number))];
    let out = import_flow_export(&db, request).await.expect("imported");
    assert_eq!(out.created_bot_field_ids.len(), 1);
    let target = &out.created_bot_field_ids[0];
    let field = rows(
        &db,
        r#"SELECT "name", "type" FROM "BotField" WHERE "id" = ?1"#,
        target.parse().unwrap(),
    )
    .await;
    assert_eq!(
        Value::Object(field[0].clone()),
        json!({ "name": "Loyalty Points", "type": "number" })
    );
    let (nodes, _) = stored(&db, out.flow_id).await;
    let step = &nodes[0]["data"]["details"]["steps"][0];
    assert_eq!(step["inputFieldId"], json!(format!("bot_field:{target}")));
    assert_eq!(step["value"], json!(format!("{{{{bot_field:{target}}}}}")));
}

#[tokio::test]
async fn import_flow_export_an_empty_manifest_short_circuits_resolution_nodes_pass_through_unmapped()
 {
    let db = two_workspaces().await;
    let nodes = vec![node(json!([]))];
    let out = import_flow_export(&db, input(nodes.clone()))
        .await
        .expect("imported");
    assert!(out.created_custom_field_ids.is_empty());
    assert!(out.created_bot_field_ids.is_empty());
    let (stored_nodes, _) = stored(&db, out.flow_id).await;
    assert_eq!(stored_nodes, Value::Array(nodes));
}

#[tokio::test]
async fn import_flow_export_a_failed_flow_insert_rolls_back_and_leaves_no_orphaned_custom_fields() {
    // Adapted: the insert fails on an unpublishable graph (no node is valid)
    // instead of a mocked insert error; the call rejects and the created
    // field is rolled back with it.
    let db = two_workspaces().await;
    let mut request = input(vec![json!({ "id": "1" })]);
    request.custom_fields = vec![("42".into(), entry("Birthday", CustomFieldType::Date))];
    assert!(import_flow_export(&db, request).await.is_err());
    let fields = rows(
        &db,
        r#"SELECT "id" FROM "CustomField" WHERE "workspaceId" = ?1"#,
        WS,
    )
    .await;
    assert!(fields.is_empty());
    let flows = rows(
        &db,
        r#"SELECT "id" FROM "Flow" WHERE "workspaceId" = ?1"#,
        WS,
    )
    .await;
    assert!(flows.is_empty());
}

async fn import_into_folder(db: &TestDb, folder_id: Option<&str>) -> Value {
    let mut request = input(vec![node(json!([]))]);
    request.folder_id = folder_id.map(str::to_owned);
    let out = import_flow_export(db, request).await.expect("imported");
    stored(db, out.flow_id).await.1
}

#[tokio::test]
async fn import_flow_export_a_valid_folder_id_is_resolved_and_persisted_on_the_flow_insert() {
    let db = two_workspaces().await;
    setup(&db, |tx| {
        folder(tx, 61, WS, "flow")?;
        Ok(())
    })
    .await;
    assert_eq!(import_into_folder(&db, Some("61")).await, json!(61));
}

#[tokio::test]
async fn import_flow_export_a_folder_missing_in_the_workspace_falls_back_to_null_without_failing_the_import()
 {
    let db = two_workspaces().await;
    setup(&db, |tx| {
        // Another workspace's folder, and a folder of another type.
        folder(tx, 62, common::OTHER_WS, "flow")?;
        folder(tx, 63, WS, "tag")?;
        Ok(())
    })
    .await;
    for id in ["404", "62", "63"] {
        assert_eq!(import_into_folder(&db, Some(id)).await, Value::Null, "{id}");
    }
}

#[tokio::test]
async fn import_flow_export_the_root_folder_sentinel_skips_the_lookup_and_persists_null() {
    let db = two_workspaces().await;
    assert_eq!(import_into_folder(&db, Some("0")).await, Value::Null);
}

#[tokio::test]
async fn import_flow_export_an_omitted_folder_id_skips_the_lookup_and_persists_null() {
    let db = two_workspaces().await;
    assert_eq!(import_into_folder(&db, None).await, Value::Null);
}

// --- field-token-binder ------------------------------------------------

#[test]
fn field_token_binder_binds_only_renamed_unambiguous_names_and_leaves_other_tokens() {
    let target = |name: &str, renamed: bool| {
        Some(ResolvedField {
            id: "1".into(),
            name: name.into(),
            renamed,
        })
    };
    let field = |name: &str, kind: &str, target| BindingInput {
        source_id: name.into(),
        name: name.into(),
        kind: kind.into(),
        target,
    };
    let (bindings, renamed) = compute_custom_field_name_bindings(&[
        field("size", "number", target("size_1", true)),
        field("Email Opt", "text", target("email opt", false)),
        field("same", "text", target("same", false)),
        field("dup", "text", target("dup_1", true)),
        field("dup", "number", target("dup_2", true)),
    ]);
    assert_eq!(bindings.len(), 2);
    assert_eq!(bindings["size"], "size_1");
    assert_eq!(bindings["Email Opt"], "email opt");
    assert_eq!(renamed.len(), 3);
    let text = json!({ "a": ["{{size}} {{ raw:size }} {{first_name}} {{bot_field:size}} {{coupon:size}} {{dup}}"] });
    assert_eq!(
        rewrite_field_tokens_deep(&text, &bindings),
        json!({ "a": ["{{size_1}} {{raw:size_1}} {{first_name}} {{bot_field:size}} {{coupon:size}} {{dup}}"] })
    );
    let ids = [("7".to_owned(), "77".to_owned())].into_iter().collect();
    assert_eq!(
        rewrite_bot_field_tokens_deep(&json!("{{bot_field:7}} {{bot_field:8}} {{7}}"), &ids),
        json!("{{bot_field:77}} {{bot_field:8}} {{7}}")
    );
}

// --- runFlowImport ------------------------------------------------------

fn export_json(nodes: Vec<Value>, extra: Value) -> Value {
    let mut out = json!({
        "formatVersion": 2,
        "exportedAt": "2026-11-16T10:00:00.000Z",
        "source": { "workspaceId": "source-workspace", "flowId": "source-flow" },
        "flows": [{
            "name": "Onboarding",
            "active": true,
            "enableInInbox": true,
            "startNodeId": "1",
            "nodes": nodes,
            "edges": [],
        }],
        "customFields": {},
    });
    for (key, value) in extra.as_object().cloned().unwrap_or_default() {
        out[key] = value;
    }
    out
}

/// The default export: one `subscribeSequence` step on an unknown sequence.
fn default_export() -> Value {
    export_json(
        vec![node(
            json!([{ "id": "3", "stepType": "subscribeSequence", "sequenceId": "999" }]),
        )],
        json!({}),
    )
}

/// Uploads `body`, starts the import and runs it; returns the import row.
async fn run(db: &TestDb, body: Vec<u8>) -> serde_json::Map<String, Value> {
    file(db, 1, "import", "flow").await;
    let uploader = Uploader::in_memory();
    uploader
        .put_object(PATH, body, PutOptions::default(), None)
        .await
        .expect("put");
    let import_id = start_flow_import(db, WS, None, 1, None)
        .await
        .unwrap()
        .expect("started");
    run_flow_import(db, &uploader, import_id)
        .await
        .expect("run");
    let mut out = rows(
        db,
        r#"SELECT "status", "totalCount", "processedCount", "successCount", "failedCount", "errorSample", "errorMessage" FROM "Import" WHERE "id" = ?1"#,
        import_id,
    )
    .await
    .remove(0);
    let sample = out["errorSample"]
        .as_str()
        .map(|s| serde_json::from_str(s).unwrap())
        .unwrap_or(Value::Null);
    out.insert("errorSample".into(), sample);
    out
}

async fn run_json(db: &TestDb, export: &Value) -> serde_json::Map<String, Value> {
    run(db, serde_json::to_vec(export).unwrap()).await
}

async fn flow_count(db: &TestDb) -> usize {
    rows(
        db,
        r#"SELECT "id" FROM "Flow" WHERE "workspaceId" = ?1"#,
        WS,
    )
    .await
    .len()
}

#[tokio::test]
async fn run_flow_import_inserts_a_new_flow_row_and_reports_a_warning_for_an_unresolved_sequence_id()
 {
    let db = two_workspaces().await;
    let row = run_json(&db, &default_export()).await;
    assert_eq!(flow_count(&db).await, 1);
    let flow = rows(
        &db,
        r#"SELECT "name" FROM "Flow" WHERE "workspaceId" = ?1"#,
        WS,
    )
    .await;
    assert_eq!(flow[0]["name"], json!("Onboarding"));
    assert_eq!(row["status"], json!("completed"));
    assert_eq!(row["successCount"], json!(1));
    assert_eq!(row["failedCount"], json!(0));
    let sample = row["errorSample"].as_array().expect("sample");
    assert_eq!(sample.len(), 1);
    assert!(sample[0]["path"].is_string());
    assert!(sample[0]["reason"].as_str().unwrap().contains("sequence"));
    // A dangling reference must not look like a clean import.
    assert!(
        row["errorMessage"]
            .as_str()
            .unwrap()
            .contains("unresolved reference")
    );
}

#[tokio::test]
async fn run_flow_import_passes_nodes_byte_for_byte_when_the_manifest_is_empty() {
    // Adapted: the stored draft is compared (the original checks the
    // service input); the graph has no reference the sanitizer would clear.
    let db = two_workspaces().await;
    let export = export_json(vec![node(json!([]))], json!({}));
    run_json(&db, &export).await;
    let flow = rows(
        &db,
        r#"SELECT "id" FROM "Flow" WHERE "workspaceId" = ?1"#,
        WS,
    )
    .await;
    let (nodes, _) = stored(&db, flow[0]["id"].as_i64().unwrap()).await;
    assert_eq!(nodes, export["flows"][0]["nodes"]);
}

#[tokio::test]
async fn run_flow_import_fails_the_import_without_inserting_a_flow_when_format_version_is_unknown()
{
    let db = two_workspaces().await;
    let mut export = default_export();
    export["formatVersion"] = json!(999);
    let row = run_json(&db, &export).await;
    assert_eq!(flow_count(&db).await, 0);
    assert_eq!(row["status"], json!("failed"));
}

#[tokio::test]
async fn run_flow_import_persists_every_tiktok_unsupported_step_capability_error_without_importing()
{
    let db = two_workspaces().await;
    let mut export = default_export();
    let details = &mut export["flows"][0]["nodes"][0]["data"]["details"];
    details["beforeStep"]["channel"] = json!("tiktok");
    details["steps"] = json!([
        { "id": "4", "stepType": "sendVideo", "mode": "url", "url": "https://example.com/video.mp4", "buttons": [] },
        { "id": "5", "stepType": "sendFile", "mode": "url", "url": "https://example.com/file.pdf", "buttons": [] },
    ]);
    let row = run_json(&db, &export).await;
    assert_eq!(flow_count(&db).await, 0);
    assert_eq!(row["status"], json!("failed"));
    assert_eq!(row["failedCount"], json!(1));
    assert_eq!(row["processedCount"], json!(1));
    assert_eq!(row["successCount"], json!(0));
    assert_eq!(
        row["errorMessage"],
        json!("Flow uses steps not supported by tiktok: sendVideo, sendFile.")
    );
    let sample = row["errorSample"].as_array().expect("sample");
    assert_eq!(sample.len(), 2);
    for (item, block) in sample.iter().zip(["sendVideo", "sendFile"]) {
        assert_eq!(item["code"], json!("unsupportedBlock"));
        assert_eq!(item["capability"]["block"], json!(block));
        assert_eq!(item["capability"]["channel"], json!("tiktok"));
    }
}

#[test]
fn run_flow_import_summarizes_channel_policy_errors_raised_while_persisting_an_otherwise_valid_flow()
 {
    // Adapted: the mocked `importFlowExport` rejection becomes the summary of
    // the same authoring errors (the persist path uses this summary).
    let errors = vec![FlowAuthoringError {
        capability: Some(
            json!({ "block": "sendVideo", "channel": "tiktok", "code": "unsupportedBlock", "policyVersion": 1 }),
        ),
        code: "unsupportedBlock".into(),
        message: "The tiktok channel does not support sendVideo.".into(),
        path: "nodes[0].data.details.steps[0]".into(),
        ..FlowAuthoringError::default()
    }];
    assert_eq!(
        summarize_schema_error(&errors),
        "Flow uses steps not supported by tiktok: sendVideo."
    );
}

#[tokio::test]
async fn run_flow_import_fails_the_import_when_the_stream_exceeds_the_byte_limit() {
    let db = two_workspaces().await;
    let row = run(&db, vec![b' '; 6 * 1024 * 1024]).await;
    assert_eq!(flow_count(&db).await, 0);
    assert_eq!(row["status"], json!("failed"));
    assert!(
        row["errorMessage"]
            .as_str()
            .unwrap()
            .contains("File exceeds")
    );
}

#[tokio::test]
async fn run_flow_import_does_not_warn_about_a_custom_field_reference_that_was_successfully_remapped()
 {
    let db = two_workspaces().await;
    let export = export_json(
        vec![node(json!([set_field_step("41", "1990-01-01")]))],
        json!({ "customFields": { "41": { "name": "Birthday", "type": "date" } } }),
    );
    let row = run_json(&db, &export).await;
    assert_eq!(row["status"], json!("completed"));
    assert_eq!(row["errorMessage"], Value::Null);
}

#[tokio::test]
async fn run_flow_import_still_warns_about_a_custom_field_reference_absent_from_the_manifest() {
    // Adapted: the original mocks `importFlowExport`. For real, the sanitizer
    // clears the unresolved required `inputFieldId`, so the publish check
    // rejects the graph and the import fails (DECISIONS 2026-11-16 flow import).
    let db = two_workspaces().await;
    let export = export_json(
        vec![node(json!([set_field_step("999", "1990-01-01")]))],
        json!({}),
    );
    let row = run_json(&db, &export).await;
    assert_eq!(row["status"], json!("failed"));
    assert_eq!(row["failedCount"], json!(1));
    assert!(
        row["errorMessage"]
            .as_str()
            .unwrap()
            .starts_with("Invalid export file")
    );
    assert_eq!(flow_count(&db).await, 0);
}

#[tokio::test]
async fn run_flow_import_does_not_warn_about_a_bot_field_reference_that_was_successfully_remapped()
{
    let db = two_workspaces().await;
    let export = export_json(
        vec![node(json!([set_field_step("bot_field:7", "42")]))],
        json!({ "botFields": { "7": { "name": "Loyalty Points", "type": "number" } } }),
    );
    let row = run_json(&db, &export).await;
    assert_eq!(row["status"], json!("completed"));
    assert_eq!(row["errorMessage"], Value::Null);
}

#[tokio::test]
async fn run_flow_import_still_warns_about_a_bot_field_reference_absent_from_the_manifest() {
    // Adapted: the original mocks `importFlowExport`. For real, the sanitizer
    // clears the unresolved required `inputFieldId`, so the publish check
    // rejects the graph and the import fails (DECISIONS 2026-11-16 flow import).
    let db = two_workspaces().await;
    let export = export_json(
        vec![node(json!([set_field_step("bot_field:999", "42")]))],
        json!({ "botFields": {} }),
    );
    let row = run_json(&db, &export).await;
    assert_eq!(row["status"], json!("failed"));
    assert_eq!(row["failedCount"], json!(1));
    assert!(
        row["errorMessage"]
            .as_str()
            .unwrap()
            .starts_with("Invalid export file")
    );
    assert_eq!(flow_count(&db).await, 0);
}

#[tokio::test]
async fn run_flow_import_reports_a_renamed_custom_field_in_the_import_s_sample_and_warning() {
    let db = two_workspaces().await;
    setup(&db, |tx| {
        custom_field(tx, 900, WS, "size", "shortText")?;
        Ok(())
    })
    .await;
    let mut export = default_export();
    export["customFields"] = json!({ "9": { "name": "size", "type": "number" } });
    let row = run_json(&db, &export).await;
    assert!(
        row["errorSample"][0]["reason"]
            .as_str()
            .unwrap()
            .contains("\"size_1\"")
    );
    assert!(
        row["errorMessage"]
            .as_str()
            .unwrap()
            .contains("1 renamed custom field")
    );
}

#[tokio::test]
async fn run_flow_import_reports_a_renamed_bot_field_in_the_import_s_sample_and_warning() {
    let db = two_workspaces().await;
    setup(&db, |tx| {
        bot_field(tx, 900, WS, "score", "shortText", None, 1)?;
        Ok(())
    })
    .await;
    let mut export = default_export();
    export["botFields"] = json!({ "9": { "name": "score", "type": "number" } });
    let row = run_json(&db, &export).await;
    assert!(
        row["errorSample"][0]["reason"]
            .as_str()
            .unwrap()
            .contains("\"score_1\"")
    );
    assert!(
        row["errorMessage"]
            .as_str()
            .unwrap()
            .contains("1 renamed bot field")
    );
}

#[tokio::test]
async fn run_flow_import_an_export_with_no_bot_fields_key_still_imports_cleanly() {
    let db = two_workspaces().await;
    let export = default_export();
    assert!(export.get("botFields").is_none());
    let row = run_json(&db, &export).await;
    assert_eq!(row["status"], json!("completed"));
}
