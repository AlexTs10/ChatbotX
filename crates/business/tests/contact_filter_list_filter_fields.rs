//! Port of `apps/builder/src/features/contact-filter/lib/__tests__/list-filter-fields.test.ts`
//! (P3.6-D2 slice filter fields). Adapted: the mocked `customFieldService.list`,
//! `botFieldService.list`, `tagService.listActive` and
//! `pipelineService.listWithStages` become real rows of a migrated database
//! (rows of another workspace stand for "scoped by workspaceId"); the
//! `isContactActivityFiltersEnabled` mock is the `tracked_fields_enabled`
//! argument.

mod common;

use chatbotx_business::contact::filter_fields::list_contact_filter_fields_for_api;
use chatbotx_business::contact_filter::Surface;
use chatbotx_db::Db;
use chatbotx_db::rusqlite::params;
use common::{TestDb, bot_field, custom_field, fresh_db, pipeline_row, setup, tag, workspace};
use serde_json::{Value, json};

const WS: i64 = 2001;
const OTHER_WS: i64 = 2002;

async fn db() -> TestDb {
    let db = fresh_db().await;
    setup(&db, |tx| {
        workspace(tx, 1001, WS)?;
        workspace(tx, 1002, OTHER_WS)?;
        Ok(())
    })
    .await;
    db
}

async fn list(db: &Db, surface: Surface) -> Value {
    list_contact_filter_fields_for_api(db, WS, surface, true)
        .await
        .expect("listContactFilterFieldsForAPI")
}

fn names(result: &Value) -> Vec<String> {
    result["staticFields"]
        .as_array()
        .expect("staticFields")
        .iter()
        .map(|f| f["field"].as_str().unwrap_or_default().to_string())
        .collect()
}

fn find<'a>(result: &'a Value, field: &str) -> &'a Value {
    result["staticFields"]
        .as_array()
        .and_then(|rows| rows.iter().find(|f| f["field"] == field))
        .unwrap_or(&Value::Null)
}

fn contains_all(list: &Value, items: &[&str]) -> bool {
    let list = list.as_array().cloned().unwrap_or_default();
    items.iter().all(|item| list.contains(&json!(item)))
}

fn parse_ms(text: &Value) -> Option<i64> {
    text.as_str()
        .and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok())
        .map(|d| d.timestamp_millis())
}

// ---- listContactFilterFieldsForAPI ------------------------------------------

#[tokio::test]
async fn list_contact_filter_fields_for_api_scopes_every_lookup_by_workspace_id() {
    let db = db().await;
    setup(&db, |tx| {
        custom_field(tx, 11, OTHER_WS, "Other field", "shortText")?;
        bot_field(tx, 12, OTHER_WS, "OtherBot", "number", None, 1)?;
        tag(tx, 13, OTHER_WS, "Other tag")?;
        Ok(())
    })
    .await;

    let result = list(&db, Surface::Contacts).await;

    assert_eq!(result["customFields"], json!([]));
    assert_eq!(result["botFields"], json!([]));
    assert_eq!(result["tags"], json!([]));
}

#[tokio::test]
async fn list_contact_filter_fields_for_api_excludes_hidden_static_fields_e_g_legacy_locale_existing_contact()
 {
    let db = db().await;

    let result = list(&db, Surface::Contacts).await;

    let field_names = names(&result);
    assert!(!field_names.contains(&"locale".to_string()));
    assert!(!field_names.contains(&"existingContact".to_string()));
    assert!(field_names.contains(&"email".to_string()));
    assert!(field_names.contains(&"tags".to_string()));
}

#[tokio::test]
async fn list_contact_filter_fields_for_api_each_static_field_carries_its_enabled_operator_list() {
    let db = db().await;

    let result = list(&db, Surface::Contacts).await;

    let email_field = find(&result, "email");
    assert!(contains_all(&email_field["operators"], &["eq", "contains"]));
}

#[tokio::test]
async fn list_contact_filter_fields_for_api_derives_each_custom_bot_field_s_value_type_from_its_raw_custom_field_type()
 {
    let db = db().await;
    setup(&db, |tx| {
        custom_field(tx, 1, WS, "Company", "shortText")?;
        tx.execute(
            r#"UPDATE "CustomField" SET "createdAt" = 2000 WHERE "id" = 1"#,
            [],
        )?;
        custom_field(tx, 2, WS, "Signup date", "date")?;
        tx.execute(
            r#"UPDATE "CustomField" SET "createdAt" = 1000 WHERE "id" = 2"#,
            [],
        )?;
        bot_field(tx, 3, WS, "LastIntent", "number", None, 1)?;
        tag(tx, 4, WS, "VIP")?;
        Ok(())
    })
    .await;

    let result = list(&db, Surface::Contacts).await;

    assert_eq!(
        result["customFields"],
        json!([
            {
                "id": "2",
                "name": "Signup date",
                "type": "date",
                "valueType": "datetime",
            },
            { "id": "1", "name": "Company", "type": "shortText", "valueType": "text" },
        ])
    );
    assert_eq!(
        result["botFields"],
        json!([{ "id": "3", "name": "LastIntent", "type": "number", "valueType": "number" }])
    );
    assert_eq!(result["tags"], json!([{ "id": "4", "name": "VIP" }]));
}

