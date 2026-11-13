//! Port of `packages/database/__tests__/contact-filter-pipeline-item.test.ts` (P3.2b slice
//! D2). The TS rendered the `AND[0].RAW` predicate with `PgDialect`; here the SQLite text and
//! binds of the same one-condition filter, prepared on the migrated schema (adapted: ids bind
//! as INTEGER, the enum casts `::"pipelineItemStatus"` are plain text binds).

mod common;
#[path = "common/repo8.rs"]
mod repo8;

use chatbotx_db::repo::queries::contact_filter::apply_contact_filter;
use chatbotx_db::repo::sql::Sql;
use repo8::{NOW, crit, has, has_param, int, lacks, prepares, text};
use serde_json::{Value as Json, json};

const WORKSPACE_ID: i64 = 11;
const PIPELINE_ID: &str = "22";

/// The predicate of `{ field: "pipelineItem", ...condition }` without the AND parentheses
/// (the TS `AND[0].RAW`), or `None`.
fn render(condition: Json, workspace_id: Option<i64>) -> Option<Sql> {
    let mut c = condition;
    c["field"] = json!("pipelineItem");
    let q = apply_contact_filter(
        &crit(json!({ "operator": "and", "conditions": [c] })),
        workspace_id,
        NOW,
    )?;
    prepares(&q);
    let text = q.text[1..q.text.len() - 1].to_owned();
    Some(Sql::new(text, q.params))
}

fn r(condition: Json) -> Sql {
    render(condition, Some(WORKSPACE_ID)).expect("a predicate")
}

fn starts_exists(q: &Sql) {
    assert!(q.text.starts_with("EXISTS ("), "{}", q.text);
}

fn starts_not_exists(q: &Sql) {
    assert!(q.text.starts_with("NOT EXISTS ("), "{}", q.text);
}

// ts:32 adapted
#[test]
fn pipeline_item_contact_condition_sql_exists_is_not_empty_with_any_status_is_a_workspace_scoped_exists()
 {
    let q = r(json!({
        "attribute": "exists",
        "pipelineId": null,
        "operator": "isNotEmpty",
        "value": "any",
    }));
    starts_exists(&q);
    has(&q, "\"PipelineItem\".\"workspaceId\" =");
    has(&q, "\"PipelineItem\".\"contactId\" =");
    lacks(&q, "\"PipelineItem\".\"pipelineId\"");
    lacks(&q, "\"PipelineItem\".\"status\"");
    has_param(&q, int(WORKSPACE_ID));
}

// ts:50 adapted
#[test]
fn pipeline_item_contact_condition_sql_exists_is_empty_with_a_status_is_not_exists_of_that_status_in_one_pipeline()
 {
    let q = r(json!({
        "attribute": "exists",
        "pipelineId": PIPELINE_ID,
        "operator": "isEmpty",
        "value": "won",
    }));
    starts_not_exists(&q);
    has(&q, "\"PipelineItem\".\"pipelineId\" =");
    has(&q, "\"PipelineItem\".\"status\" =");
    has_param(&q, int(WORKSPACE_ID));
    has_param(&q, int(22));
    has_param(&q, text("won"));
}

/// ts:68 "%s in/notIn bind every value and negate with NOT EXISTS".
fn membership(attribute: &str, value: &[&str], column: &str) {
    let positive = r(json!({
        "attribute": attribute,
        "pipelineId": null,
        "operator": "in",
        "value": value,
    }));
    let negative = r(json!({
        "attribute": attribute,
        "pipelineId": null,
        "operator": "notIn",
        "value": value,
    }));
    starts_exists(&positive);
    has(&positive, &format!("\"PipelineItem\".\"{column}\" IN ("));
    starts_not_exists(&negative);
    for v in value {
        has_param(&positive, text(v));
    }
}

// ts:68 adapted
#[test]
fn pipeline_item_contact_condition_sql_status_in_not_in_bind_every_value_and_negate_with_not_exists()
 {
    membership("status", &["open", "lost"], "status");
}

// ts:68 adapted
#[test]
fn pipeline_item_contact_condition_sql_priority_in_not_in_bind_every_value_and_negate_with_not_exists()
 {
    membership("priority", &["high"], "priority");
}

// ts:86 adapted
#[test]
fn pipeline_item_contact_condition_sql_stage_in_binds_the_stage_ids_inside_the_required_pipeline() {
    let q = r(json!({
        "attribute": "stage",
        "pipelineId": PIPELINE_ID,
        "operator": "in",
        "value": ["5", "6"],
    }));
    starts_exists(&q);
    has(&q, "\"PipelineItem\".\"pipelineId\" =");
    has(&q, "\"PipelineItem\".\"stageId\" IN (");
    has_param(&q, int(5));
    has_param(&q, int(6));
    has_param(&q, int(22));
}

