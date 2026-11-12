//! Port of `packages/database/__tests__/integration/pipeline-field-filter.test.ts` (P3.3).
//! The TS runs against Postgres; each port runs against a migrated SQLite file (one fresh
//! file per case instead of one workspace per `describe`).

mod common;
#[macro_use]
#[path = "common/repo.rs"]
mod repo_common;
#[path = "common/repo7.rs"]
mod repo7;

use std::collections::BTreeMap;

use chatbotx_db::Db;
use chatbotx_db::repo::pipeline_field_condition::{
    CustomFieldType, PipelineFieldCondition, PipelineFieldOperator,
};
use chatbotx_db::repo::pipeline_item::{self, ListStagePageInput};
use chatbotx_db::repo::pipeline_item_cursor::{PipelineItemSort, PipelineItemSortKey};
use chatbotx_db::repo::pipeline_item_filter::PipelineItemFilter;
use chatbotx_db::repo::utils::SortDir;
use chatbotx_db::repo::{self, Json};
use repo_common::{ok, setup};
use repo7::*;

const FIELD_ID: &str = "50";
const VN: &str = "Asia/Ho_Chi_Minh";
const LA: &str = "America/Los_Angeles";

/// A seed: the field type and `(title, stored value)` (`None` = key missing).
type Seed = (
    CustomFieldType,
    &'static [(&'static str, Option<&'static str>)],
);

const TEXT: Seed = (
    CustomFieldType::ShortText,
    &[
        ("Gold", Some("Gold Plan")),
        ("Silver", Some("silver")),
        ("Percent", Some("100%_off")),
        ("Empty", Some("")),
        ("Missing", None),
    ],
);
const NUMBER: Seed = (
    CustomFieldType::Number,
    &[
        ("Ten", Some("10")),
        ("Twenty", Some("20.5")),
        ("Negative", Some("-3")),
        ("Malformed", Some("12abc")),
        ("Exponent", Some("1e3")),
        ("Empty", Some("")),
        ("Missing", None),
    ],
);
const DATETIME: Seed = (
    CustomFieldType::Datetime,
    &[
        // 2026-05-19 17:00Z: the 20th in Ho Chi Minh, the 19th in UTC and Los Angeles.
        ("MorningVn", Some("2026-05-20T00:00:00+07:00")),
        // 2026-05-20 16:30Z: the 20th in UTC, Los Angeles and Ho Chi Minh.
        ("EveningVn", Some("2026-05-20T23:30:00+07:00")),
        // 2026-05-21 00:00Z: the 21st in UTC and Ho Chi Minh, the 20th in Los Angeles.
        ("NextUtc", Some("2026-05-21T00:00:00Z")),
        ("NotADate", Some("not a date")),
        ("ImpossibleDay", Some("2026-02-30T00:00:00Z")),
        ("ImpossibleMonth", Some("2026-13-01T00:00:00Z")),
        ("Empty", Some("")),
        ("Missing", None),
    ],
);
const DATE: Seed = (
    CustomFieldType::Date,
    &[
        // Picked "20 May" in Ho Chi Minh: shown as the 20th everywhere.
        ("PickedVn", Some("2026-05-20T00:00:00+07:00")),
        ("PickedUtc", Some("2026-05-21T00:00:00Z")),
        ("NotADate", Some("not a date")),
        ("Missing", None),
    ],
);
const EXPONENT: Seed = (
    CustomFieldType::Number,
    &[
        ("Tiny", Some("1e-7")),
        ("Huge", Some("1e+21")),
        ("Zero", Some("0")),
    ],
);
const BOOLEAN: Seed = (
    CustomFieldType::Boolean,
    &[
        ("Yes", Some("true")),
        ("No", Some("false")),
        ("Maybe", Some("maybe")),
        ("Empty", Some("")),
        ("Missing", None),
    ],
);

/// A fresh file with fixture 1 and one item per seed value (the TS `seed`).
async fn seed(seed: Seed) -> (common::TestDb, CustomFieldType) {
    let db = common::fresh_db().await;
    let fx = PipeFx::ids(1);
    setup(&db, move |tx| put_fixture(tx, fx)).await;
    for (title, value) in seed.1 {
        let fields = match value {
            Some(v) => serde_json::json!({ FIELD_ID: v }),
            None => serde_json::json!({}),
        };
        insert_item(&db, fx, move |v| {
            v.title = (*title).to_owned();
            v.field_values = Json(fields);
        })
        .await;
    }
    (db, seed.0)
}

