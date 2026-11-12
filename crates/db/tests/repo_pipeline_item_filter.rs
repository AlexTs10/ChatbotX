//! Port of `packages/database/__tests__/pipeline-item-filter.test.ts` (P3.3). The cursor
//! cases are ported; the ordering and WHERE cases rendered the Drizzle SQL with
//! `PgDialect` and are asserted on the SQLite text and bind values of the same builders
//! (adapted: `?` marks, integer ms / x 10,000 binds, JSON paths for field keys).

use std::collections::BTreeMap;

use chatbotx_db::repo::parse_iso;
use chatbotx_db::repo::pipeline_field_condition::{
    CustomFieldType, PipelineFieldCondition, PipelineFieldOperator,
};
use chatbotx_db::repo::pipeline_item_cursor::{
    PipelineItemCursor, PipelineItemSort, PipelineItemSortKey, decode_pipeline_item_cursor,
    encode_pipeline_item_cursor, is_invalid_cursor, pipeline_item_after_cursor,
    pipeline_item_order_by,
};
use chatbotx_db::repo::pipeline_item_filter::{
    ClosedAs, DateRange, PipelineItemFilter, PipelineValueFilter, PipelineValueOperator,
    build_pipeline_item_where,
};
use chatbotx_db::repo::sql::Sql;
use chatbotx_db::repo::utils::SortDir;
use chatbotx_db::rusqlite::types::Value;
use regex::Regex;

const SORT_VALUE_DESC: PipelineItemSort = PipelineItemSort {
    key: PipelineItemSortKey::Value,
    direction: SortDir::Desc,
};
const SORT_CREATED_ASC: PipelineItemSort = PipelineItemSort {
    key: PipelineItemSortKey::CreatedAt,
    direction: SortDir::Asc,
};

fn cursor() -> PipelineItemCursor {
    PipelineItemCursor {
        v: 1,
        sort: PipelineItemSortKey::Value,
        dir: SortDir::Desc,
        null_rank: 0,
        sort_value: Some("12.5000".to_owned()),
        id: "42".to_owned(),
    }
}

fn encode_for(edit: impl FnOnce(&mut PipelineItemCursor)) -> String {
    let mut c = cursor();
    edit(&mut c);
    encode_pipeline_item_cursor(&c)
}

fn created_asc(c: &mut PipelineItemCursor, sort_value: &str) {
    c.sort = PipelineItemSortKey::CreatedAt;
    c.dir = SortDir::Asc;
    c.sort_value = Some(sort_value.to_owned());
}

/// `Buffer.from(text).toString("base64url")`.
fn base64url(text: &str) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::new();
    for chunk in text.as_bytes().chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for i in 0..=chunk.len() {
            out.push(char::from(A[((n >> (18 - 6 * i)) & 63) as usize]));
        }
    }
    out
}

fn assert_rejected(raw: &str, sort: PipelineItemSort) {
    let err = decode_pipeline_item_cursor(raw, sort).expect_err("rejected");
    assert!(is_invalid_cursor(&err), "{err:?}");
}

/// Port of: packages/database/__tests__/pipeline-item-filter.test.ts:38 "pipeline item cursor > round-trips through encode and decode"
#[test]
fn pipeline_item_cursor_round_trips_through_encode_and_decode() {
    let decoded =
        decode_pipeline_item_cursor(&encode_pipeline_item_cursor(&cursor()), SORT_VALUE_DESC)
            .expect("decode");

    assert_eq!(decoded, cursor());
}

/// Port of: packages/database/__tests__/pipeline-item-filter.test.ts:47 "pipeline item cursor > rejects a cursor issued for another sort"
#[test]
fn pipeline_item_cursor_rejects_a_cursor_issued_for_another_sort() {
    let encoded = encode_pipeline_item_cursor(&cursor());

    assert_rejected(&encoded, SORT_CREATED_ASC);
}

/// Port of: packages/database/__tests__/pipeline-item-filter.test.ts:58 "pipeline item cursor > rejects %s (not base64 json)"
#[test]
fn pipeline_item_cursor_rejects_not_base64_json() {
    assert_rejected("%%%", SORT_VALUE_DESC);
}

