//! Port of the `listItems` / `stageTotals` filter cases of
//! `apps/builder/__tests__/pipelines-public-api.test.ts` (lines 159-354,
//! 412-427, 491-527) and the value-interval case of
//! `pipelines-activity-filter.test.ts`. Adapted: `listItemsPublicRequest` and
//! `toItemListQuery` are `handlers::pipelines::item_list_query` (path
//! params + raw query string -> the repository query, or the 422 error).
//! Repository differences: ids are `i64`, dates unix ms, money the stored
//! x 10,000 integer, and "no field conditions" is an empty map.

mod handlers_common;

use std::collections::BTreeMap;

use chatbotx_api::handlers::pipelines::item_list_query;
use chatbotx_api::orpc::OrpcError;
use chatbotx_business::pipeline::query::ItemListQuery;
use chatbotx_db::repo::pipeline_field_condition::{PipelineFieldCondition, PipelineFieldOperator};
use chatbotx_db::repo::pipeline_item_cursor::NEWEST_FIRST_SORT;
use chatbotx_db::repo::pipeline_item_filter::{
    DateRange, PipelineActivityMatch, PipelineValueFilter, PipelineValueOperator,
};
use handlers_common::{WS, call, fixture};
use serde_json::{Value, json};

/// 2026-01-01T00:00:00.000Z
const JAN_1: i64 = 1_767_225_600_000;
/// 2026-02-01T00:00:00.000Z
const FEB_1: i64 = 1_769_904_000_000;

fn parse(pairs: &[(&str, &str)]) -> Result<ItemListQuery, OrpcError> {
    let query = url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs(pairs)
        .finish();
    let params = [("id", "1".to_string())];
    item_list_query(&params, Some(&query))
}

fn parsed(pairs: &[(&str, &str)]) -> ItemListQuery {
    match parse(pairs) {
        Ok(query) => query,
        Err(error) => panic!("{pairs:?} refused: {error:?}"),
    }
}

fn refused(pairs: &[(&str, &str)]) -> bool {
    parse(pairs).is_err()
}

fn condition(operator: PipelineFieldOperator, value: Option<&str>) -> PipelineFieldCondition {
    PipelineFieldCondition {
        operator,
        value: value.map(str::to_string),
        value_to: None,
    }
}

#[test]
fn list_items_maps_every_board_filter_onto_the_repository_filter() {
    let filter = parsed(&[
        ("updatedFrom", "2026-01-01T00:00:00.000Z"),
        ("lastCommentTo", "2026-02-01T00:00:00.000Z"),
        ("statusChangedFrom", "2026-01-02T00:00:00.000Z"),
        ("stageEnteredTo", "2026-01-03T00:00:00.000Z"),
        ("wonClosedFrom", "2026-01-04T00:00:00.000Z"),
        ("lostClosedTo", "2026-01-05T00:00:00.000Z"),
        ("fieldId", "99"),
        ("fieldOperator", "is"),
        ("fieldValue", "gold"),
    ])
    .filter;

    assert_eq!(filter.updated_at.and_then(|r| r.from), Some(JAN_1));
    assert_eq!(filter.last_comment_at.and_then(|r| r.to), Some(FEB_1));
    assert!(filter.status_changed_at.and_then(|r| r.from).is_some());
    assert!(filter.stage_entered_at.and_then(|r| r.to).is_some());
    let closed_as = filter.closed_as.expect("closedAs");
    assert!(closed_as.won.and_then(|r| r.from).is_some());
    assert!(closed_as.lost.and_then(|r| r.to).is_some());
    assert_eq!(closed_as.won.and_then(|r| r.to), None);
    assert_eq!(
        filter.field_conditions,
        BTreeMap::from([(
            "99".to_string(),
            condition(PipelineFieldOperator::Is, Some("gold"))
        )])
    );
}

