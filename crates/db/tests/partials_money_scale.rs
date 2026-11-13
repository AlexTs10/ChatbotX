//! Port of `packages/database/__tests__/money-scale.test.ts` (P3.2b). One test per
//! `test.each` row (ported).

use chatbotx_db::repo::partials::shared::round_to_money_scale;

/// `toBe`: the same float, and never `-0` where the TS expects `0` (`Object.is`).
fn assert_rounds(input: f64, expected: f64) {
    let got = round_to_money_scale(input);
    assert_eq!(got, expected, "{input}");
    assert_eq!(
        got.is_sign_negative(),
        expected.is_sign_negative(),
        "sign of {input}"
    );
}

#[test]
#[allow(clippy::float_cmp)]
fn rounds_1_12345_like_a_numeric_19_4_column_to_1_1235() {
    assert_rounds(1.123_45, 1.1235);
}

#[test]
#[allow(clippy::float_cmp)]
fn rounds_1_12344_like_a_numeric_19_4_column_to_1_1234() {
    assert_rounds(1.123_44, 1.1234);
}

#[test]
#[allow(clippy::float_cmp)]
fn rounds_minus_1_12345_like_a_numeric_19_4_column_to_minus_1_1235() {
    assert_rounds(-1.123_45, -1.1235);
}

#[test]
#[allow(clippy::float_cmp)]
fn rounds_0_30000000000000004_like_a_numeric_19_4_column_to_0_3() {
    assert_rounds(0.1 + 0.2, 0.3);
}

#[test]
#[allow(clippy::float_cmp)]
fn rounds_1e_7_like_a_numeric_19_4_column_to_0() {
    assert_rounds(1e-7, 0.0);
}

#[test]
#[allow(clippy::float_cmp)]
fn rounds_42_like_a_numeric_19_4_column_to_42() {
    assert_rounds(42.0, 42.0);
}

#[test]
#[allow(clippy::float_cmp)]
fn rounds_1_6391499999999999_like_a_numeric_19_4_column_to_1_6391() {
    assert_rounds(1.639_149_999_999_999_9, 1.6391);
}

#[test]
#[allow(clippy::float_cmp)]
fn rounds_1000000000000_0001_like_a_numeric_19_4_column_to_1000000000000_0001() {
    assert_rounds(1_000_000_000_000.000_1, 1_000_000_000_000.000_1);
}

#[test]
#[allow(clippy::float_cmp)]
fn rounds_0_99995_like_a_numeric_19_4_column_to_1() {
    assert_rounds(0.999_95, 1.0);
}

#[test]
#[allow(clippy::float_cmp)]
fn rounds_minus_0_00001_like_a_numeric_19_4_column_to_0() {
    assert_rounds(-0.000_01, 0.0);
}
