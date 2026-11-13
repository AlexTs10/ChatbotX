//! Port of `apps/builder/__tests__/pipelines-public-api.test.ts`.
//! Adapted: the zod request schemas are checked through `handlers::dispatch`
//! (a schema refusal is the `422 invalidRequestData` issue path, an accepted
//! request reaches the service); `toPublicPipeline` is the JSON of a real
//! pipeline; `mergePipelineSettings` is
//! `chatbotx_business::pipeline::settings::merge_pipeline_settings`.

mod handlers_common;

use chatbotx_business::pipeline::settings::{
    PipelineSettingsPatch, StatusLabelsPatch, merge_pipeline_settings,
};
use chatbotx_db::repo::partials::pipeline::parse_pipeline_settings;
use handlers_common::{Fixture, WS, call, fixture};
use serde_json::{Value, json};

async fn new_pipeline(f: &Fixture) -> String {
    let (status, body) = call(
        f,
        "pipelines.create",
        WS,
        &[],
        None,
        Some(json!({ "type": "opportunity", "name": "Sales" })),
    )
    .await;
    assert_eq!(status, 201, "{body}");
    body["id"].as_str().expect("id").to_string()
}

async fn patch_pipeline(f: &Fixture, id: &str, body: Value) -> (u16, Value) {
    call(f, "pipelines.update", WS, &[("id", id)], None, Some(body)).await
}

fn issue_paths(body: &Value) -> Vec<String> {
    body["data"]["issues"]
        .as_array()
        .map(|issues| {
            issues
                .iter()
                .map(|issue| {
                    issue["path"]
                        .as_array()
                        .map(|path| {
                            path.iter()
                                .map(|seg| {
                                    seg.as_str().map_or_else(|| seg.to_string(), str::to_string)
                                })
                                .collect::<Vec<_>>()
                                .join(".")
                        })
                        .unwrap_or_default()
                })
                .collect()
        })
        .unwrap_or_default()
}

fn assert_invalid(status: u16, body: &Value, path: &str) {
    assert_eq!(status, 422, "{body}");
    assert_eq!(body["code"], "invalidRequestData", "{body}");
    assert!(
        issue_paths(body).iter().any(|p| p == path),
        "{path}: {body}"
    );
}

#[tokio::test]
async fn a_settings_patch_keeps_only_what_the_caller_sent() {
    let f = fixture().await;
    let id = new_pipeline(&f).await;
    let (_, before) = call(&f, "pipelines.get", WS, &[("id", &id)], None, None).await;
    let (status, body) = patch_pipeline(
        &f,
        &id,
        json!({ "settings": { "assignContactOnAssign": true } }),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    let mut expected = before["settings"].clone();
    expected["assignContactOnAssign"] = json!(true);
    assert_eq!(body["settings"], expected);
}

#[tokio::test]
async fn update_takes_a_currency_code_null_to_follow_the_workspace_or_nothing() {
    let f = fixture().await;
    let id = new_pipeline(&f).await;
    let (status, body) = patch_pipeline(&f, &id, json!({ "currency": "VND" })).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["currencyOverride"], "VND");
    let (status, body) = patch_pipeline(&f, &id, json!({})).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["currencyOverride"], "VND");
    let (status, body) = patch_pipeline(&f, &id, json!({ "currency": null })).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["currencyOverride"], Value::Null);
}

#[tokio::test]
async fn update_rejects_the_malformed_currency() {
    let f = fixture().await;
    let id = new_pipeline(&f).await;
    for currency in ["vnd", "VN", "VNDD", ""] {
        let (status, body) = patch_pipeline(&f, &id, json!({ "currency": currency })).await;
        assert_invalid(status, &body, "currency");
    }
}

#[tokio::test]
async fn a_pipeline_resource_carries_the_effective_currency_and_the_override() {
    let f = fixture().await;
    f.state
        .db
        .write(|tx| {
            tx.execute(
                r#"UPDATE "Workspace" SET "targetCountry" = 'VN' WHERE "id" = ?1"#,
                [WS],
            )?;
            Ok(())
        })
        .await
        .expect("target country");
    let id = new_pipeline(&f).await;
    let (_, following) = call(&f, "pipelines.get", WS, &[("id", &id)], None, None).await;
    assert_eq!(following["currency"], "VND");
    assert_eq!(following["currencyOverride"], Value::Null);
    let (_, overridden) = patch_pipeline(&f, &id, json!({ "currency": "USD" })).await;
    assert_eq!(overridden["currency"], "USD");
    assert_eq!(overridden["currencyOverride"], "USD");
}

#[test]
fn merging_a_one_group_patch_leaves_every_other_stored_setting_alone() {
    let stored = parse_pipeline_settings(&json!({
        "statusLabels": { "open": "Open", "won": "Resolved", "lost": "Not Resolved" },
        "maxOpenItemsPerContact": 3,
        "cardElements": { "cardId": true },
    }))
    .expect("settings");
    let merged = merge_pipeline_settings(
        &stored,
        &PipelineSettingsPatch {
            assign_contact_on_assign: Some(true),
            status_labels: StatusLabelsPatch {
                won: Some("Done".to_string()),
                ..StatusLabelsPatch::default()
            },
            ..PipelineSettingsPatch::default()
        },
    );
    assert!(merged.assign_contact_on_assign);
    assert_eq!(
        (
            merged.status_labels.open.as_str(),
            merged.status_labels.won.as_str(),
            merged.status_labels.lost.as_str()
        ),
        ("Open", "Done", "Not Resolved")
    );
    assert_eq!(merged.max_open_items_per_contact, Some(3));
    assert!(merged.card_elements.card_id);
}

#[test]
fn a_null_open_item_limit_clears_it_an_omitted_one_keeps_it() {
    let stored =
        parse_pipeline_settings(&json!({ "maxOpenItemsPerContact": 5 })).expect("settings");
    let cleared = merge_pipeline_settings(
        &stored,
        &PipelineSettingsPatch {
            max_open_items_per_contact: Some(None),
            ..PipelineSettingsPatch::default()
        },
    );
    assert_eq!(cleared.max_open_items_per_contact, None);
    let kept = merge_pipeline_settings(&stored, &PipelineSettingsPatch::default());
    assert_eq!(kept.max_open_items_per_contact, Some(5));
}

#[tokio::test]
async fn delete_stage_validates_the_destination_stage_id_as_a_bigint() {
    let f = fixture().await;
    let id = new_pipeline(&f).await;
    let (status, body) = call(
        &f,
        "pipelines.deleteStage",
        WS,
        &[("id", &id), ("stageId", "2")],
        None,
        Some(json!({ "moveItemsToStageId": "x; drop" })),
    )
    .await;
    assert_invalid(status, &body, "moveItemsToStageId");
}