#[tokio::test]
async fn list_contact_filter_fields_for_api_lists_the_flow_only_working_hours_field_on_the_flow_condition_surface_only()
 {
    let db = db().await;

    let contacts = names(&list(&db, Surface::Contacts).await);
    let flow = names(&list(&db, Surface::FlowCondition).await);

    // No surface = the default `contacts` surface.
    assert!(!contacts.contains(&"isWithinWorkingHours".to_string()));
    assert!(flow.contains(&"isWithinWorkingHours".to_string()));
    assert!(flow.contains(&"email".to_string()));
}

// ---- listContactFilterFieldsForAPI > activity-backed fields -----------------

#[tokio::test]
async fn list_contact_filter_fields_for_api_activity_backed_fields_tells_an_agent_where_each_field_s_values_come_from()
 {
    let db = db().await;
    let result = list(&db, Surface::Contacts).await;

    let executed_flow = &find(&result, "executedFlow")["valueSource"];
    assert_eq!(executed_flow["operation"], json!("flows.list"));
    assert_eq!(executed_flow["scope"], json!("automation"));
    assert_eq!(executed_flow["fixedValues"], json!([]));
    assert_eq!(executed_flow["valueFormat"], json!("id"));
    let executed_step = &find(&result, "executedStep")["valueSource"];
    assert_eq!(executed_step["operation"], json!("flows.listNodes"));
    assert_eq!(executed_step["scope"], json!("automation"));
    assert_eq!(executed_step["valueFormat"], json!("flowId:nodeId"));
    assert_eq!(
        find(&result, "keywordsReceived")["valueSource"]["operation"],
        json!("keywords.list")
    );
    assert_eq!(
        find(&result, "completedWhatsAppFlows")["valueSource"]["operation"],
        json!("whatsappFlows.list")
    );
    let email_opened = &find(&result, "emailOpened")["valueSource"];
    assert_eq!(email_opened["operation"], json!("emailTopics.list"));
    assert_eq!(email_opened["scope"], json!("broadcasts"));
    assert_eq!(email_opened["fixedValues"], json!(["noTopic"]));
}

#[tokio::test]
async fn list_contact_filter_fields_for_api_activity_backed_fields_clicked_ad_points_an_agent_at_the_ads_it_can_pick_and_starts_later_than_the_store()
 {
    let db = db().await;
    let result = list(&db, Surface::Contacts).await;
    let value_source = &find(&result, "clickedAd")["valueSource"];

    assert_eq!(value_source["operation"], json!("ads.listReferralAds"));
    assert_eq!(value_source["scope"], json!("ads"));
    assert_eq!(value_source["fixedValues"], json!([]));
    assert_eq!(value_source["valueFormat"], json!("id"));
    // Ad clicks were not recorded from the day the activity store was.
    let clicked = parse_ms(&value_source["trackingStartedAt"]).expect("clickedAd date");
    let flow = parse_ms(&find(&result, "executedFlow")["valueSource"]["trackingStartedAt"])
        .expect("executedFlow date");
    assert!(clicked > flow);
}

#[tokio::test]
async fn list_contact_filter_fields_for_api_activity_backed_fields_free_form_tracked_fields_carry_only_their_format_and_baseline()
 {
    let db = db().await;
    let result = list(&db, Surface::Contacts).await;

    let sent = &find(&result, "sentMessage")["valueSource"];
    assert_eq!(sent["fixedValues"], json!([]));
    assert_eq!(sent["valueFormat"], json!("text"));
    assert!(sent.get("operation").is_none());
    assert_eq!(
        find(&result, "lastDelivered")["valueSource"]["valueFormat"],
        json!("datetime")
    );
}

#[tokio::test]
async fn list_contact_filter_fields_for_api_activity_backed_fields_reports_when_tracking_began_for_every_tracked_field_no_backfill()
 {
    let db = db().await;
    let result = list(&db, Surface::Contacts).await;

    for field in [
        "executedFlow",
        "executedStep",
        "keywordsReceived",
        "completedWhatsAppFlows",
        "clickedAd",
        "emailSent",
        "emailDelivered",
        "emailOpened",
        "emailClicked",
        "sentMessage",
        "lastDelivered",
    ] {
        let tracking_started_at = &find(&result, field)["valueSource"]["trackingStartedAt"];
        assert!(!tracking_started_at.is_null(), "{field}");
        assert!(parse_ms(tracking_started_at).is_some(), "{field}");
    }
}

