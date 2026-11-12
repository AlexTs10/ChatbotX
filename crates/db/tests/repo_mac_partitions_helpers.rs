//! Port of the date/name helper cases of
//! `packages/database/__tests__/mac-partitions-repository.test.ts` (P3.3; the
//! partition/pg_class cases are not-applicable since P2.5).

use chatbotx_db::repo::mac_partitions::{
    add_utc_months, format_monthly_partition_name, format_utc_date,
};
use chatbotx_db::repo::{iso, parse_iso};

fn utc(text: &str) -> i64 {
    parse_iso(text).expect("iso")
}

/// Port of: packages/database/__tests__/mac-partitions-repository.test.ts:118 "date/name helpers > addUtcMonths advances by whole UTC months, normalized to day 1"
#[test]
fn date_name_helpers_add_utc_months_advances_by_whole_utc_months_normalized_to_day_1() {
    let result = add_utc_months(utc("2026-12-15T00:00:00.000Z"), 2);
    assert_eq!(iso(result), "2027-02-01T00:00:00.000Z");
}

/// Port of: packages/database/__tests__/mac-partitions-repository.test.ts:123 "date/name helpers > formatMonthlyPartitionName zero-pads the month"
#[test]
fn date_name_helpers_format_monthly_partition_name_zero_pads_the_month() {
    assert_eq!(
        format_monthly_partition_name(utc("2026-01-01T00:00:00.000Z")),
        "ContactActiveHourly_2026_01"
    );
}

/// Port of: packages/database/__tests__/mac-partitions-repository.test.ts:129 "date/name helpers > formatUtcDate renders YYYY-MM-01"
#[test]
fn date_name_helpers_format_utc_date_renders_yyyy_mm_01() {
    assert_eq!(
        format_utc_date(utc("2026-09-20T00:00:00.000Z")),
        "2026-09-01"
    );
}