fn cond(op: &str, value: Option<&str>, value_to: Option<&str>) -> PipelineFieldCondition {
    use PipelineFieldOperator as O;
    let operator = match op {
        "is" => O::Is,
        "isNot" => O::IsNot,
        "hasValue" => O::HasValue,
        "noValue" => O::NoValue,
        "contains" => O::Contains,
        "notContains" => O::NotContains,
        "startsWith" => O::StartsWith,
        "endsWith" => O::EndsWith,
        "gt" => O::Gt,
        "lt" => O::Lt,
        "between" => O::Between,
        other => panic!("operator {other}"),
    };
    PipelineFieldCondition {
        operator,
        value: value.map(str::to_owned),
        value_to: value_to.map(str::to_owned),
    }
}

fn names(n: &[&str]) -> Vec<String> {
    n.iter().map(|s| (*s).to_owned()).collect()
}

/// The TS `matcher(fixture, fieldType)(condition, timezone)`.
async fn run(
    seeded: &(common::TestDb, CustomFieldType),
    condition: PipelineFieldCondition,
    timezone: Option<&str>,
) -> Vec<String> {
    let fx = PipeFx::ids(1);
    let input = ListStagePageInput {
        workspace_id: fx.workspace_id,
        pipeline_id: fx.pipeline_id,
        stage_id: Some(fx.stage_ids[0]),
        filter: PipelineItemFilter {
            field_conditions: BTreeMap::from([(FIELD_ID.to_owned(), condition)]),
            field_types: BTreeMap::from([(FIELD_ID.to_owned(), seeded.1)]),
            timezone: timezone.map(str::to_owned),
            ..Default::default()
        },
        sort: PipelineItemSort {
            key: PipelineItemSortKey::CreatedAt,
            direction: SortDir::Asc,
        },
        limit: 50,
        cursor: None,
    };
    let db: &Db = &seeded.0;
    titles(&ok(repo::read(db, move |c| pipeline_item::list_stage_page(c, &input)).await).items)
}

/// Port of: packages/database/__tests__/integration/pipeline-field-filter.test.ts:170 "pipeline custom-field filter SQL > text > %j matches %j ({"operator":"is","value":"silver"} -> ["Silver"])"
#[tokio::test]
async fn pipeline_custom_field_filter_sql_text_case_01_is_silver() {
    let db = seed(TEXT).await;
    assert_eq!(
        run(&db, cond("is", Some("silver"), None), None).await,
        names(&["Silver"])
    );
}

/// Port of: packages/database/__tests__/integration/pipeline-field-filter.test.ts:170 "pipeline custom-field filter SQL > text > %j matches %j ({"operator":"is","value":"SILVER"} -> [])"
#[tokio::test]
async fn pipeline_custom_field_filter_sql_text_case_02_is_silver() {
    let db = seed(TEXT).await;
    assert_eq!(
        run(&db, cond("is", Some("SILVER"), None), None).await,
        names(&[])
    );
}

/// Port of: packages/database/__tests__/integration/pipeline-field-filter.test.ts:170 "pipeline custom-field filter SQL > text > %j matches %j ({"operator":"isNot","value":"silver"} -> ["Gold","Percent"])"
#[tokio::test]
async fn pipeline_custom_field_filter_sql_text_case_03_is_not_silver() {
    let db = seed(TEXT).await;
    assert_eq!(
        run(&db, cond("isNot", Some("silver"), None), None).await,
        names(&["Gold", "Percent"])
    );
}

/// Port of: packages/database/__tests__/integration/pipeline-field-filter.test.ts:170 "pipeline custom-field filter SQL > text > %j matches %j ({"operator":"contains","value":"GOLD"} -> ["Gold"])"
#[tokio::test]
async fn pipeline_custom_field_filter_sql_text_case_04_contains_gold() {
    let db = seed(TEXT).await;
    assert_eq!(
        run(&db, cond("contains", Some("GOLD"), None), None).await,
        names(&["Gold"])
    );
}

/// Port of: packages/database/__tests__/integration/pipeline-field-filter.test.ts:170 "pipeline custom-field filter SQL > text > %j matches %j ({"operator":"contains","value":"%"} -> ["Percent"])"
#[tokio::test]
async fn pipeline_custom_field_filter_sql_text_case_05_contains() {
    let db = seed(TEXT).await;
    assert_eq!(
        run(&db, cond("contains", Some("%"), None), None).await,
        names(&["Percent"])
    );
}

/// Port of: packages/database/__tests__/integration/pipeline-field-filter.test.ts:170 "pipeline custom-field filter SQL > text > %j matches %j ({"operator":"contains","value":"_"} -> ["Percent"])"
#[tokio::test]
async fn pipeline_custom_field_filter_sql_text_case_06_contains() {
    let db = seed(TEXT).await;
    assert_eq!(
        run(&db, cond("contains", Some("_"), None), None).await,
        names(&["Percent"])
    );
}

