//! Port of `packages/utils/__tests__/money.test.ts`.

use chatbotx_core::utils::money::{
    FormatMoneyOptions, LinePrice, LinePriceInput, SymbolPosition, compute_line_price,
    format_money, get_currency_fraction_digits, percent_of, round_money, sum_money,
};

fn opts<'a>(
    currency: &'a str,
    symbol: &'a str,
    position: SymbolPosition,
) -> FormatMoneyOptions<'a> {
    FormatMoneyOptions {
        currency,
        symbol: Some(symbol),
        position,
    }
}

/// Port of: packages/utils/__tests__/money.test.ts:12 "getCurrencyFractionDigits > knows currencies without, with two and with three decimals"
#[test]
fn getcurrencyfractiondigits_knows_currencies_without_with_two_and_with_three_decimals() {
    assert_eq!(get_currency_fraction_digits("JPY"), 0);
    assert_eq!(get_currency_fraction_digits("VND"), 0);
    assert_eq!(get_currency_fraction_digits("usd"), 2);
    assert_eq!(get_currency_fraction_digits("KWD"), 3);
}

/// Port of: packages/utils/__tests__/money.test.ts:19 "getCurrencyFractionDigits > falls back to two decimals for a code it does not know"
#[test]
fn getcurrencyfractiondigits_falls_back_to_two_decimals_for_a_code_it_does_not_know() {
    assert_eq!(get_currency_fraction_digits("not-a-code"), 2);
}

/// Port of: packages/utils/__tests__/money.test.ts:25 "roundMoney > rounds a written-down half up instead of down"
#[test]
fn roundmoney_rounds_a_written_down_half_up_instead_of_down() {
    assert_eq!(round_money(1.005, "USD"), 1.01);
    assert_eq!(round_money(2.675, "USD"), 2.68);
}

/// Port of: packages/utils/__tests__/money.test.ts:30 "roundMoney > rounds to the currency's own unit"
#[test]
fn roundmoney_rounds_to_the_currency_s_own_unit() {
    assert_eq!(round_money(1234.5, "JPY"), 1235.0);
    assert_eq!(round_money(1.0005, "KWD"), 1.001);
}

/// Port of: packages/utils/__tests__/money.test.ts:35 "roundMoney > rounds negative amounts away from zero"
#[test]
fn roundmoney_rounds_negative_amounts_away_from_zero() {
    assert_eq!(round_money(-1.005, "USD"), -1.01);
}

/// Port of: packages/utils/__tests__/money.test.ts:41 "computeLinePrice > applies the discount before the tax"
#[test]
fn computelineprice_applies_the_discount_before_the_tax() {
    assert_eq!(
        compute_line_price(&LinePriceInput {
            unit_price: 10.0,
            quantity: 3.0,
            discount_percent: Some(10.0),
            tax_percent: Some(10.0),
            currency: "USD",
        }),
        LinePrice {
            gross: 30.0,
            discount_total: 3.0,
            tax_total: 2.7,
            line_total: 29.7,
        }
    );
}

/// Port of: packages/utils/__tests__/money.test.ts:53 "computeLinePrice > defaults the discount and tax to none"
#[test]
fn computelineprice_defaults_the_discount_and_tax_to_none() {
    assert_eq!(
        compute_line_price(&LinePriceInput {
            unit_price: 50_000.0,
            quantity: 2.0,
            discount_percent: None,
            tax_percent: None,
            currency: "VND",
        }),
        LinePrice {
            gross: 100_000.0,
            discount_total: 0.0,
            tax_total: 0.0,
            line_total: 100_000.0,
        }
    );
}

/// Port of: packages/utils/__tests__/money.test.ts:64 "computeLinePrice > rounds each part once so the parts add up to the total"
#[test]
fn computelineprice_rounds_each_part_once_so_the_parts_add_up_to_the_total() {
    let line = compute_line_price(&LinePriceInput {
        unit_price: 0.335,
        quantity: 3.0,
        discount_percent: Some(33.0),
        tax_percent: Some(7.0),
        currency: "USD",
    });
    assert_eq!(
        line.line_total,
        round_money(line.gross - line.discount_total + line.tax_total, "USD")
    );
}