// ts:104 adapted
#[test]
fn pipeline_item_contact_condition_sql_stage_not_in_is_not_exists() {
    starts_not_exists(&r(json!({
        "attribute": "stage",
        "pipelineId": PIPELINE_ID,
        "operator": "notIn",
        "value": ["5"],
    })));
}

/// ts:117 "fails closed for %s": the predicate is exactly `FALSE`.
fn fails_closed(condition: Json, workspace_id: Option<i64>) {
    let q = render(condition, workspace_id).expect("a predicate");
    assert_eq!(q.text, "FALSE");
}

// ts:117 adapted
#[test]
fn pipeline_item_contact_condition_sql_fails_closed_for_no_workspace() {
    fails_closed(
        json!({ "attribute": "status", "pipelineId": null, "operator": "in", "value": ["open"] }),
        None,
    );
}

// ts:117 adapted
#[test]
fn pipeline_item_contact_condition_sql_fails_closed_for_stage_without_a_pipeline() {
    fails_closed(
        json!({ "attribute": "stage", "pipelineId": null, "operator": "in", "value": ["5"] }),
        Some(WORKSPACE_ID),
    );
}

// ts:117 adapted
#[test]
fn pipeline_item_contact_condition_sql_fails_closed_for_non_numeric_pipeline_id() {
    fails_closed(
        json!({ "attribute": "status", "pipelineId": "1; DROP", "operator": "in", "value": ["open"] }),
        Some(WORKSPACE_ID),
    );
}

// ts:117 adapted
#[test]
fn pipeline_item_contact_condition_sql_fails_closed_for_non_numeric_stage_id() {
    fails_closed(
        json!({ "attribute": "stage", "pipelineId": PIPELINE_ID, "operator": "in", "value": ["x"] }),
        Some(WORKSPACE_ID),
    );
}

// ts:117 adapted
#[test]
fn pipeline_item_contact_condition_sql_fails_closed_for_unknown_status() {
    fails_closed(
        json!({ "attribute": "status", "pipelineId": null, "operator": "in", "value": ["archived"] }),
        Some(WORKSPACE_ID),
    );
}

// ts:117 adapted
#[test]
fn pipeline_item_contact_condition_sql_fails_closed_for_unknown_priority() {
    fails_closed(
        json!({ "attribute": "priority", "pipelineId": null, "operator": "in", "value": [] }),
        Some(WORKSPACE_ID),
    );
}

// ts:117 adapted
#[test]
fn pipeline_item_contact_condition_sql_fails_closed_for_wrong_operator_for_status() {
    fails_closed(
        json!({ "attribute": "status", "pipelineId": null, "operator": "eq", "value": ["open"] }),
        Some(WORKSPACE_ID),
    );
}

// ts:117 adapted
#[test]
fn pipeline_item_contact_condition_sql_fails_closed_for_wrong_operator_for_exists() {
    fails_closed(
        json!({ "attribute": "exists", "pipelineId": null, "operator": "in", "value": "any" }),
        Some(WORKSPACE_ID),
    );
}

// ts:117 adapted
#[test]
fn pipeline_item_contact_condition_sql_fails_closed_for_unknown_exists_value() {
    fails_closed(
        json!({ "attribute": "exists", "pipelineId": null, "operator": "isNotEmpty", "value": "closed" }),
        Some(WORKSPACE_ID),
    );
}

// ts:117 adapted
#[test]
fn pipeline_item_contact_condition_sql_fails_closed_for_unknown_attribute() {
    fails_closed(
        json!({ "attribute": "title", "pipelineId": null, "operator": "in", "value": ["a"] }),
        Some(WORKSPACE_ID),
    );
}

// ts:117 adapted
#[test]
fn pipeline_item_contact_condition_sql_fails_closed_for_an_inherited_name_as_the_exists_operator() {
    fails_closed(
        json!({ "attribute": "exists", "pipelineId": null, "operator": "constructor", "value": "any" }),
        Some(WORKSPACE_ID),
    );
}

// ts:117 adapted
#[test]
fn pipeline_item_contact_condition_sql_fails_closed_for_an_inherited_name_as_the_membership_operator()
 {
    fails_closed(
        json!({ "attribute": "status", "pipelineId": null, "operator": "toString", "value": ["open"] }),
        Some(WORKSPACE_ID),
    );
}