/// Port of: packages/database/__tests__/integration/pipeline-field-filter.test.ts:170 "pipeline custom-field filter SQL > text > %j matches %j ({"operator":"contains","value":"zzz"} -> [])"
#[tokio::test]
async fn pipeline_custom_field_filter_sql_text_case_07_contains_zzz() {
    let db = seed(TEXT).await;
    assert_eq!(
        run(&db, cond("contains", Some("zzz"), None), None).await,
        names(&[])
    );
}

/// Port of: packages/database/__tests__/integration/pipeline-field-filter.test.ts:170 "pipeline custom-field filter SQL > text > %j matches %j ({"operator":"notContains","value":"gold"} -> ["Silver","Percent"])"
#[tokio::test]
async fn pipeline_custom_field_filter_sql_text_case_08_not_contains_gold() {
    let db = seed(TEXT).await;
    assert_eq!(
        run(&db, cond("notContains", Some("gold"), None), None).await,
        names(&["Silver", "Percent"])
    );
}

/// Port of: packages/database/__tests__/integration/pipeline-field-filter.test.ts:170 "pipeline custom-field filter SQL > text > %j matches %j ({"operator":"startsWith","value":"SIL"} -> ["Silver"])"
#[tokio::test]
async fn pipeline_custom_field_filter_sql_text_case_09_starts_with_sil() {
    let db = seed(TEXT).await;
    assert_eq!(
        run(&db, cond("startsWith", Some("SIL"), None), None).await,
        names(&["Silver"])
    );
}

/// Port of: packages/database/__tests__/integration/pipeline-field-filter.test.ts:170 "pipeline custom-field filter SQL > text > %j matches %j ({"operator":"startsWith","value":"plan"} -> [])"
#[tokio::test]
async fn pipeline_custom_field_filter_sql_text_case_10_starts_with_plan() {
    let db = seed(TEXT).await;
    assert_eq!(
        run(&db, cond("startsWith", Some("plan"), None), None).await,
        names(&[])
    );
}

/// Port of: packages/database/__tests__/integration/pipeline-field-filter.test.ts:170 "pipeline custom-field filter SQL > text > %j matches %j ({"operator":"endsWith","value":"plan"} -> ["Gold"])"
#[tokio::test]
async fn pipeline_custom_field_filter_sql_text_case_11_ends_with_plan() {
    let db = seed(TEXT).await;
    assert_eq!(
        run(&db, cond("endsWith", Some("plan"), None), None).await,
        names(&["Gold"])
    );
}

/// Port of: packages/database/__tests__/integration/pipeline-field-filter.test.ts:170 "pipeline custom-field filter SQL > text > %j matches %j ({"operator":"endsWith","value":"gold"} -> [])"
#[tokio::test]
async fn pipeline_custom_field_filter_sql_text_case_12_ends_with_gold() {
    let db = seed(TEXT).await;
    assert_eq!(
        run(&db, cond("endsWith", Some("gold"), None), None).await,
        names(&[])
    );
}

/// Port of: packages/database/__tests__/integration/pipeline-field-filter.test.ts:170 "pipeline custom-field filter SQL > text > %j matches %j ({"operator":"hasValue"} -> ["Gold","Silver","Percent"])"
#[tokio::test]
async fn pipeline_custom_field_filter_sql_text_case_13_has_value() {
    let db = seed(TEXT).await;
    assert_eq!(
        run(&db, cond("hasValue", None, None), None).await,
        names(&["Gold", "Silver", "Percent"])
    );
}

/// Port of: packages/database/__tests__/integration/pipeline-field-filter.test.ts:170 "pipeline custom-field filter SQL > text > %j matches %j ({"operator":"noValue"} -> ["Empty","Missing"])"
#[tokio::test]
async fn pipeline_custom_field_filter_sql_text_case_14_no_value() {
    let db = seed(TEXT).await;
    assert_eq!(
        run(&db, cond("noValue", None, None), None).await,
        names(&["Empty", "Missing"])
    );
}

/// Port of: packages/database/__tests__/integration/pipeline-field-filter.test.ts:208 "pipeline custom-field filter SQL > number > %j matches %j ({"operator":"is","value":"10"} -> ["Ten"])"
#[tokio::test]
async fn pipeline_custom_field_filter_sql_number_case_01_is_10() {
    let db = seed(NUMBER).await;
    assert_eq!(
        run(&db, cond("is", Some("10"), None), None).await,
        names(&["Ten"])
    );
}