/// Port of: packages/utils/__tests__/money.test.ts:79 "sumMoney and percentOf > sums without floating-point drift"
#[test]
fn summoney_and_percentof_sums_without_floating_point_drift() {
    assert_eq!(sum_money(&[0.1, 0.2], "USD"), 0.3);
}

/// Port of: packages/utils/__tests__/money.test.ts:83 "sumMoney and percentOf > takes a rounded percentage"
#[test]
fn summoney_and_percentof_takes_a_rounded_percentage() {
    assert_eq!(percent_of(99.99, 5.0, "USD"), 5.0);
}

/// Port of: packages/utils/__tests__/money.test.ts:89 "formatMoney > puts the symbol left or right, with or without a space"
#[test]
fn formatmoney_puts_the_symbol_left_or_right_with_or_without_a_space() {
    assert_eq!(
        format_money(1234.5, &opts("USD", "$", SymbolPosition::Left)),
        "$1,234.50"
    );
    assert_eq!(
        format_money(5.0, &opts("USD", "$", SymbolPosition::LeftSpace)),
        "$ 5.00"
    );
    assert_eq!(
        format_money(125_000.0, &opts("VND", "₫", SymbolPosition::Right)),
        "125,000₫"
    );
    assert_eq!(
        format_money(125_000.0, &opts("VND", "₫", SymbolPosition::RightSpace)),
        "125,000 ₫"
    );
}

/// Port of: packages/utils/__tests__/money.test.ts:108 "formatMoney > falls back to the currency code without a symbol"
#[test]
fn formatmoney_falls_back_to_the_currency_code_without_a_symbol() {
    assert_eq!(
        format_money(
            5.0,
            &FormatMoneyOptions {
                currency: "eur",
                ..FormatMoneyOptions::default()
            }
        ),
        "EUR5.00"
    );
}

/// Port of: packages/utils/__tests__/money.test.ts:112 "formatMoney > puts the minus sign before the whole amount, in every position"
#[test]
fn formatmoney_puts_the_minus_sign_before_the_whole_amount_in_every_position() {
    assert_eq!(
        format_money(-6.9, &opts("USD", "$", SymbolPosition::Left)),
        "-$6.90"
    );
    assert_eq!(
        format_money(-6.9, &opts("USD", "$", SymbolPosition::LeftSpace)),
        "-$ 6.90"
    );
    assert_eq!(
        format_money(-125_000.0, &opts("VND", "₫", SymbolPosition::Right)),
        "-125,000₫"
    );
    assert_eq!(
        format_money(-125_000.0, &opts("VND", "₫", SymbolPosition::RightSpace)),
        "-125,000 ₫"
    );
    assert_eq!(
        format_money(
            -1234.5,
            &FormatMoneyOptions {
                currency: "eur",
                ..FormatMoneyOptions::default()
            }
        ),
        "-EUR1,234.50"
    );
    assert_eq!(
        format_money(-1500.0, &opts("JPY", "¥", SymbolPosition::Left)),
        "-¥1,500"
    );
    assert_eq!(
        format_money(-12.345, &opts("KWD", "KD", SymbolPosition::LeftSpace)),
        "-KD 12.345"
    );
}

/// Port of: packages/utils/__tests__/money.test.ts:150 "formatMoney > shows no sign for an amount that rounds to zero"
#[test]
fn formatmoney_shows_no_sign_for_an_amount_that_rounds_to_zero() {
    assert_eq!(
        format_money(-0.001, &opts("USD", "$", SymbolPosition::Left)),
        "$0.00"
    );
    assert_eq!(
        format_money(-0.4, &opts("VND", "₫", SymbolPosition::Right)),
        "0₫"
    );
}
