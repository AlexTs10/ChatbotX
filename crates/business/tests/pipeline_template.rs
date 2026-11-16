//! Own smoke tests of `pipeline::template` (`pipeline-template.service.ts` has
//! no TypeScript test). Real rows in a migrated SQLite file.

mod common;

use chatbotx_business::pipeline::template::{
    TemplateEntryResolution, TemplateField, TemplatePipelineEntry, TemplateStage,
    resolve_template_entry,
};
use chatbotx_business::tx;
use chatbotx_db::Db;
use common::{WS, rows, setup, two_workspaces};
use serde_json::json;

fn stage(source_id: &str, name: &str) -> TemplateStage {
    TemplateStage {
        source_id: source_id.into(),
        name: name.into(),
        ..TemplateStage::default()
    }
}

fn field(source_id: &str, reference_name: &str, kind: &str) -> TemplateField {
    TemplateField {
        source_id: source_id.into(),
        name: format!("Field {reference_name}"),
        reference_name: reference_name.into(),
        r#type: kind.into(),
        required: false,
    }
}

fn entry() -> TemplatePipelineEntry {
    TemplatePipelineEntry {
        name: "Sales".into(),
        reference_name: "sales".into(),
        r#type: "opportunity".into(),
        currency: Some("EUR".into()),
        settings: json!({}),
        stages: vec![stage("s1", "Lead"), stage("s2", "Won deal")],
        fields: vec![field("f1", "budget", "number")],
    }
}

async fn resolve(db: &Db, entry: TemplatePipelineEntry) -> TemplateEntryResolution {
    tx::write(db, move |conn| resolve_template_entry(conn, WS, &entry))
        .await
        .expect("resolveTemplateEntry")
}

async fn stages_of(db: &Db, pipeline_id: i64) -> Vec<(String, i64)> {
    rows(
        db,
        r#"SELECT "name", "position" FROM "PipelineStage" WHERE "pipelineId" = ?1 ORDER BY "position""#,
        pipeline_id,
    )
    .await
    .into_iter()
    .map(|row| {
        (
            row["name"].as_str().unwrap_or_default().to_string(),
            row["position"].as_i64().unwrap_or(-1),
        )
    })
    .collect()
}

#[tokio::test]
async fn creates_the_pipeline_its_stages_and_fields_for_a_new_reference_name() {
    let db = two_workspaces().await;
    let out = resolve(&db, entry()).await;
    assert!(out.created);
    let pipeline = rows(
        &db,
        r#"SELECT "name", "referenceName", "type", "currency", "workspaceId" FROM "Pipeline" WHERE "id" = ?1"#,
        out.pipeline_id,
    )
    .await;
    assert_eq!(pipeline[0]["name"], json!("Sales"));
    assert_eq!(pipeline[0]["referenceName"], json!("sales"));
    assert_eq!(pipeline[0]["type"], json!("opportunity"));
    assert_eq!(pipeline[0]["currency"], json!("EUR"));
    assert_eq!(pipeline[0]["workspaceId"], json!(WS));
    assert_eq!(
        stages_of(&db, out.pipeline_id).await,
        vec![("Lead".to_string(), 0), ("Won deal".to_string(), 1)]
    );
    assert_eq!(out.stage_ids.len(), 2);
    assert_eq!(out.field_ids.len(), 1);
    let fields = rows(
        &db,
        r#"SELECT "id", "referenceName", "type" FROM "PipelineField" WHERE "pipelineId" = ?1"#,
        out.pipeline_id,
    )
    .await;
    assert_eq!(fields.len(), 1);
    assert_eq!(fields[0]["referenceName"], json!("budget"));
    assert_eq!(fields[0]["type"], json!("number"));
    assert_eq!(fields[0]["id"], json!(out.field_ids["f1"]));
    assert!(out.unmatched_stages.is_empty() && out.unmatched_fields.is_empty());
}

#[tokio::test]
async fn a_new_pipeline_without_template_stages_gets_the_type_first_stage() {
    let db = two_workspaces().await;
    let mut ticket = entry();
    ticket.r#type = "ticket".into();
    ticket.stages.clear();
    let out = resolve(&db, ticket).await;
    assert_eq!(
        stages_of(&db, out.pipeline_id).await,
        vec![("New Issue".to_string(), 0)]
    );
    assert!(out.stage_ids.is_empty());
}