/// Port of: packages/database/__tests__/pipeline-item-filter.test.ts:58 "pipeline item cursor > rejects %s (a JSON null)"
#[test]
fn pipeline_item_cursor_rejects_a_json_null() {
    assert_rejected(&base64url("null"), SORT_VALUE_DESC);
}

/// Port of: packages/database/__tests__/pipeline-item-filter.test.ts:58 "pipeline item cursor > rejects %s (a non numeric id)"
#[test]
fn pipeline_item_cursor_rejects_a_non_numeric_id() {
    assert_rejected(
        &encode_for(|c| c.id = "1; drop table".to_owned()),
        SORT_VALUE_DESC,
    );
}

/// Port of: packages/database/__tests__/pipeline-item-filter.test.ts:58 "pipeline item cursor > rejects %s (a non null rank without a value)"
#[test]
fn pipeline_item_cursor_rejects_a_non_null_rank_without_a_value() {
    assert_rejected(&encode_for(|c| c.sort_value = None), SORT_VALUE_DESC);
}

/// Port of: packages/database/__tests__/pipeline-item-filter.test.ts:58 "pipeline item cursor > rejects %s (a null rank carrying a value)"
#[test]
fn pipeline_item_cursor_rejects_a_null_rank_carrying_a_value() {
    assert_rejected(&encode_for(|c| c.null_rank = 1), SORT_VALUE_DESC);
}

/// Port of: packages/database/__tests__/pipeline-item-filter.test.ts:58 "pipeline item cursor > rejects %s (a non numeric value for the value sort)"
#[test]
fn pipeline_item_cursor_rejects_a_non_numeric_value_for_the_value_sort() {
    assert_rejected(
        &encode_for(|c| c.sort_value = Some("abc".to_owned())),
        SORT_VALUE_DESC,
    );
}

/// Port of: packages/database/__tests__/pipeline-item-filter.test.ts:58 "pipeline item cursor > rejects %s (a non timestamp value for a timestamp sort)"
#[test]
fn pipeline_item_cursor_rejects_a_non_timestamp_value_for_a_timestamp_sort() {
    assert_rejected(&encode_for(|c| created_asc(c, "abc")), SORT_CREATED_ASC);
}

/// Port of: packages/database/__tests__/pipeline-item-filter.test.ts:58 "pipeline item cursor > rejects %s (the wrong version)"
#[test]
fn pipeline_item_cursor_rejects_the_wrong_version() {
    let raw = base64url(
        r#"{"v":2,"sort":"value","dir":"desc","nullRank":0,"sortValue":"12.5000","id":"42"}"#,
    );
    assert_rejected(&raw, SORT_VALUE_DESC);
}

/// Port of: packages/database/__tests__/pipeline-item-filter.test.ts:93 "pipeline item cursor > rejects out-of-range %s (an id that overflows a signed bigint)"
#[test]
fn pipeline_item_cursor_rejects_out_of_range_an_id_that_overflows_a_signed_bigint() {
    assert_rejected(
        &encode_for(|c| c.id = "9223372036854775808".to_owned()),
        SORT_VALUE_DESC,
    );
}

/// Port of: packages/database/__tests__/pipeline-item-filter.test.ts:93 "pipeline item cursor > rejects out-of-range %s (an id longer than 19 digits)"
#[test]
fn pipeline_item_cursor_rejects_out_of_range_an_id_longer_than_19_digits() {
    assert_rejected(
        &encode_for(|c| c.id = "10000000000000000000".to_owned()),
        SORT_VALUE_DESC,
    );
}

/// Port of: packages/database/__tests__/pipeline-item-filter.test.ts:93 "pipeline item cursor > rejects out-of-range %s (month 99)"
#[test]
fn pipeline_item_cursor_rejects_out_of_range_month_99() {
    assert_rejected(
        &encode_for(|c| created_asc(c, "2026-99-25 15:55:32+00")),
        SORT_CREATED_ASC,
    );
}