/// Port of: packages/database/__tests__/integration/pipeline-field-filter.test.ts:208 "pipeline custom-field filter SQL > number > %j matches %j ({"operator":"is","value":"10.0"} -> ["Ten"])"
#[tokio::test]
async fn pipeline_custom_field_filter_sql_number_case_02_is_10_0() {
    let db = seed(NUMBER).await;
    assert_eq!(
        run(&db, cond("is", Some("10.0"), None), None).await,
        names(&["Ten"])
    );
}

/// Port of: packages/database/__tests__/integration/pipeline-field-filter.test.ts:208 "pipeline custom-field filter SQL > number > %j matches %j ({"operator":"is","value":"99"} -> [])"
#[tokio::test]
async fn pipeline_custom_field_filter_sql_number_case_03_is_99() {
    let db = seed(NUMBER).await;
    assert_eq!(
        run(&db, cond("is", Some("99"), None), None).await,
        names(&[])
    );
}

/// Port of: packages/database/__tests__/integration/pipeline-field-filter.test.ts:208 "pipeline custom-field filter SQL > number > %j matches %j ({"operator":"isNot","value":"10"} -> ["Twenty","Negative"])"
#[tokio::test]
async fn pipeline_custom_field_filter_sql_number_case_04_is_not_10() {
    let db = seed(NUMBER).await;
    assert_eq!(
        run(&db, cond("isNot", Some("10"), None), None).await,
        names(&["Twenty", "Negative"])
    );
}

/// Port of: packages/database/__tests__/integration/pipeline-field-filter.test.ts:208 "pipeline custom-field filter SQL > number > %j matches %j ({"operator":"gt","value":"10"} -> ["Twenty"])"
#[tokio::test]
async fn pipeline_custom_field_filter_sql_number_case_05_gt_10() {
    let db = seed(NUMBER).await;
    assert_eq!(
        run(&db, cond("gt", Some("10"), None), None).await,
        names(&["Twenty"])
    );
}

/// Port of: packages/database/__tests__/integration/pipeline-field-filter.test.ts:208 "pipeline custom-field filter SQL > number > %j matches %j ({"operator":"gt","value":"-3"} -> ["Ten","Twenty"])"
#[tokio::test]
async fn pipeline_custom_field_filter_sql_number_case_06_gt_3() {
    let db = seed(NUMBER).await;
    assert_eq!(
        run(&db, cond("gt", Some("-3"), None), None).await,
        names(&["Ten", "Twenty"])
    );
}

/// Port of: packages/database/__tests__/integration/pipeline-field-filter.test.ts:208 "pipeline custom-field filter SQL > number > %j matches %j ({"operator":"lt","value":"20.5"} -> ["Ten","Negative"])"
#[tokio::test]
async fn pipeline_custom_field_filter_sql_number_case_07_lt_20_5() {
    let db = seed(NUMBER).await;
    assert_eq!(
        run(&db, cond("lt", Some("20.5"), None), None).await,
        names(&["Ten", "Negative"])
    );
}

/// Port of: packages/database/__tests__/integration/pipeline-field-filter.test.ts:208 "pipeline custom-field filter SQL > number > %j matches %j ({"operator":"lt","value":"-3"} -> [])"
#[tokio::test]
async fn pipeline_custom_field_filter_sql_number_case_08_lt_3() {
    let db = seed(NUMBER).await;
    assert_eq!(
        run(&db, cond("lt", Some("-3"), None), None).await,
        names(&[])
    );
}

/// Port of: packages/database/__tests__/integration/pipeline-field-filter.test.ts:208 "pipeline custom-field filter SQL > number > %j matches %j ({"operator":"between","value":"10","valueTo":"20.5"} -> ["Ten","Twenty"])"
#[tokio::test]
async fn pipeline_custom_field_filter_sql_number_case_09_between_10_20_5() {
    let db = seed(NUMBER).await;
    assert_eq!(
        run(&db, cond("between", Some("10"), Some("20.5")), None).await,
        names(&["Ten", "Twenty"])
    );
}

/// Port of: packages/database/__tests__/integration/pipeline-field-filter.test.ts:208 "pipeline custom-field filter SQL > number > %j matches %j ({"operator":"between","value":"10"} -> ["Ten","Twenty"])"
#[tokio::test]
async fn pipeline_custom_field_filter_sql_number_case_10_between_10() {
    let db = seed(NUMBER).await;
    assert_eq!(
        run(&db, cond("between", Some("10"), None), None).await,
        names(&["Ten", "Twenty"])
    );
}