#[tokio::test]
async fn reuses_the_pipeline_with_the_reference_name_and_matches_stages_and_fields() {
    let db = two_workspaces().await;
    let first = resolve(&db, entry()).await;
    let mut again = entry();
    // Name match is trimmed and case-insensitive; a new stage and field are added.
    again.stages = vec![
        stage("t1", "  lead "),
        stage("t2", "Won deal"),
        stage("t3", "Lost"),
    ];
    again.fields = vec![
        field("g1", "budget", "number"),
        field("g2", "source", "shortText"),
    ];
    let out = resolve(&db, again).await;
    assert!(!out.created);
    assert_eq!(out.pipeline_id, first.pipeline_id);
    assert_eq!(out.stage_ids["t1"], first.stage_ids["s1"]);
    assert_eq!(out.stage_ids["t2"], first.stage_ids["s2"]);
    assert_eq!(
        stages_of(&db, out.pipeline_id).await,
        vec![
            ("Lead".to_string(), 0),
            ("Won deal".to_string(), 1),
            ("Lost".to_string(), 2)
        ]
    );
    assert_eq!(out.field_ids["g1"], first.field_ids["f1"]);
    assert!(out.field_ids.contains_key("g2"));
}

#[tokio::test]
async fn each_existing_stage_is_matched_once_and_a_field_of_another_type_is_unmatched() {
    let db = two_workspaces().await;
    let first = resolve(&db, entry()).await;
    let mut again = entry();
    // Two template stages named "Lead": the second one gets a new stage.
    again.stages = vec![stage("t1", "Lead"), stage("t2", "Lead")];
    again.fields = vec![field("g1", "budget", "shortText")];
    let out = resolve(&db, again).await;
    assert_eq!(out.stage_ids["t1"], first.stage_ids["s1"]);
    assert_ne!(out.stage_ids["t2"], first.stage_ids["s1"]);
    assert_eq!(stages_of(&db, out.pipeline_id).await.len(), 3);
    assert_eq!(out.unmatched_fields, vec!["g1".to_string()]);
    assert!(!out.field_ids.contains_key("g1"));
}

#[tokio::test]
async fn invalid_currency_colors_and_settings_fall_back() {
    let db = two_workspaces().await;
    let mut odd = entry();
    odd.currency = Some("euro".into());
    odd.settings = json!({ "statusLabels": 7 });
    odd.stages = vec![TemplateStage {
        source_id: "s1".into(),
        name: "Lead".into(),
        description: Some("First contact".into()),
        text_color: Some("red".into()),
        background_color: Some("#AABBCC".into()),
    }];
    let out = resolve(&db, odd).await;
    let pipeline = rows(
        &db,
        r#"SELECT "currency", "settings" FROM "Pipeline" WHERE "id" = ?1"#,
        out.pipeline_id,
    )
    .await;
    assert_eq!(pipeline[0]["currency"], json!(null));
    let settings: serde_json::Value =
        serde_json::from_str(pipeline[0]["settings"].as_str().expect("settings text"))
            .expect("settings JSON");
    assert_eq!(
        settings["statusLabels"],
        json!({ "open": "Open", "won": "Won", "lost": "Lost" })
    );
    let stage = rows(
        &db,
        r#"SELECT "description", "textColor", "backgroundColor" FROM "PipelineStage" WHERE "pipelineId" = ?1"#,
        out.pipeline_id,
    )
    .await;
    assert_eq!(stage[0]["description"], json!("First contact"));
    assert_eq!(stage[0]["textColor"], json!(null));
    assert_eq!(stage[0]["backgroundColor"], json!("#AABBCC"));
}

#[tokio::test]
async fn a_taken_pipeline_name_gets_a_free_name_but_keeps_the_reference_name() {
    let db = two_workspaces().await;
    setup(&db, |tx| {
        common::pipeline_row(tx, 4001, WS, "Sales", None)?;
        Ok(())
    })
    .await;
    let out = resolve(&db, entry()).await;
    assert!(out.created);
    let pipeline = rows(
        &db,
        r#"SELECT "name", "referenceName" FROM "Pipeline" WHERE "id" = ?1"#,
        out.pipeline_id,
    )
    .await;
    assert_ne!(pipeline[0]["name"], json!("Sales"));
    assert_eq!(pipeline[0]["referenceName"], json!("sales"));
}