#[test]
fn list_items_takes_the_board_s_any_of_lists_and_several_field_conditions() {
    let conditions = json!({
        "41": { "operator": "gt", "value": "5" },
        "42": { "operator": "hasValue" },
    })
    .to_string();
    let filter = parsed(&[
        ("stageId", "11,12"),
        ("status", "won,lost"),
        ("priority", "high"),
        ("assigneeMemberId", "21,22"),
        ("assigneeInboxTeamId", "31"),
        ("fieldConditions", &conditions),
        ("fieldId", "43"),
        ("fieldOperator", "is"),
        ("fieldValue", "VIP"),
    ])
    .filter;

    assert_eq!(filter.stage_ids, vec![11, 12]);
    assert_eq!(filter.statuses, vec!["won", "lost"]);
    assert_eq!(filter.priorities, vec!["high"]);
    assert_eq!(filter.assignee_member_ids, vec![21, 22]);
    assert_eq!(filter.assignee_team_ids, vec![31]);
    let keys: Vec<&str> = filter.field_conditions.keys().map(String::as_str).collect();
    assert_eq!(keys, vec!["41", "42", "43"]);
}

#[test]
fn list_items_rejects_malformed_field_conditions_and_unknown_statuses() {
    assert!(refused(&[("fieldConditions", "{oops")]));
    assert!(refused(&[("status", "open,maybe")]));
}

#[tokio::test]
async fn field_values_timezone_must_be_a_real_iana_zone() {
    let f = fixture().await;
    f.state
        .db
        .write(|tx| {
            tx.execute(
                r#"INSERT INTO "Contact" ("id", "firstName", "workspaceId") VALUES (3001, 'Ada', ?1)"#,
                [WS],
            )?;
            Ok(())
        })
        .await
        .expect("contact");
    let (status, body) = call(
        &f,
        "pipelines.create",
        WS,
        &[],
        None,
        Some(json!({ "type": "opportunity", "name": "Sales" })),
    )
    .await;
    assert_eq!(status, 201, "{body}");
    let pipeline_id = body["id"].as_str().expect("id").to_string();
    let mut results: Vec<(u16, Value)> = Vec::new();
    for zone in ["Asia/Ho_Chi_Minh", "Mars/Olympus"] {
        results.push(
            call(
                &f,
                "pipelines.createItem",
                WS,
                &[("id", &pipeline_id)],
                None,
                Some(json!({ "contactId": "3001", "title": "x", "fieldValuesTimezone": zone })),
            )
            .await,
        );
    }

    assert_eq!(results[0].0, 201, "{}", results[0].1);
    assert_eq!(results[1].0, 422, "{}", results[1].1);
    assert_eq!(
        results[1].1["code"], "invalidRequestData",
        "{}",
        results[1].1
    );
}

#[test]
fn list_items_without_the_new_params_adds_no_extra_filters() {
    let filter = parsed(&[]).filter;

    assert_eq!(filter.updated_at, None);
    assert_eq!(filter.closed_as, None);
    assert!(filter.field_conditions.is_empty());
}

#[test]
fn list_items_maps_value_and_field_operators_onto_the_repository_filter() {
    let filter = parsed(&[
        ("valueOperator", "gt"),
        ("valueAmount", "100"),
        ("fieldId", "99"),
        ("fieldOperator", "contains"),
        ("fieldValue", "gold"),
    ])
    .filter;

    // 100 major units = 1,000,000 stored.
    assert_eq!(
        filter.value,
        Some(PipelineValueFilter {
            operator: Some(PipelineValueOperator::Gt),
            amount: Some(1_000_000),
            min: None,
            max: None,
        })
    );
    assert_eq!(
        filter.field_conditions,
        BTreeMap::from([(
            "99".to_string(),
            condition(PipelineFieldOperator::Contains, Some("gold"))
        )])
    );
}

#[test]
fn list_items_maps_an_interval_and_the_timezone_onto_the_repository_filter() {
    let filter = parsed(&[
        ("fieldId", "5"),
        ("fieldOperator", "between"),
        ("fieldValue", "2026-01-01"),
        ("fieldValueTo", "2026-01-31"),
        ("timezone", "Asia/Ho_Chi_Minh"),
    ])
    .filter;

    assert_eq!(
        filter.field_conditions,
        BTreeMap::from([(
            "5".to_string(),
            PipelineFieldCondition::between(Some("2026-01-01"), Some("2026-01-31"))
        )])
    );
    assert_eq!(filter.timezone.as_deref(), Some("Asia/Ho_Chi_Minh"));
}

