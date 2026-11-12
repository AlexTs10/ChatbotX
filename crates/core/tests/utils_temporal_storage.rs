//! Port of `packages/utils/__tests__/temporal-storage.test.ts`. The loose
//! parser's `now` (default `new Date()`) is the real clock.

mod support;

use chatbotx_core::utils::datetime::{TemporalCustomFieldType, TemporalInputParsing};
use chatbotx_core::utils::temporal_input::{
    TemporalStorageNormalizationInput, normalize_temporal_value_for_storage,
};

const ANCHOR: &str = "Asia/Ho_Chi_Minh";

fn normalize(
    kind: TemporalCustomFieldType,
    value: &str,
    timezone: Option<&str>,
    parsing: Option<TemporalInputParsing>,
) -> Option<String> {
    normalize_temporal_value_for_storage(
        &TemporalStorageNormalizationInput {
            kind,
            value,
            timezone,
            parsing,
        },
        support::now_ms(),
    )
}

fn s(value: &str) -> Option<String> {
    Some(value.to_owned())
}

use TemporalCustomFieldType::{Date, Datetime};
const LENIENT: Option<TemporalInputParsing> = Some(TemporalInputParsing::Lenient);

/// Port of: packages/utils/__tests__/temporal-storage.test.ts:9 "normalizeTemporalValueForStorage > strict parsing (default) > accepts canonical values"
#[test]
fn normalizetemporalvalueforstorage_strict_parsing_default_accepts_canonical_values() {
    assert_eq!(
        normalize(Date, "2026-05-19", Some(ANCHOR), None),
        s("2026-05-19T00:00:00+07:00")
    );
    assert_eq!(
        normalize(Datetime, "2026-05-19 23:30:00", Some(ANCHOR), None),
        s("2026-05-19T16:30:00.000Z")
    );
}

/// Port of: packages/utils/__tests__/temporal-storage.test.ts:27 "normalizeTemporalValueForStorage > strict parsing (default) > rejects non-canonical values instead of guessing"
#[test]
fn normalizetemporalvalueforstorage_strict_parsing_default_rejects_non_canonical_values_instead_of_guessing()
 {
    assert_eq!(normalize(Date, "19/05/2026", Some(ANCHOR), None), None);
}

/// Port of: packages/utils/__tests__/temporal-storage.test.ts:41 "normalizeTemporalValueForStorage > lenient parsing > keeps the authored calendar day of date 2026-05-19T23:30:00-04:00"
#[test]
fn normalizetemporalvalueforstorage_lenient_parsing_keeps_the_authored_calendar_day_of_date_2026_05_19t23_30_00_04_00()
 {
    assert_eq!(
        normalize(Date, "2026-05-19T23:30:00-04:00", Some(ANCHOR), LENIENT),
        s("2026-05-19T00:00:00+07:00")
    );
}

/// Port of: packages/utils/__tests__/temporal-storage.test.ts:41 "normalizeTemporalValueForStorage > lenient parsing > keeps the authored calendar day of date 2026-05-19T20:00:00Z"
#[test]
fn normalizetemporalvalueforstorage_lenient_parsing_keeps_the_authored_calendar_day_of_date_2026_05_19t20_00_00z()
 {
    assert_eq!(
        normalize(Date, "2026-05-19T20:00:00Z", Some(ANCHOR), LENIENT),
        s("2026-05-19T00:00:00+07:00")
    );
}

/// Port of: packages/utils/__tests__/temporal-storage.test.ts:41 "normalizeTemporalValueForStorage > lenient parsing > keeps the authored calendar day of date 2026-05-19 23:30:00"
#[test]
fn normalizetemporalvalueforstorage_lenient_parsing_keeps_the_authored_calendar_day_of_date_2026_05_19_23_30_00()
 {
    assert_eq!(
        normalize(Date, "2026-05-19 23:30:00", Some(ANCHOR), LENIENT),
        s("2026-05-19T00:00:00+07:00")
    );
}