/// Port of: packages/database/__tests__/pipeline-item-filter.test.ts:93 "pipeline item cursor > rejects out-of-range %s (day 32)"
#[test]
fn pipeline_item_cursor_rejects_out_of_range_day_32() {
    assert_rejected(
        &encode_for(|c| created_asc(c, "2026-10-32 15:55:32+00")),
        SORT_CREATED_ASC,
    );
}

/// Port of: packages/database/__tests__/pipeline-item-filter.test.ts:93 "pipeline item cursor > rejects out-of-range %s (hour 25)"
#[test]
fn pipeline_item_cursor_rejects_out_of_range_hour_25() {
    assert_rejected(
        &encode_for(|c| created_asc(c, "2026-10-25 25:55:32+00")),
        SORT_CREATED_ASC,
    );
}

/// Port of: packages/database/__tests__/pipeline-item-filter.test.ts:93 "pipeline item cursor > rejects out-of-range %s (minute 60)"
#[test]
fn pipeline_item_cursor_rejects_out_of_range_minute_60() {
    assert_rejected(
        &encode_for(|c| created_asc(c, "2026-10-25 15:60:32+00")),
        SORT_CREATED_ASC,
    );
}

/// Port of: packages/database/__tests__/pipeline-item-filter.test.ts:93 "pipeline item cursor > rejects out-of-range %s (an offset of 16 hours)"
#[test]
fn pipeline_item_cursor_rejects_out_of_range_an_offset_of_16_hours() {
    assert_rejected(
        &encode_for(|c| created_asc(c, "2026-10-25 15:55:32+16")),
        SORT_CREATED_ASC,
    );
}

/// Port of: packages/database/__tests__/pipeline-item-filter.test.ts:93 "pipeline item cursor > rejects out-of-range %s (an absurdly long numeric value)"
#[test]
fn pipeline_item_cursor_rejects_out_of_range_an_absurdly_long_numeric_value() {
    let long = format!("{}.5", "9".repeat(40));
    assert_rejected(&encode_for(|c| c.sort_value = Some(long)), SORT_VALUE_DESC);
}

/// Port of: packages/database/__tests__/pipeline-item-filter.test.ts:128 "pipeline item cursor > accepts the largest signed bigint id"
#[test]
fn pipeline_item_cursor_accepts_the_largest_signed_bigint_id() {
    let raw = encode_for(|c| c.id = "9223372036854775807".to_owned());

    assert_eq!(
        decode_pipeline_item_cursor(&raw, SORT_VALUE_DESC)
            .expect("decode")
            .id,
        "9223372036854775807"
    );
}

fn accepts_timestamptz_text(sort_value: &str) {
    let raw = encode_for(|c| created_asc(c, sort_value));

    assert_eq!(
        decode_pipeline_item_cursor(&raw, SORT_CREATED_ASC)
            .expect("decode")
            .sort_value
            .as_deref(),
        Some(sort_value)
    );
}

/// Port of: packages/database/__tests__/pipeline-item-filter.test.ts:136 "pipeline item cursor > accepts the Postgres timestamptz text %s (20000-01-01 00:00:00+00)"
#[test]
fn pipeline_item_cursor_accepts_the_postgres_timestamptz_text_20000_01_01_00_00_00_00() {
    accepts_timestamptz_text("20000-01-01 00:00:00+00");
}

/// Port of: packages/database/__tests__/pipeline-item-filter.test.ts:136 "pipeline item cursor > accepts the Postgres timestamptz text %s (1900-01-01 07:06:30+07:06:30)"
#[test]
fn pipeline_item_cursor_accepts_the_postgres_timestamptz_text_1900_01_01_07_06_30_07_06_30() {
    accepts_timestamptz_text("1900-01-01 07:06:30+07:06:30");
}

/// Port of: packages/database/__tests__/pipeline-item-filter.test.ts:136 "pipeline item cursor > accepts the Postgres timestamptz text %s (0044-03-15 12:00:00+00 BC)"
#[test]
fn pipeline_item_cursor_accepts_the_postgres_timestamptz_text_0044_03_15_12_00_00_00_bc() {
    accepts_timestamptz_text("0044-03-15 12:00:00+00 BC");
}

