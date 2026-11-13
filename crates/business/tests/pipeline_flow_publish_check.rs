//! Port of `packages/business/__tests__/pipeline-flow-publish-check.test.ts`.
//! Adapted: the mocked `listFieldsByPipelineIds` is real `"PipelineField"`
//! rows (pipeline 1: Budget required, Notes optional; pipeline 2: Company
//! required). The mock call-count assertions ("loads fields once", "without
//! a query") have no counterpart; the outcome is asserted.

mod common;

use chatbotx_business::BusinessError;
use chatbotx_business::pipeline::flow_publish_check::assert_flow_pipeline_steps_publishable;
use chatbotx_core::flow_config::pipeline_step_default_fn;

use common::{TestDb, WS, pipeline_field_row, pipeline_row, setup, two_workspaces};
use serde_json::{Value, json};

async fn board() -> TestDb {
    let db = two_workspaces().await;
    setup(&db, |tx| {
        pipeline_row(tx, 1, WS, "Sales", None)?;
        pipeline_row(tx, 2, WS, "Support", None)?;
        pipeline_field_row(tx, 10, WS, 1, "Budget", true)?;
        pipeline_field_row(tx, 11, WS, 1, "Notes", false)?;
        pipeline_field_row(tx, 20, WS, 2, "Company", true)
    })
    .await;
    db
}

fn add_to_pipeline(pipeline_id: &str, field_values: Value) -> Value {
    let mut step = pipeline_step_default_fn("addToPipeline", &json!({ "pipelineId": pipeline_id }));
    step["fieldValues"] = field_values;
    step
}

fn node(name: &str, steps: Vec<Value>) -> Value {
    json!({ "id": "n1", "data": { "name": name, "details": { "steps": steps } } })
}

fn validation_message(result: Result<(), BusinessError>) -> String {
    match result {
        Err(error) => {
            assert_eq!(error.code(), Some("validation"), "{error:?}");
            assert_eq!(error.status(), Some(422), "{error:?}");
            error.message()
        }
        Ok(()) => panic!("expected a validation error"),
    }
}

#[tokio::test]
async fn rejects_an_add_to_pipeline_step_that_leaves_a_required_field_empty_naming_the_node_and_field()
 {
    let db = board().await;
    let nodes = json!([node("Qualify lead", vec![add_to_pipeline("1", json!([]))])]);

    let result = assert_flow_pipeline_steps_publishable(&db, WS, &nodes).await;

    let message = validation_message(result);
    assert!(message.contains("\"Qualify lead\": Budget"), "{message}");
}

#[tokio::test]
async fn only_checks_the_fields_of_the_step_s_own_pipeline() {
    let db = board().await;
    let nodes = json!([node(
        "Qualify lead",
        vec![add_to_pipeline(
            "1",
            json!([{ "pipelineFieldId": "10", "value": "500" }])
        )]
    )]);

    assert!(
        assert_flow_pipeline_steps_publishable(&db, WS, &nodes)
            .await
            .is_ok()
    );
}

#[tokio::test]
async fn reports_every_failing_step_in_one_error() {
    let db = board().await;
    let nodes = json!([
        node("Node A", vec![add_to_pipeline("1", json!([]))]),
        node("Node B", vec![add_to_pipeline("2", json!([]))]),
    ]);

    let message = validation_message(assert_flow_pipeline_steps_publishable(&db, WS, &nodes).await);

    assert!(message.contains("\"Node A\": Budget"), "{message}");
    assert!(message.contains("\"Node B\": Company"), "{message}");
}

#[tokio::test]
async fn a_variable_value_is_accepted() {
    let db = board().await;
    let nodes = json!([node(
        "Qualify lead",
        vec![add_to_pipeline(
            "1",
            json!([{ "pipelineFieldId": "10", "value": "{{budget}}" }])
        )]
    )]);

    assert!(
        assert_flow_pipeline_steps_publishable(&db, WS, &nodes)
            .await
            .is_ok()
    );
}

#[tokio::test]
async fn ignores_other_pipeline_actions_and_flows_without_pipeline_steps() {
    let db = board().await;
    let nodes = json!([
        node(
            "Move",
            vec![pipeline_step_default_fn("moveToStage", &json!({ "pipelineId": "1" }))]
        ),
        node("Text", vec![json!({ "stepType": "sendText", "text": "hi" })]),
        { "id": "start", "data": {} },
    ]);

    assert!(
        assert_flow_pipeline_steps_publishable(&db, WS, &nodes)
            .await
            .is_ok()
    );
}

#[tokio::test]
async fn a_deleted_pipeline_no_fields_does_not_block_publish() {
    let db = board().await;
    let nodes = json!([node("Gone", vec![add_to_pipeline("9", json!([]))])]);

    assert!(
        assert_flow_pipeline_steps_publishable(&db, WS, &nodes)
            .await
            .is_ok()
    );
}