/// Port of: packages/utils/__tests__/temporal-storage.test.ts:41 "normalizeTemporalValueForStorage > lenient parsing > keeps the authored calendar day of date 2026-05-19"
#[test]
fn normalizetemporalvalueforstorage_lenient_parsing_keeps_the_authored_calendar_day_of_date_2026_05_19()
 {
    assert_eq!(
        normalize(Date, "2026-05-19", Some(ANCHOR), LENIENT),
        s("2026-05-19T00:00:00+07:00")
    );
}

/// Port of: packages/utils/__tests__/temporal-storage.test.ts:57 "normalizeTemporalValueForStorage > lenient parsing > preserves milliseconds the loose parser would drop"
#[test]
fn normalizetemporalvalueforstorage_lenient_parsing_preserves_milliseconds_the_loose_parser_would_drop()
 {
    assert_eq!(
        normalize(Datetime, "2026-05-19T23:30:00.123", Some(ANCHOR), LENIENT),
        s("2026-05-19T16:30:00.123Z")
    );
}

/// Port of: packages/utils/__tests__/temporal-storage.test.ts:68 "normalizeTemporalValueForStorage > lenient parsing > falls back to the loose parser for date 19/05/2026"
#[test]
fn normalizetemporalvalueforstorage_lenient_parsing_falls_back_to_the_loose_parser_for_date_19_05_2026()
 {
    assert_eq!(
        normalize(Date, "19/05/2026", Some(ANCHOR), LENIENT),
        s("2026-05-19T00:00:00+07:00")
    );
}

/// Port of: packages/utils/__tests__/temporal-storage.test.ts:68 "normalizeTemporalValueForStorage > lenient parsing > falls back to the loose parser for date 1700000000"
#[test]
fn normalizetemporalvalueforstorage_lenient_parsing_falls_back_to_the_loose_parser_for_date_1700000000()
 {
    assert_eq!(
        normalize(Date, "1700000000", Some(ANCHOR), LENIENT),
        s("2023-11-15T00:00:00+07:00")
    );
}

/// Port of: packages/utils/__tests__/temporal-storage.test.ts:68 "normalizeTemporalValueForStorage > lenient parsing > falls back to the loose parser for date ngày 19 tháng 5 năm 2026"
#[test]
fn normalizetemporalvalueforstorage_lenient_parsing_falls_back_to_the_loose_parser_for_date_ng_y_19_th_ng_5_n_m_2026()
 {
    assert_eq!(
        normalize(Date, "ngày 19 tháng 5 năm 2026", Some(ANCHOR), LENIENT),
        s("2026-05-19T00:00:00+07:00")
    );
}

/// Port of: packages/utils/__tests__/temporal-storage.test.ts:83 "normalizeTemporalValueForStorage > lenient parsing > returns null for genuinely unparseable input"
#[test]
fn normalizetemporalvalueforstorage_lenient_parsing_returns_null_for_genuinely_unparseable_input() {
    assert_eq!(
        normalize(Datetime, "not-a-date-at-all", Some(ANCHOR), LENIENT),
        None
    );
}

/// Port of: packages/utils/__tests__/temporal-storage.test.ts:94 "normalizeTemporalValueForStorage > lenient parsing > rejects a calendar date that does not exist"
#[test]
fn normalizetemporalvalueforstorage_lenient_parsing_rejects_a_calendar_date_that_does_not_exist() {
    assert_eq!(normalize(Date, "2026-02-30", Some(ANCHOR), LENIENT), None);
}

/// Port of: packages/utils/__tests__/temporal-storage.test.ts:106 "normalizeTemporalValueForStorage > anchors to UTC when no usable timezone is supplied"
#[test]
fn normalizetemporalvalueforstorage_anchors_to_utc_when_no_usable_timezone_is_supplied() {
    for timezone in [None, None, Some("Not/AZone")] {
        assert_eq!(
            normalize(Date, "19/05/2026", timezone, LENIENT),
            s("2026-05-19T00:00:00Z")
        );
    }
}