/// Port of: packages/database/__tests__/pipeline-item-filter.test.ts:136 "pipeline item cursor > accepts the Postgres timestamptz text %s (2026-10-25 15:55:32.5+05:30)"
#[test]
fn pipeline_item_cursor_accepts_the_postgres_timestamptz_text_2026_10_25_15_55_32_5_05_30() {
    accepts_timestamptz_text("2026-10-25 15:55:32.5+05:30");
}

/// Port of: packages/database/__tests__/pipeline-item-filter.test.ts:149 "pipeline item cursor > accepts a Postgres timestamptz text value"
#[test]
fn pipeline_item_cursor_accepts_a_postgres_timestamptz_text_value() {
    let raw = encode_for(|c| created_asc(c, "2026-10-25 15:55:32.123456+00"));

    assert_eq!(
        decode_pipeline_item_cursor(&raw, SORT_CREATED_ASC)
            .expect("decode")
            .id,
        "42"
    );
}

/// Port of: packages/database/__tests__/pipeline-item-filter.test.ts:161 "pipeline item ordering > omits the null-rank term for sort keys that cannot be NULL"
#[test]
fn pipeline_item_ordering_omits_the_null_rank_term_for_sort_keys_that_cannot_be_null() {
    let parts = pipeline_item_order_by(SORT_CREATED_ASC);

    assert_eq!(parts.len(), 2);
    assert!(!parts.join(" ").contains("is null"));
}

/// Port of: packages/database/__tests__/pipeline-item-filter.test.ts:170 "pipeline item ordering > puts NULL sort values last and breaks ties by id"
#[test]
fn pipeline_item_ordering_puts_null_sort_values_last_and_breaks_ties_by_id() {
    let parts = pipeline_item_order_by(SORT_VALUE_DESC);
    let rendered = parts.join(", ");

    assert!(parts[0].contains("is null"));
    assert!(rendered.contains("desc"));
    assert!(rendered.trim().ends_with("desc"));
}

fn after(sort: PipelineItemSort, edit: impl FnOnce(&mut PipelineItemCursor)) -> Sql {
    let mut c = cursor();
    edit(&mut c);
    pipeline_item_after_cursor(sort, &c)
}

/// Port of: packages/database/__tests__/pipeline-item-filter.test.ts:181 "pipeline item ordering > keyset after a non-null value includes NULL rows and binds the value"
#[test]
fn pipeline_item_ordering_keyset_after_a_non_null_value_includes_null_rows_and_binds_the_value() {
    let predicate = after(SORT_VALUE_DESC, |c| c.sort_value = Some("12.5".to_owned()));

    assert!(predicate.text.contains("is null or"));
    assert!(predicate.text.contains('<'));
    // `12.5` is bound as money x 10,000.
    assert!(predicate.params.contains(&Value::Integer(125_000)));
    assert!(predicate.params.contains(&Value::Integer(42)));
}

/// Port of: packages/database/__tests__/pipeline-item-filter.test.ts:199 "pipeline item ordering > keyset after a NULL value only walks the remaining NULL rows by id"
#[test]
fn pipeline_item_ordering_keyset_after_a_null_value_only_walks_the_remaining_null_rows_by_id() {
    let predicate = after(SORT_VALUE_DESC, |c| {
        c.null_rank = 1;
        c.sort_value = None;
    });

    assert!(predicate.text.contains("is null"));
    assert!(!predicate.text.contains(" or "));
}

