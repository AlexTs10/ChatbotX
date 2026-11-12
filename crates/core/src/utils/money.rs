//! `packages/utils/src/money.ts`. Amounts are major units (`12.5` USD) in
//! `f64`, rounded half away from zero to the currency's unit.

const DEFAULT_FRACTION_DIGITS: u32 = 2;

/// Currencies whose `Intl` (ICU 78, Node 24) fraction digits are not 2.
const FRACTION_DIGITS: &[(&str, u32)] = &[
    ("ADP", 0),
    ("AFN", 0),
    ("ALL", 0),
    ("BHD", 3),
    ("BIF", 0),
    ("BYR", 0),
    ("CLF", 4),
    ("CLP", 0),
    ("COP", 0),
    ("DJF", 0),
    ("ESP", 0),
    ("GNF", 0),
    ("HUF", 0),
    ("IDR", 0),
    ("IQD", 0),
    ("IRR", 0),
    ("ISK", 0),
    ("ITL", 0),
    ("JOD", 3),
    ("JPY", 0),
    ("KMF", 0),
    ("KPW", 0),
    ("KRW", 0),
    ("KWD", 3),
    ("LAK", 0),
    ("LBP", 0),
    ("LUF", 0),
    ("LYD", 3),
    ("MGA", 0),
    ("MGF", 0),
    ("MMK", 0),
    ("MRO", 0),
    ("OMR", 3),
    ("PKR", 0),
    ("PYG", 0),
    ("RWF", 0),
    ("SLL", 0),
    ("SOS", 0),
    ("STD", 0),
    ("SYP", 0),
    ("TMM", 0),
    ("TND", 3),
    ("TRL", 0),
    ("UGX", 0),
    ("UYI", 0),
    ("UYW", 4),
    ("VND", 0),
    ("VUV", 0),
    ("XAF", 0),
    ("XOF", 0),
    ("XPF", 0),
    ("YER", 0),
    ("ZMK", 0),
    ("ZWD", 0),
];

/// `getCurrencyFractionDigits`: 0 for JPY, 2 for USD, 3 for KWD; 2 for an
/// unknown or malformed code.
pub fn get_currency_fraction_digits(currency: &str) -> u32 {
    let code = crate::js::trim(currency).to_uppercase();
    FRACTION_DIGITS
        .iter()
        .find(|(c, _)| *c == code)
        .map_or(DEFAULT_FRACTION_DIGITS, |(_, d)| *d)
}

/// `roundMoney`: half away from zero, nudged by an epsilon scaled to the
/// value so a written-down half (`1.005`) rounds up.
pub fn round_money(amount: f64, currency: &str) -> f64 {
    let factor = 10f64.powi(get_currency_fraction_digits(currency) as i32);
    let scaled = amount.abs() * factor;
    let rounded = (scaled + scaled * f64::EPSILON).round() / factor;
    if amount < 0.0 { -rounded } else { rounded }
}

/// `sumMoney`.
pub fn sum_money(amounts: &[f64], currency: &str) -> f64 {
    round_money(amounts.iter().fold(0.0, |total, a| total + a), currency)
}

/// `percentOf`: `percent` (0–100) of `amount`, rounded.
pub fn percent_of(amount: f64, percent: f64, currency: &str) -> f64 {
    round_money((amount * percent) / 100.0, currency)
}

/// `LinePriceInput`.
#[derive(Debug, Clone, PartialEq)]
pub struct LinePriceInput<'a> {
    pub unit_price: f64,
    pub quantity: f64,
    /// 0–100; `None` = 0.
    pub discount_percent: Option<f64>,
    /// 0–100, after the discount; `None` = 0.
    pub tax_percent: Option<f64>,
    pub currency: &'a str,
}

/// `LinePrice`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LinePrice {
    pub gross: f64,
    pub discount_total: f64,
    pub tax_total: f64,
    pub line_total: f64,
}

/// `computeLinePrice`: each part rounded once.
pub fn compute_line_price(input: &LinePriceInput<'_>) -> LinePrice {
    let gross = round_money(input.unit_price * input.quantity, input.currency);
    let discount_total = percent_of(gross, input.discount_percent.unwrap_or(0.0), input.currency);
    let tax_total = percent_of(
        gross - discount_total,
        input.tax_percent.unwrap_or(0.0),
        input.currency,
    );
    LinePrice {
        gross,
        discount_total,
        tax_total,
        line_total: round_money(gross - discount_total + tax_total, input.currency),
    }
}

/// Where `formatMoney` puts the symbol.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SymbolPosition {
    #[default]
    Left,
    LeftSpace,
    Right,
    RightSpace,
}

/// `FormatMoneyOptions`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FormatMoneyOptions<'a> {
    pub currency: &'a str,
    /// Shown instead of the ISO code when set (and not empty).
    pub symbol: Option<&'a str>,
    pub position: SymbolPosition,
}

/// `Intl.NumberFormat("en", { minimumFractionDigits: d, maximumFractionDigits: d })`.
pub fn format_grouped(value: f64, digits: u32) -> String {
    let fixed = format!("{:.*}", digits as usize, value.abs());
    let (int, frac) = fixed.split_once('.').unwrap_or((fixed.as_str(), ""));
    let mut grouped = String::new();
    for (i, c) in int.chars().enumerate() {
        if i > 0 && (int.len() - i) % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(c);
    }
    let sign = if value < 0.0 && fixed.chars().any(|c| c.is_ascii_digit() && c != '0') {
        "-"
    } else {
        ""
    };
    if frac.is_empty() {
        format!("{sign}{grouped}")
    } else {
        format!("{sign}{grouped}.{frac}")
    }
}

/// `formatMoney`: `$12.50`, `$ 12.50`, `12,500₫`, `12,500 ₫`; the sign
/// leads the whole amount, and an amount that rounds to zero has none.
pub fn format_money(amount: f64, options: &FormatMoneyOptions<'_>) -> String {
    let digits = get_currency_fraction_digits(options.currency);
    let rounded = round_money(amount, options.currency);
    let number = format_grouped(rounded.abs(), digits);
    let symbol = match options.symbol {
        Some(s) if !s.is_empty() => s.to_owned(),
        _ => options.currency.to_uppercase(),
    };
    let sign = if rounded < 0.0 { "-" } else { "" };
    match options.position {
        SymbolPosition::LeftSpace => format!("{sign}{symbol} {number}"),
        SymbolPosition::Right => format!("{sign}{number}{symbol}"),
        SymbolPosition::RightSpace => format!("{sign}{number} {symbol}"),
        SymbolPosition::Left => format!("{sign}{symbol}{number}"),
    }
}

/// `stackDiscountPercents`: 10% then 20% is 28% off.
pub fn stack_discount_percents(product_percent: f64, vendor_percent: f64) -> f64 {
    100.0 - ((100.0 - product_percent) * (100.0 - vendor_percent)) / 100.0
}