#[test]
fn list_items_rejects_an_unknown_timezone_and_the_removed_field_params() {
    assert!(refused(&[("timezone", "Mars/Olympus")]));
    // `fieldFrom` is stripped (zod drops unknown keys): it adds no condition.
    let filter = parsed(&[("fieldId", "5"), ("fieldFrom", "2026-01-01T00:00:00Z")]).filter;
    assert!(filter.field_conditions.is_empty());
}

#[test]
fn list_items_rejects_a_field_operator_without_a_field() {
    assert!(refused(&[("fieldOperator", "hasValue")]));
}

#[test]
fn list_items_keeps_the_plain_value_interval_without_an_operator() {
    let value = parsed(&[("valueMin", "5")]).filter.value.expect("value");

    assert_eq!(value.operator, None);
    assert_eq!(value.min, Some(50_000));
}

#[test]
fn list_items_has_value_needs_no_field_value() {
    assert!(parse(&[("fieldId", "99"), ("fieldOperator", "hasValue")]).is_ok());
}

#[test]
fn list_items_rejects_an_operator_without_its_operand() {
    let cases: [&[(&str, &str)]; 4] = [
        &[("valueOperator", "eq")],
        &[("fieldOperator", "is"), ("fieldValue", "x")],
        &[("fieldId", "99"), ("fieldOperator", "contains")],
        &[
            ("fieldId", "99"),
            ("fieldOperator", "gt"),
            ("fieldValue", ""),
        ],
    ];
    for patch in cases {
        assert!(refused(patch), "{patch:?}");
    }
}

#[test]
fn list_items_defaults_to_newest_first_with_a_bounded_page() {
    let query = parsed(&[]);

    assert_eq!(query.sort, NEWEST_FIRST_SORT);
    assert_eq!(query.limit, 50);
    assert!(refused(&[("limit", "101")]));
}

#[test]
fn list_items_rejects_out_of_range_ids_dates_and_nul() {
    let long_id = "9".repeat(25);
    let cases: [(&str, &[(&str, &str)]); 5] = [
        (
            "an assignee id past a signed bigint",
            &[("assigneeMemberId", &long_id)],
        ),
        ("a non-numeric team id", &[("assigneeInboxTeamId", "abc")]),
        ("a date before 1900", &[("dueFrom", "0001-01-01T00:00:00Z")]),
        (
            "a date after 2200",
            &[("createdTo", "3000-01-01T00:00:00Z")],
        ),
        ("a NUL in the search", &[("search", "a\u{0}b")]),
    ];
    for (name, patch) in cases {
        assert!(refused(patch), "{name}");
    }
}

#[test]
fn list_items_accepts_in_range_ids_dates_and_search() {
    assert!(
        parse(&[
            ("assigneeMemberId", "42"),
            ("dueFrom", "2026-01-01T00:00:00Z"),
            ("search", "acme"),
        ])
        .is_ok()
    );
}

#[test]
fn list_items_maps_the_activity_params_onto_one_activity_condition() {
    let filter = parsed(&[
        ("activityGroups", "stage, comments"),
        ("activityActor", "7,automation"),
        ("activityFrom", "2026-01-01T00:00:00.000Z"),
    ])
    .filter;

    assert_eq!(
        filter.activity_match,
        Some(PipelineActivityMatch {
            kinds: vec![
                "stageChanged".to_string(),
                "commentAdded".to_string(),
                "commentDeleted".to_string(),
            ],
            actor_user_ids: vec![7],
            automation: true,
            occurred_at: Some(DateRange {
                from: Some(JAN_1),
                to: None,
            }),
        })
    );
}

#[test]
fn list_items_without_activity_params_sets_no_activity_condition() {
    assert_eq!(parsed(&[]).filter.activity_match, None);
}

#[test]
fn list_items_rejects_an_unknown_activity_group_or_actor() {
    assert!(refused(&[("activityGroups", "nope")]));
    assert!(refused(&[("activityActor", "bob")]));
}

#[test]
fn list_items_value_between_needs_at_least_one_bound() {
    assert!(refused(&[("valueOperator", "between")]));
    assert!(parse(&[("valueOperator", "between"), ("valueMin", "5")]).is_ok());
}