/// Port of: packages/database/__tests__/integration/pipeline-field-filter.test.ts:208 "pipeline custom-field filter SQL > number > %j matches %j ({"operator":"between","valueTo":"10"} -> ["Ten","Negative"])"
#[tokio::test]
async fn pipeline_custom_field_filter_sql_number_case_11_between_10() {
    let db = seed(NUMBER).await;
    assert_eq!(
        run(&db, cond("between", None, Some("10")), None).await,
        names(&["Ten", "Negative"])
    );
}

/// Port of: packages/database/__tests__/integration/pipeline-field-filter.test.ts:208 "pipeline custom-field filter SQL > number > %j matches %j ({"operator":"between","value":"11","valueTo":"12"} -> [])"
#[tokio::test]
async fn pipeline_custom_field_filter_sql_number_case_12_between_11_12() {
    let db = seed(NUMBER).await;
    assert_eq!(
        run(&db, cond("between", Some("11"), Some("12")), None).await,
        names(&[])
    );
}

/// Port of: packages/database/__tests__/integration/pipeline-field-filter.test.ts:208 "pipeline custom-field filter SQL > number > %j matches %j ({"operator":"gt","value":"abc"} -> ["Ten","Twenty","Negative","Malformed","Exponent","Empty","Missing"])"
#[tokio::test]
async fn pipeline_custom_field_filter_sql_number_case_13_gt_abc() {
    let db = seed(NUMBER).await;
    assert_eq!(
        run(&db, cond("gt", Some("abc"), None), None).await,
        names(&[
            "Ten",
            "Twenty",
            "Negative",
            "Malformed",
            "Exponent",
            "Empty",
            "Missing"
        ])
    );
}

/// Port of: packages/database/__tests__/integration/pipeline-field-filter.test.ts:208 "pipeline custom-field filter SQL > number > %j matches %j ({"operator":"hasValue"} -> ["Ten","Twenty","Negative","Malformed","Exponent"])"
#[tokio::test]
async fn pipeline_custom_field_filter_sql_number_case_14_has_value() {
    let db = seed(NUMBER).await;
    assert_eq!(
        run(&db, cond("hasValue", None, None), None).await,
        names(&["Ten", "Twenty", "Negative", "Malformed", "Exponent"])
    );
}

/// Port of: packages/database/__tests__/integration/pipeline-field-filter.test.ts:208 "pipeline custom-field filter SQL > number > %j matches %j ({"operator":"noValue"} -> ["Empty","Missing"])"
#[tokio::test]
async fn pipeline_custom_field_filter_sql_number_case_15_no_value() {
    let db = seed(NUMBER).await;
    assert_eq!(
        run(&db, cond("noValue", None, None), None).await,
        names(&["Empty", "Missing"])
    );
}

/// Port of: packages/database/__tests__/integration/pipeline-field-filter.test.ts:272 "pipeline custom-field filter SQL > datetime > is compares the calendar day in the given timezone"
#[tokio::test]
async fn pipeline_custom_field_filter_sql_datetime_is_compares_the_calendar_day_in_the_given_timezone()
 {
    let db = seed(DATETIME).await;
    let is = || cond("is", Some("2026-05-20"), None);
    assert_eq!(
        run(&db, is(), Some(VN)).await,
        names(&["MorningVn", "EveningVn"])
    );
    assert_eq!(run(&db, is(), Some("UTC")).await, names(&["EveningVn"]));
    assert_eq!(
        run(&db, is(), Some(LA)).await,
        names(&["EveningVn", "NextUtc"])
    );
}

/// Port of: packages/database/__tests__/integration/pipeline-field-filter.test.ts:279 "pipeline custom-field filter SQL > datetime > a missing or unknown timezone falls back to UTC"
#[tokio::test]
async fn pipeline_custom_field_filter_sql_datetime_a_missing_or_unknown_timezone_falls_back_to_utc()
{
    let db = seed(DATETIME).await;
    let is = || cond("is", Some("2026-05-20"), None);
    assert_eq!(run(&db, is(), None).await, names(&["EveningVn"]));
    assert_eq!(
        run(&db, is(), Some("Mars/Olympus")).await,
        names(&["EveningVn"])
    );
}

/// Port of: packages/database/__tests__/integration/pipeline-field-filter.test.ts:285 "pipeline custom-field filter SQL > datetime > %j matches %j in Ho Chi Minh ({"operator":"isNot","value":"2026-05-20"} -> ["NextUtc"])"
#[tokio::test]
async fn pipeline_custom_field_filter_sql_datetime_case_01_is_not_2026_05_20() {
    let db = seed(DATETIME).await;
    assert_eq!(
        run(&db, cond("isNot", Some("2026-05-20"), None), Some(VN)).await,
        names(&["NextUtc"])
    );
}