/// Port of: packages/database/__tests__/pipeline-item-filter.test.ts:215 "pipeline item ordering > keyset over a non-nullable key is one row comparison the index can start from"
#[test]
fn pipeline_item_ordering_keyset_over_a_non_nullable_key_is_one_row_comparison_the_index_can_start_from()
 {
    let predicate = after(SORT_CREATED_ASC, |c| {
        created_asc(c, "2026-01-01 00:00:00.123456+00")
    });
    let row_comparison_after = Regex::new(r"\(\(.+, .+\) > \(\?, \?\)\)").expect("regex");

    assert!(
        row_comparison_after.is_match(&predicate.text),
        "{}",
        predicate.text
    );
    assert!(!predicate.text.contains(" or "));
    // The timestamp is bound as unix ms (microseconds truncated), the id as an integer.
    assert_eq!(
        predicate.params,
        vec![
            Value::Integer(parse_iso("2026-01-01T00:00:00.123Z").expect("iso")),
            Value::Integer(42),
        ]
    );
}

fn render(filter: &PipelineItemFilter) -> Sql {
    build_pipeline_item_where(7, None, filter)
}

fn text(v: &str) -> Value {
    Value::Text(v.to_owned())
}

fn conditions(
    items: &[(&str, PipelineFieldCondition, Option<CustomFieldType>)],
) -> (
    BTreeMap<String, PipelineFieldCondition>,
    BTreeMap<String, CustomFieldType>,
) {
    let mut c = BTreeMap::new();
    let mut t = BTreeMap::new();
    for (id, condition, field_type) in items {
        c.insert((*id).to_owned(), condition.clone());
        if let Some(ft) = field_type {
            t.insert((*id).to_owned(), *ft);
        }
    }
    (c, t)
}

/// Port of: packages/database/__tests__/pipeline-item-filter.test.ts:234 "buildPipelineItemWhere > always scopes by workspace and binds filter values as parameters"
#[test]
fn build_pipeline_item_where_always_scopes_by_workspace_and_binds_filter_values_as_parameters() {
    let (field_conditions, field_types) = conditions(&[(
        "55",
        PipelineFieldCondition::with(PipelineFieldOperator::Is, "gold"),
        Some(CustomFieldType::ShortText),
    )]);
    let filter = PipelineItemFilter {
        statuses: vec!["open".to_owned()],
        search: Some("'; drop table x; --".to_owned()),
        field_conditions,
        field_types,
        ..Default::default()
    };
    let where_sql = build_pipeline_item_where(7, Some(9), &filter);

    assert!(where_sql.text.contains("\"workspaceId\""));
    assert!(!where_sql.text.contains("drop table"));
    assert!(where_sql.params.contains(&Value::Integer(7)));
    assert!(where_sql.params.contains(&text("gold")));
    // The field key is bound as its JSON path.
    assert!(where_sql.params.contains(&text("$.\"55\"")));
}

fn search(s: &str) -> Sql {
    render(&PipelineItemFilter {
        search: Some(s.to_owned()),
        ..Default::default()
    })
}

/// Port of: packages/database/__tests__/pipeline-item-filter.test.ts:254 "buildPipelineItemWhere > treats a numeric search as an item number lookup"
#[test]
fn build_pipeline_item_where_treats_a_numeric_search_as_an_item_number_lookup() {
    assert!(search("#123").params.contains(&Value::Integer(123)));
}

/// Port of: packages/database/__tests__/pipeline-item-filter.test.ts:262 "buildPipelineItemWhere > a zero-padded item number finds the item, compared as a number"
#[test]
fn build_pipeline_item_where_a_zero_padded_item_number_finds_the_item_compared_as_a_number() {
    let where_sql = search("#0012");

    assert!(where_sql.params.contains(&Value::Integer(12)));
    assert!(where_sql.text.contains("\"number\" = ?"));
    assert!(!where_sql.text.contains("\"number\" AS TEXT"));
}

fn search_checks(s: &str, email: bool, phone: bool) {
    let where_sql = search(s);

    assert_eq!(where_sql.text.contains("\"email\""), email);
    assert_eq!(where_sql.text.contains("\"phoneNumber\""), phone);
    assert!(where_sql.text.contains("\"fullName\""));
}

/// Port of: packages/database/__tests__/pipeline-item-filter.test.ts:274 "buildPipelineItemWhere > search %j checks email %o (9)"
#[test]
fn build_pipeline_item_where_search_checks_email_9() {
    search_checks("9", false, false);
}