#[tokio::test]
async fn list_contact_filter_fields_for_api_activity_backed_fields_other_fields_have_no_value_source()
 {
    let db = db().await;
    let result = list(&db, Surface::Contacts).await;

    assert!(find(&result, "tags").get("valueSource").is_none());
    assert!(find(&result, "email").get("valueSource").is_none());
}

#[tokio::test]
async fn list_contact_filter_fields_for_api_activity_backed_fields_lists_the_same_operators_the_schema_validates()
 {
    let db = db().await;
    let result = list(&db, Surface::Contacts).await;

    assert!(contains_all(
        &find(&result, "executedFlow")["operators"],
        &["in", "notIn", "eq", "ne", "isEmpty"]
    ));
    assert!(contains_all(
        &find(&result, "lastDelivered")["operators"],
        &["isNotEmpty", "isBetween"]
    ));
}

#[tokio::test]
async fn list_contact_filter_fields_for_api_activity_backed_fields_omits_every_tracked_field_when_the_kill_switch_is_off()
 {
    let db = db().await;
    let result = list_contact_filter_fields_for_api(&db, WS, Surface::Contacts, false)
        .await
        .expect("listContactFilterFieldsForAPI");
    let names = names(&result);

    for field in [
        "executedFlow",
        "executedStep",
        "keywordsReceived",
        "completedWhatsAppFlows",
        "emailSent",
        "emailDelivered",
        "emailOpened",
        "emailClicked",
        "sentMessage",
        "lastDelivered",
    ] {
        assert!(!names.contains(&field.to_string()), "{field}");
    }
    assert!(names.contains(&"tags".to_string()));
}

// ---- listContactFilterFieldsForAPI pipelineItems ----------------------------

#[tokio::test]
async fn list_contact_filter_fields_for_api_pipeline_items_lists_the_token_visible_pipelines_with_stages_and_the_four_attribute_shapes()
 {
    let db = db().await;
    setup(&db, |tx| {
        pipeline_row(tx, 1, WS, "Sales", None)?;
        for (id, name, position) in [(11_i64, "Lead", 0_i64), (12, "Won", 1)] {
            tx.execute(
                r#"INSERT INTO "PipelineStage" ("id", "workspaceId", "pipelineId", "name", "position") VALUES (?1, ?2, 1, ?3, ?4)"#,
                params![id, WS, name, position],
            )?;
        }
        pipeline_row(tx, 2, OTHER_WS, "Other", None)?;
        Ok(())
    })
    .await;

    let result = list(&db, Surface::Contacts).await;

    assert_eq!(
        result["pipelineItems"]["pipelines"],
        json!([
            {
                "id": "1",
                "name": "Sales",
                "stages": [
                    { "id": "11", "name": "Lead" },
                    { "id": "12", "name": "Won" },
                ],
            },
        ])
    );
    assert_eq!(
        result["pipelineItems"]["attributes"],
        json!([
            {
                "attribute": "exists",
                "operators": ["isNotEmpty", "isEmpty"],
                "pipelineIdRequired": false,
                "valueShape": "single",
                "values": ["any", "open", "won", "lost"],
            },
            {
                "attribute": "status",
                "operators": ["in", "notIn"],
                "pipelineIdRequired": false,
                "valueShape": "list",
                "values": ["open", "won", "lost"],
            },
            {
                "attribute": "stage",
                "operators": ["in", "notIn"],
                "pipelineIdRequired": true,
                "valueShape": "list",
                "values": [],
            },
            {
                "attribute": "priority",
                "operators": ["in", "notIn"],
                "pipelineIdRequired": false,
                "valueShape": "list",
                "values": ["low", "medium", "high"],
            },
        ])
    );
}

#[tokio::test]
async fn list_contact_filter_fields_for_api_pipeline_items_omits_the_pipeline_items_block_on_the_flow_condition_surface()
 {
    let db = db().await;
    setup(&db, |tx| {
        pipeline_row(tx, 1, WS, "Sales", None)?;
        Ok(())
    })
    .await;

    let flow = list(&db, Surface::FlowCondition).await;
    let contacts = list(&db, Surface::Contacts).await;

    assert_eq!(
        flow["pipelineItems"],
        json!({ "pipelines": [], "attributes": [] })
    );
    assert_eq!(
        contacts["pipelineItems"]["pipelines"]
            .as_array()
            .map(Vec::len),
        Some(1)
    );
    assert_eq!(
        contacts["pipelineItems"]["attributes"]
            .as_array()
            .map(Vec::len),
        Some(4)
    );
}