/// Port of: packages/database/__tests__/integration/pipeline-field-filter.test.ts:285 "pipeline custom-field filter SQL > datetime > %j matches %j in Ho Chi Minh ({"operator":"gt","value":"2026-05-20"} -> ["NextUtc"])"
#[tokio::test]
async fn pipeline_custom_field_filter_sql_datetime_case_02_gt_2026_05_20() {
    let db = seed(DATETIME).await;
    assert_eq!(
        run(&db, cond("gt", Some("2026-05-20"), None), Some(VN)).await,
        names(&["NextUtc"])
    );
}

/// Port of: packages/database/__tests__/integration/pipeline-field-filter.test.ts:285 "pipeline custom-field filter SQL > datetime > %j matches %j in Ho Chi Minh ({"operator":"gt","value":"2026-05-21"} -> [])"
#[tokio::test]
async fn pipeline_custom_field_filter_sql_datetime_case_03_gt_2026_05_21() {
    let db = seed(DATETIME).await;
    assert_eq!(
        run(&db, cond("gt", Some("2026-05-21"), None), Some(VN)).await,
        names(&[])
    );
}

/// Port of: packages/database/__tests__/integration/pipeline-field-filter.test.ts:285 "pipeline custom-field filter SQL > datetime > %j matches %j in Ho Chi Minh ({"operator":"lt","value":"2026-05-21"} -> ["MorningVn","EveningVn"])"
#[tokio::test]
async fn pipeline_custom_field_filter_sql_datetime_case_04_lt_2026_05_21() {
    let db = seed(DATETIME).await;
    assert_eq!(
        run(&db, cond("lt", Some("2026-05-21"), None), Some(VN)).await,
        names(&["MorningVn", "EveningVn"])
    );
}

/// Port of: packages/database/__tests__/integration/pipeline-field-filter.test.ts:285 "pipeline custom-field filter SQL > datetime > %j matches %j in Ho Chi Minh ({"operator":"lt","value":"2026-05-20"} -> [])"
#[tokio::test]
async fn pipeline_custom_field_filter_sql_datetime_case_05_lt_2026_05_20() {
    let db = seed(DATETIME).await;
    assert_eq!(
        run(&db, cond("lt", Some("2026-05-20"), None), Some(VN)).await,
        names(&[])
    );
}

/// Port of: packages/database/__tests__/integration/pipeline-field-filter.test.ts:285 "pipeline custom-field filter SQL > datetime > %j matches %j in Ho Chi Minh ({"operator":"between","value":"2026-05-20","valueTo":"2026-05-20"} -> ["MorningVn","EveningVn"])"
#[tokio::test]
async fn pipeline_custom_field_filter_sql_datetime_case_06_between_2026_05_20_2026_05_20() {
    let db = seed(DATETIME).await;
    assert_eq!(
        run(
            &db,
            cond("between", Some("2026-05-20"), Some("2026-05-20")),
            Some(VN)
        )
        .await,
        names(&["MorningVn", "EveningVn"])
    );
}

/// Port of: packages/database/__tests__/integration/pipeline-field-filter.test.ts:285 "pipeline custom-field filter SQL > datetime > %j matches %j in Ho Chi Minh ({"operator":"between","value":"2026-05-20","valueTo":"2026-05-21"} -> ["MorningVn","EveningVn","NextUtc"])"
#[tokio::test]
async fn pipeline_custom_field_filter_sql_datetime_case_07_between_2026_05_20_2026_05_21() {
    let db = seed(DATETIME).await;
    assert_eq!(
        run(
            &db,
            cond("between", Some("2026-05-20"), Some("2026-05-21")),
            Some(VN)
        )
        .await,
        names(&["MorningVn", "EveningVn", "NextUtc"])
    );
}

/// Port of: packages/database/__tests__/integration/pipeline-field-filter.test.ts:285 "pipeline custom-field filter SQL > datetime > %j matches %j in Ho Chi Minh ({"operator":"between","value":"2026-05-21"} -> ["NextUtc"])"
#[tokio::test]
async fn pipeline_custom_field_filter_sql_datetime_case_08_between_2026_05_21() {
    let db = seed(DATETIME).await;
    assert_eq!(
        run(&db, cond("between", Some("2026-05-21"), None), Some(VN)).await,
        names(&["NextUtc"])
    );
}

/// Port of: packages/database/__tests__/integration/pipeline-field-filter.test.ts:285 "pipeline custom-field filter SQL > datetime > %j matches %j in Ho Chi Minh ({"operator":"between","valueTo":"2026-05-19"} -> [])"
#[tokio::test]
async fn pipeline_custom_field_filter_sql_datetime_case_09_between_2026_05_19() {
    let db = seed(DATETIME).await;
    assert_eq!(
        run(&db, cond("between", None, Some("2026-05-19")), Some(VN)).await,
        names(&[])
    );
}