/// Port of: packages/database/__tests__/pipeline-item-filter.test.ts:274 "buildPipelineItemWhere > search %j checks email %o (ann)"
#[test]
fn build_pipeline_item_where_search_checks_email_ann() {
    search_checks("ann", false, false);
}

/// Port of: packages/database/__tests__/pipeline-item-filter.test.ts:274 "buildPipelineItemWhere > search %j checks email %o (ann@example)"
#[test]
fn build_pipeline_item_where_search_checks_email_ann_example() {
    search_checks("ann@example", true, false);
}

/// Port of: packages/database/__tests__/pipeline-item-filter.test.ts:274 "buildPipelineItemWhere > search %j checks email %o (0901 234)"
#[test]
fn build_pipeline_item_where_search_checks_email_0901_234() {
    search_checks("0901 234", false, true);
}

/// Port of: packages/database/__tests__/pipeline-item-filter.test.ts:274 "buildPipelineItemWhere > search %j checks email %o (+84 90-123)"
#[test]
fn build_pipeline_item_where_search_checks_email_84_90_123() {
    search_checks("+84 90-123", false, true);
}

/// Port of: packages/database/__tests__/pipeline-item-filter.test.ts:290 "buildPipelineItemWhere > a phone search matches the digits whatever the formatting"
#[test]
fn build_pipeline_item_where_a_phone_search_matches_the_digits_whatever_the_formatting() {
    assert!(search("+84 (90) 123").params.contains(&text("%8490123%")));
}

/// Port of: packages/database/__tests__/pipeline-item-filter.test.ts:298 "buildPipelineItemWhere > closedAs pairs the status with its closure range as bound params"
#[test]
fn build_pipeline_item_where_closed_as_pairs_the_status_with_its_closure_range_as_bound_params() {
    let from = parse_iso("2026-01-01T00:00:00.000Z").expect("iso");
    let to = parse_iso("2026-01-31T23:59:59.999Z").expect("iso");
    let where_sql = render(&PipelineItemFilter {
        closed_as: Some(ClosedAs {
            won: Some(DateRange {
                from: Some(from),
                to: Some(to),
            }),
            lost: Some(DateRange {
                from: Some(from),
                to: None,
            }),
        }),
        ..Default::default()
    });

    assert!(where_sql.text.contains("\"status\" = ?"));
    assert!(where_sql.text.contains(" OR "));
    assert!(!where_sql.text.contains("won"));
    for value in [
        text("won"),
        text("lost"),
        Value::Integer(from),
        Value::Integer(to),
    ] {
        assert!(where_sql.params.contains(&value), "{value:?}");
    }
}

/// Port of: packages/database/__tests__/pipeline-item-filter.test.ts:321 "buildPipelineItemWhere > closedAs with a single status adds no OR"
#[test]
fn build_pipeline_item_where_closed_as_with_a_single_status_adds_no_or() {
    let from = parse_iso("2026-01-01T00:00:00.000Z").expect("iso");
    let where_sql = render(&PipelineItemFilter {
        closed_as: Some(ClosedAs {
            won: Some(DateRange {
                from: Some(from),
                to: None,
            }),
            lost: None,
        }),
        ..Default::default()
    });

    assert!(!where_sql.text.contains(" OR "));
}

/// Port of: packages/database/__tests__/pipeline-item-filter.test.ts:333 "buildPipelineItemWhere > binds the item's own comment, status and stage timestamps"
#[test]
fn build_pipeline_item_where_binds_the_item_s_own_comment_status_and_stage_timestamps() {
    let from = DateRange {
        from: Some(parse_iso("2026-01-01T00:00:00.000Z").expect("iso")),
        to: None,
    };
    let where_sql = render(&PipelineItemFilter {
        last_comment_at: Some(from),
        status_changed_at: Some(from),
        stage_entered_at: Some(from),
        ..Default::default()
    });

    assert!(where_sql.text.contains("\"lastCommentAt\" >="));
    assert!(where_sql.text.contains("\"statusChangedAt\" >="));
    assert!(where_sql.text.contains("\"stageEnteredAt\" >="));
    assert!(!where_sql.text.contains("PipelineItemActivity"));
}

