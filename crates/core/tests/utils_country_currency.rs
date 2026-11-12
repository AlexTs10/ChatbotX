//! Port of `packages/utils/__tests__/country-currency.test.ts`.

use chatbotx_core::utils::country_currency::primary_currency_for_country;

/// Port of: packages/utils/__tests__/country-currency.test.ts:5 "primaryCurrencyForCountry > VN → VND"
#[test]
fn primarycurrencyforcountry_vn_vnd() {
    assert_eq!(primary_currency_for_country(Some("VN")), Some("VND"));
}

/// Port of: packages/utils/__tests__/country-currency.test.ts:5 "primaryCurrencyForCountry > JP → JPY"
#[test]
fn primarycurrencyforcountry_jp_jpy() {
    assert_eq!(primary_currency_for_country(Some("JP")), Some("JPY"));
}

/// Port of: packages/utils/__tests__/country-currency.test.ts:5 "primaryCurrencyForCountry > US → USD"
#[test]
fn primarycurrencyforcountry_us_usd() {
    assert_eq!(primary_currency_for_country(Some("US")), Some("USD"));
}

/// Port of: packages/utils/__tests__/country-currency.test.ts:5 "primaryCurrencyForCountry > vn → VND"
#[test]
fn primarycurrencyforcountry_vn_vnd_2() {
    assert_eq!(primary_currency_for_country(Some("vn")), Some("VND"));
}

/// Port of: packages/utils/__tests__/country-currency.test.ts:5 "primaryCurrencyForCountry >  jp  → JPY"
#[test]
fn primarycurrencyforcountry_jp_jpy_2() {
    assert_eq!(primary_currency_for_country(Some(" jp ")), Some("JPY"));
}

/// Port of: packages/utils/__tests__/country-currency.test.ts:15 "primaryCurrencyForCountry > takes the first of several currencies"
#[test]
fn primarycurrencyforcountry_takes_the_first_of_several_currencies() {
    assert_eq!(primary_currency_for_country(Some("CH")), Some("CHF"));
}

/// Port of: packages/utils/__tests__/country-currency.test.ts:19 "primaryCurrencyForCountry > unknown has no currency"
#[test]
fn primarycurrencyforcountry_unknown_has_no_currency() {
    assert_eq!(primary_currency_for_country(Some("unknown")), None);
}

/// Port of: packages/utils/__tests__/country-currency.test.ts:19 "primaryCurrencyForCountry > ZZ has no currency"
#[test]
fn primarycurrencyforcountry_zz_has_no_currency() {
    assert_eq!(primary_currency_for_country(Some("ZZ")), None);
}

/// Port of: packages/utils/__tests__/country-currency.test.ts:19 "primaryCurrencyForCountry >  has no currency"
#[test]
fn primarycurrencyforcountry_has_no_currency() {
    assert_eq!(primary_currency_for_country(Some("")), None);
}

/// Port of: packages/utils/__tests__/country-currency.test.ts:19 "primaryCurrencyForCountry > null has no currency"
#[test]
fn primarycurrencyforcountry_null_has_no_currency() {
    assert_eq!(primary_currency_for_country(None), None);
}

/// Port of: packages/utils/__tests__/country-currency.test.ts:19 "primaryCurrencyForCountry > undefined has no currency"
#[test]
fn primarycurrencyforcountry_undefined_has_no_currency() {
    assert_eq!(primary_currency_for_country(None), None);
}

/// Port of: packages/utils/__tests__/country-currency.test.ts:19 "primaryCurrencyForCountry > AQ has no currency"
#[test]
fn primarycurrencyforcountry_aq_has_no_currency() {
    assert_eq!(primary_currency_for_country(Some("AQ")), None);
}