/// Port of: packages/database/__tests__/integration/pipeline-field-filter.test.ts:308 "pipeline custom-field filter SQL > datetime > malformed stored values never throw and only count as present"
#[tokio::test]
async fn pipeline_custom_field_filter_sql_datetime_malformed_stored_values_never_throw_and_only_count_as_present()
 {
    let db = seed(DATETIME).await;
    assert_eq!(
        run(&db, cond("gt", Some("2000-01-01"), None), Some(VN)).await,
        names(&["MorningVn", "EveningVn", "NextUtc"])
    );
    assert_eq!(
        run(&db, cond("hasValue", None, None), None).await,
        names(&[
            "MorningVn",
            "EveningVn",
            "NextUtc",
            "NotADate",
            "ImpossibleDay",
            "ImpossibleMonth"
        ])
    );
    assert_eq!(
        run(&db, cond("noValue", None, None), None).await,
        names(&["Empty", "Missing"])
    );
}

/// Port of: packages/database/__tests__/integration/pipeline-field-filter.test.ts:325 "pipeline custom-field filter SQL > datetime > an impossible operand day adds no condition instead of throwing"
#[tokio::test]
async fn pipeline_custom_field_filter_sql_datetime_an_impossible_operand_day_adds_no_condition_instead_of_throwing()
 {
    let db = seed(DATETIME).await;
    assert_eq!(
        run(&db, cond("is", Some("2026-02-30"), None), Some(VN))
            .await
            .len(),
        8
    );
}

/// Port of: packages/database/__tests__/integration/pipeline-field-filter.test.ts:345 "pipeline custom-field filter SQL > date (a picked day, compared as written) > is matches the stored day whatever the filter timezone"
#[tokio::test]
async fn pipeline_custom_field_filter_sql_date_a_picked_day_compared_as_written_is_matches_the_stored_day_whatever_the_filter_timezone()
 {
    let db = seed(DATE).await;
    for zone in ["UTC", "America/Los_Angeles", "Asia/Ho_Chi_Minh"] {
        assert_eq!(
            run(&db, cond("is", Some("2026-05-20"), None), Some(zone)).await,
            names(&["PickedVn"])
        );
    }
}

/// Port of: packages/database/__tests__/integration/pipeline-field-filter.test.ts:352 "pipeline custom-field filter SQL > date (a picked day, compared as written) > %j matches %j ({"operator":"isNot","value":"2026-05-20"} -> ["PickedUtc"])"
#[tokio::test]
async fn pipeline_custom_field_filter_sql_date_a_picked_day_compared_as_written_case_01_is_not_2026_05_20()
 {
    let db = seed(DATE).await;
    assert_eq!(
        run(&db, cond("isNot", Some("2026-05-20"), None), Some(LA)).await,
        names(&["PickedUtc"])
    );
}

/// Port of: packages/database/__tests__/integration/pipeline-field-filter.test.ts:352 "pipeline custom-field filter SQL > date (a picked day, compared as written) > %j matches %j ({"operator":"gt","value":"2026-05-20"} -> ["PickedUtc"])"
#[tokio::test]
async fn pipeline_custom_field_filter_sql_date_a_picked_day_compared_as_written_case_02_gt_2026_05_20()
 {
    let db = seed(DATE).await;
    assert_eq!(
        run(&db, cond("gt", Some("2026-05-20"), None), Some(LA)).await,
        names(&["PickedUtc"])
    );
}

/// Port of: packages/database/__tests__/integration/pipeline-field-filter.test.ts:352 "pipeline custom-field filter SQL > date (a picked day, compared as written) > %j matches %j ({"operator":"lt","value":"2026-05-21"} -> ["PickedVn"])"
#[tokio::test]
async fn pipeline_custom_field_filter_sql_date_a_picked_day_compared_as_written_case_03_lt_2026_05_21()
 {
    let db = seed(DATE).await;
    assert_eq!(
        run(&db, cond("lt", Some("2026-05-21"), None), Some(LA)).await,
        names(&["PickedVn"])
    );
}

/// Port of: packages/database/__tests__/integration/pipeline-field-filter.test.ts:352 "pipeline custom-field filter SQL > date (a picked day, compared as written) > %j matches %j ({"operator":"between","value":"2026-05-20","valueTo":"2026-05-21"} -> ["PickedVn","PickedUtc"])"
#[tokio::test]
async fn pipeline_custom_field_filter_sql_date_a_picked_day_compared_as_written_case_04_between_2026_05_20_2026_05_21()
 {
    let db = seed(DATE).await;
    assert_eq!(
        run(
            &db,
            cond("between", Some("2026-05-20"), Some("2026-05-21")),
            Some(LA)
        )
        .await,
        names(&["PickedVn", "PickedUtc"])
    );
}