/// Port of: packages/database/__tests__/pipeline-item-filter.test.ts:352 "buildPipelineItemWhere > a date condition binds key, day operand and zone, with a guarded cast"
#[test]
fn build_pipeline_item_where_a_date_condition_binds_key_day_operand_and_zone_with_a_guarded_cast() {
    let (field_conditions, field_types) = conditions(&[(
        "99",
        PipelineFieldCondition::between(Some("2026-05-01"), Some("2026-05-31")),
        Some(CustomFieldType::Datetime),
    )]);
    let where_sql = render(&PipelineItemFilter {
        field_conditions,
        field_types,
        timezone: Some("Asia/Ho_Chi_Minh".to_owned()),
        ..Default::default()
    });
    let guarded_cast =
        Regex::new(r"case when regexp\(\?, .*\) then .*unixepoch\(.*\) end").expect("regex");

    // The parse only runs inside a CASE guarded by the pattern check.
    assert!(guarded_cast.is_match(&where_sql.text), "{}", where_sql.text);
    assert!(where_sql.text.contains("tz_wall_ms"));
    for value in [
        text("$.\"99\""),
        text("2026-05-01"),
        text("2026-05-31"),
        text("Asia/Ho_Chi_Minh"),
    ] {
        assert!(where_sql.params.contains(&value), "{value:?}");
    }
    assert!(!where_sql.text.contains("99"));
}

/// Port of: packages/database/__tests__/pipeline-item-filter.test.ts:384 "buildPipelineItemWhere > an unknown timezone falls back to UTC instead of reaching SQL"
#[test]
fn build_pipeline_item_where_an_unknown_timezone_falls_back_to_utc_instead_of_reaching_sql() {
    let (field_conditions, field_types) = conditions(&[(
        "99",
        PipelineFieldCondition::with(PipelineFieldOperator::Is, "2026-05-01"),
        Some(CustomFieldType::Datetime),
    )]);
    let where_sql = render(&PipelineItemFilter {
        field_conditions,
        field_types,
        timezone: Some("Mars/Olympus'; drop table x".to_owned()),
        ..Default::default()
    });

    assert!(where_sql.params.contains(&text("UTC")));
    assert!(
        !where_sql
            .params
            .contains(&text("Mars/Olympus'; drop table x"))
    );
}

fn value_sql(
    operator: Option<PipelineValueOperator>,
    amount: Option<i64>,
    min: Option<i64>,
    max: Option<i64>,
) -> Sql {
    render(&PipelineItemFilter {
        value: Some(PipelineValueFilter {
            min,
            max,
            operator,
            amount,
        }),
        ..Default::default()
    })
}

/// Port of: packages/database/__tests__/pipeline-item-filter.test.ts:401 "buildPipelineItemWhere > value operators bind the amount and keep the legacy interval"
#[test]
fn build_pipeline_item_where_value_operators_bind_the_amount_and_keep_the_legacy_interval() {
    use PipelineValueOperator::{Between, Eq, Gt, Lt, Ne};
    let eq = value_sql(Some(Eq), Some(5), None, None);
    assert!(eq.text.contains("\"value\" ="));
    assert!(eq.params.contains(&Value::Integer(5)));
    assert!(
        value_sql(Some(Gt), Some(5), None, None)
            .text
            .contains("\"value\" >")
    );
    assert!(
        value_sql(Some(Lt), Some(5), None, None)
            .text
            .contains("\"value\" <")
    );
    // `is distinct from` is SQLite `IS NOT`.
    assert!(
        value_sql(Some(Ne), Some(5), None, None)
            .text
            .contains("\"value\" IS NOT")
    );
    let between = value_sql(Some(Between), None, Some(1), Some(9));
    assert!(between.text.contains("\"value\" >="));
    assert!(between.text.contains("\"value\" <="));
    assert!(
        value_sql(None, None, Some(1), None)
            .text
            .contains("\"value\" >=")
    );
    // A single-amount operator without an amount adds nothing.
    assert_eq!(
        value_sql(Some(Gt), None, None, None).text,
        render(&PipelineItemFilter::default()).text
    );
}

/// Port of: packages/database/__tests__/pipeline-item-filter.test.ts:425 "buildPipelineItemWhere > field conditions bind keys and operands and never interpolate them"
#[test]
fn build_pipeline_item_where_field_conditions_bind_keys_and_operands_and_never_interpolate_them() {
    use PipelineFieldOperator::{Contains, EndsWith, Gt, NoValue, StartsWith};
    let (field_conditions, field_types) = conditions(&[
        (
            "11",
            PipelineFieldCondition::with(Contains, "50%_x"),
            Some(CustomFieldType::ShortText),
        ),
        (
            "12",
            PipelineFieldCondition::with(StartsWith, "ab"),
            Some(CustomFieldType::Email),
        ),
        (
            "13",
            PipelineFieldCondition::with(EndsWith, "yz"),
            Some(CustomFieldType::LongText),
        ),
        (
            "14",
            PipelineFieldCondition::with(Gt, "3"),
            Some(CustomFieldType::Number),
        ),
        (
            "15",
            PipelineFieldCondition::op(NoValue),
            Some(CustomFieldType::Number),
        ),
    ]);
    let where_sql = render(&PipelineItemFilter {
        field_conditions,
        field_types,
        ..Default::default()
    });
    let guarded_numeric_cast =
        Regex::new(r"case when regexp\(\?, .*\) then CAST\(.* AS REAL\) end").expect("regex");

    for value in [
        text("$.\"11\""),
        text("%50\\%\\_x%"),
        text("ab%"),
        text("%yz"),
        text("3"),
        text("$.\"15\""),
    ] {
        assert!(where_sql.params.contains(&value), "{value:?}");
    }
    assert!(where_sql.text.contains("ilike"));
    assert!(
        guarded_numeric_cast.is_match(&where_sql.text),
        "{}",
        where_sql.text
    );
    assert!(!where_sql.text.contains("50%"));
}

/// Port of: packages/database/__tests__/pipeline-item-filter.test.ts:456 "buildPipelineItemWhere > a condition missing its operand, its field type or a usable operand adds nothing"
#[test]
fn build_pipeline_item_where_a_condition_missing_its_operand_its_field_type_or_a_usable_operand_adds_nothing()
 {
    use PipelineFieldOperator::{Between, Contains, Gt, Is};
    let plain = render(&PipelineItemFilter::default());
    let (field_conditions, field_types) = conditions(&[
        (
            "11",
            PipelineFieldCondition::op(Contains),
            Some(CustomFieldType::ShortText),
        ),
        (
            "12",
            PipelineFieldCondition::with(Is, "abc"),
            Some(CustomFieldType::Number),
        ),
        (
            "13",
            PipelineFieldCondition::with(Is, ""),
            Some(CustomFieldType::ShortText),
        ),
        (
            "14",
            PipelineFieldCondition::with(Gt, "2026-02-30"),
            Some(CustomFieldType::Date),
        ),
        (
            "15",
            PipelineFieldCondition::with(Is, "maybe"),
            Some(CustomFieldType::Boolean),
        ),
        (
            "16",
            PipelineFieldCondition::op(Between),
            Some(CustomFieldType::Number),
        ),
        ("17", PipelineFieldCondition::with(Is, "typeless"), None),
    ]);
    let where_sql = render(&PipelineItemFilter {
        field_conditions,
        field_types,
        ..Default::default()
    });

    assert_eq!(where_sql.text, plain.text);
}

/// Port of: packages/database/__tests__/pipeline-item-filter.test.ts:486 "buildPipelineItemWhere > closedAs without a range adds no condition"
#[test]
fn build_pipeline_item_where_closed_as_without_a_range_adds_no_condition() {
    let plain = render(&PipelineItemFilter::default());
    let empty = render(&PipelineItemFilter {
        closed_as: Some(ClosedAs::default()),
        ..Default::default()
    });

    assert_eq!(empty.text, plain.text);
}