/// Port of: packages/database/__tests__/integration/pipeline-field-filter.test.ts:369 "pipeline custom-field filter SQL > number in exponent form > values the canonicalizer writes as 1e-7 / 1e+21 still compare"
#[tokio::test]
async fn pipeline_custom_field_filter_sql_number_in_exponent_form_values_the_canonicalizer_writes_as_1e_7_1e_21_still_compare()
 {
    let db = seed(EXPONENT).await;
    assert_eq!(
        run(&db, cond("is", Some("0.0000001"), None), None).await,
        names(&["Tiny"])
    );
    assert_eq!(
        run(&db, cond("gt", Some("0"), None), None).await,
        names(&["Tiny", "Huge"])
    );
    assert_eq!(
        run(&db, cond("lt", Some("1"), None), None).await,
        names(&["Tiny", "Zero"])
    );
}

/// Port of: packages/database/__tests__/integration/pipeline-field-filter.test.ts:404 "pipeline custom-field filter SQL > boolean > %j matches %j ({"operator":"is","value":"true"} -> ["Yes"])"
#[tokio::test]
async fn pipeline_custom_field_filter_sql_boolean_case_01_is_true() {
    let db = seed(BOOLEAN).await;
    assert_eq!(
        run(&db, cond("is", Some("true"), None), None).await,
        names(&["Yes"])
    );
}

/// Port of: packages/database/__tests__/integration/pipeline-field-filter.test.ts:404 "pipeline custom-field filter SQL > boolean > %j matches %j ({"operator":"is","value":"false"} -> ["No"])"
#[tokio::test]
async fn pipeline_custom_field_filter_sql_boolean_case_02_is_false() {
    let db = seed(BOOLEAN).await;
    assert_eq!(
        run(&db, cond("is", Some("false"), None), None).await,
        names(&["No"])
    );
}

/// Port of: packages/database/__tests__/integration/pipeline-field-filter.test.ts:404 "pipeline custom-field filter SQL > boolean > %j matches %j ({"operator":"isNot","value":"true"} -> ["No"])"
#[tokio::test]
async fn pipeline_custom_field_filter_sql_boolean_case_03_is_not_true() {
    let db = seed(BOOLEAN).await;
    assert_eq!(
        run(&db, cond("isNot", Some("true"), None), None).await,
        names(&["No"])
    );
}

/// Port of: packages/database/__tests__/integration/pipeline-field-filter.test.ts:404 "pipeline custom-field filter SQL > boolean > %j matches %j ({"operator":"isNot","value":"false"} -> ["Yes"])"
#[tokio::test]
async fn pipeline_custom_field_filter_sql_boolean_case_04_is_not_false() {
    let db = seed(BOOLEAN).await;
    assert_eq!(
        run(&db, cond("isNot", Some("false"), None), None).await,
        names(&["Yes"])
    );
}

/// Port of: packages/database/__tests__/integration/pipeline-field-filter.test.ts:404 "pipeline custom-field filter SQL > boolean > %j matches %j ({"operator":"hasValue"} -> ["Yes","No","Maybe"])"
#[tokio::test]
async fn pipeline_custom_field_filter_sql_boolean_case_05_has_value() {
    let db = seed(BOOLEAN).await;
    assert_eq!(
        run(&db, cond("hasValue", None, None), None).await,
        names(&["Yes", "No", "Maybe"])
    );
}

/// Port of: packages/database/__tests__/integration/pipeline-field-filter.test.ts:404 "pipeline custom-field filter SQL > boolean > %j matches %j ({"operator":"noValue"} -> ["Empty","Missing"])"
#[tokio::test]
async fn pipeline_custom_field_filter_sql_boolean_case_06_no_value() {
    let db = seed(BOOLEAN).await;
    assert_eq!(
        run(&db, cond("noValue", None, None), None).await,
        names(&["Empty", "Missing"])
    );
}

/// Port of: packages/database/__tests__/integration/pipeline-field-filter.test.ts:404 "pipeline custom-field filter SQL > boolean > %j matches %j ({"operator":"is","value":"maybe"} -> ["Yes","No","Maybe","Empty","Missing"])"
#[tokio::test]
async fn pipeline_custom_field_filter_sql_boolean_case_07_is_maybe() {
    let db = seed(BOOLEAN).await;
    assert_eq!(
        run(&db, cond("is", Some("maybe"), None), None).await,
        names(&["Yes", "No", "Maybe", "Empty", "Missing"])
    );
}
