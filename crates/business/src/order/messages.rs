//! `order/customer-messages.ts` + `customer-message-strings.ts`: the texts
//! the bot sends a customer about their order, in the workspace language.
//!
//! The strings (`customer_message_strings.json`) and the `Intl` date shapes
//! (`date_formats.json`, Node 20 ICU) are generated from the TypeScript.

use std::collections::HashMap;
use std::sync::LazyLock;

use chatbotx_core::utils::money::{FormatMoneyOptions, SymbolPosition, format_money};
use chrono::{Datelike, Offset, TimeZone, Timelike};
use serde::Deserialize;
use serde_json::Value;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Strings {
    pub thanks: String,
    pub name: String,
    pub email: String,
    pub phone: String,
    pub address: String,
    pub pickup: String,
    pub date: String,
    pub payment: String,
    pub cod: String,
    pub card: String,
    pub paid: String,
    pub refunded: String,
    pub payment_expired: String,
    pub total: String,
    pub subtotal: String,
    pub discount: String,
    pub coupon: String,
    pub delivery: String,
    pub other_fees: String,
    pub items: String,
    pub number_of_items: String,
    pub check_out: String,
    pub skipped_one: String,
    pub skipped_many: String,
    pub status_updated: String,
    pub status: String,
    pub statuses: HashMap<String, String>,
    pub collection: HashMap<String, String>,
    pub product_search: HashMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DateFormat {
    pattern: String,
    months: Vec<String>,
    h12: bool,
    am: String,
    pm: String,
    arab_digits: bool,
    tz_prefix: String,
    tz_minus: String,
    tz_sep: String,
    lrm: bool,
}

static STRINGS: LazyLock<HashMap<String, Strings>> = LazyLock::new(|| {
    serde_json::from_str(include_str!("customer_message_strings.json"))
        .expect("customer message strings")
});
static DATES: LazyLock<HashMap<String, DateFormat>> = LazyLock::new(|| {
    serde_json::from_str(include_str!("date_formats.json")).expect("date formats")
});

/// `customerMessageLanguage`: a language the messages are not written in
/// reads as English.
pub fn language(language: Option<&str>) -> &'static str {
    match language.and_then(|l| STRINGS.get_key_value(l)) {
        Some((key, _)) => key.as_str(),
        None => "en",
    }
}

/// `customerMessageStrings`.
pub fn strings(language: &str) -> &'static Strings {
    STRINGS.get(language).unwrap_or_else(|| &STRINGS["en"])
}

pub fn skipped_items(strings: &Strings, count: usize) -> String {
    if count == 1 {
        strings.skipped_one.clone()
    } else {
        strings.skipped_many.replace("{count}", &count.to_string())
    }
}

/// The store's money settings the texts format amounts with.
pub struct Payments<'a> {
    pub currency: &'a str,
    pub symbol: Option<&'a str>,
    pub position: &'a str,
}

impl<'a> Payments<'a> {
    /// From `ecommerceSettings.payments`.
    pub fn of(payments: &'a Value) -> Payments<'a> {
        Payments {
            currency: payments["currency"].as_str().unwrap_or("USD"),
            symbol: payments["currencySymbol"].as_str(),
            position: payments["currencyPosition"].as_str().unwrap_or("left"),
        }
    }
}

fn position(text: &str) -> SymbolPosition {
    match text {
        "leftSpace" => SymbolPosition::LeftSpace,
        "right" => SymbolPosition::Right,
        "rightSpace" => SymbolPosition::RightSpace,
        _ => SymbolPosition::Left,
    }
}

/// `formatMoney` in the store symbol when `currency` is the store's.
pub fn money(amount: f64, currency: &str, payments: &Payments<'_>) -> String {
    let store = currency.to_uppercase() == payments.currency;
    format_money(
        amount,
        &FormatMoneyOptions {
            currency,
            symbol: if store { payments.symbol } else { None },
            position: if store {
                position(payments.position)
            } else {
                SymbolPosition::Left
            },
        },
    )
}

/// `paymentMethodLabel`.
pub fn payment_method_label(method: &str, strings: &Strings) -> String {
    match method {
        "stripe" => strings.card.clone(),
        "paypal" => "PayPal".into(),
        _ => strings.cod.clone(),
    }
}

/// What a confirmation shows of an order (major units).
pub struct Summary<'a> {
    pub number: i64,
    pub currency: &'a str,
    pub subtotal: f64,
    pub discount_total: f64,
    pub shipping_total: f64,
    pub service_fee_total: f64,
    pub tax_total: f64,
    pub coupon_discount: f64,
    pub discount_code: Option<&'a str>,
    pub total: f64,
    pub customer_name: Option<&'a str>,
    pub customer_email: Option<&'a str>,
    pub customer_phone: Option<&'a str>,
    pub shipping_type: Option<&'a str>,
    pub shipping_address: Option<&'a Value>,
    pub payment_method: &'a str,
    pub payment_status: &'a str,
    pub created_at: i64,
    /// `(name, variantLabel, quantity)`.
    pub items: Vec<(&'a str, Option<&'a str>, i64)>,
}

fn digits(text: String, arab: bool) -> String {
    if !arab {
        return text;
    }
    text.chars()
        .map(|c| match c.to_digit(10) {
            Some(d) => char::from_u32(0x0660 + d).unwrap_or(c),
            None => c,
        })
        .collect()
}

/// `Intl.DateTimeFormat(lang, { day, month: long, year: 2-digit, hour,
/// minute: 2-digit, timeZoneName: shortOffset })`; English is `en-GB`.
pub fn format_date(at_ms: i64, language: &str, time_zone: &str) -> String {
    let format = DATES.get(language).unwrap_or_else(|| &DATES["en"]);
    let tz: chrono_tz::Tz = time_zone.parse().unwrap_or(chrono_tz::UTC);
    let Some(local) = tz.timestamp_millis_opt(at_ms).single() else {
        return String::new();
    };
    let offset = local.offset().fix().local_minus_utc() / 60;
    let zone = if offset == 0 {
        format.tz_prefix.clone()
    } else {
        let (sign, minutes) = if offset < 0 {
            (format.tz_minus.as_str(), -offset)
        } else {
            ("+", offset)
        };
        let mut text = (minutes / 60).to_string();
        if minutes % 60 != 0 {
            text = format!("{text}{}{:02}", format.tz_sep, minutes % 60);
        }
        if format.lrm {
            // ICU wraps the offset in left-to-right marks for Hebrew.
            if sign == "+" {
                format!("{}\u{200e}+{text}\u{200e}", format.tz_prefix)
            } else {
                format!("{}{sign}{text}\u{200e}", format.tz_prefix)
            }
        } else {
            format!("{}{sign}{text}", format.tz_prefix)
        }
    };
    let hour = local.hour();
    let (shown, period) = if format.h12 {
        let h = if hour % 12 == 0 { 12 } else { hour % 12 };
        (h, if hour < 12 { &format.am } else { &format.pm })
    } else {
        (hour, &format.am)
    };
    let arab = format.arab_digits;
    format
        .pattern
        .replace("{d}", &digits(local.day().to_string(), arab))
        .replace("{M}", &format.months[local.month0() as usize])
        .replace("{y}", &digits(format!("{:02}", local.year() % 100), arab))
        .replace("{H}", &digits(format!("{shown:02}"), arab))
        .replace("{m}", &digits(format!("{:02}", local.minute()), arab))
        .replace("{a}", period)
        .replace("{z}", &digits(zone, arab))
}

fn address(order: &Summary<'_>, strings: &Strings, store_address: Option<&str>) -> String {
    if order.shipping_type == Some("pickup") {
        return [
            Some(strings.pickup.as_str()),
            store_address.filter(|a| !a.is_empty()),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(": ");
    }
    let address = order.shipping_address;
    ["country", "state", "city", "line1", "line2"]
        .iter()
        .filter_map(|key| {
            address
                .and_then(|a| a[*key].as_str())
                .filter(|v| !v.is_empty())
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// `buildOrderConfirmationText`.
pub fn confirmation_text(
    order: &Summary<'_>,
    language: &str,
    payments: &Payments<'_>,
    store_address: Option<&str>,
    time_zone: &str,
) -> String {
    let s = strings(language);
    let m = |amount: f64| money(amount, order.currency, payments);
    let label = payment_method_label(order.payment_method, s);
    let payment = if order.payment_status == "paid" {
        format!("{label} ({})", s.paid)
    } else {
        label
    };
    let other_fees = order.service_fee_total + order.tax_total;
    let mut lines = vec![
        s.thanks
            .replace("{name}", order.customer_name.unwrap_or_default()),
        String::new(),
        format!("--- #{} ---", order.number),
        String::new(),
        format!("{}: {}", s.name, order.customer_name.unwrap_or_default()),
        format!("{}: {}", s.email, order.customer_email.unwrap_or_default()),
        format!("{}: {}", s.phone, order.customer_phone.unwrap_or_default()),
        String::new(),
        format!("{}: {}", s.address, address(order, s, store_address)),
        String::new(),
        format!(
            "{}: {}",
            s.date,
            format_date(order.created_at, language, time_zone)
        ),
        String::new(),
        format!("{}: {payment}", s.payment),
        String::new(),
        format!("{}: {}", s.total, m(order.total)),
        format!("{}: {}", s.subtotal, m(order.subtotal)),
    ];
    if order.discount_total > 0.0 {
        lines.push(format!("{}: {}", s.discount, m(-order.discount_total)));
    }
    lines.push(format!("{}: {}", s.delivery, m(order.shipping_total)));
    if other_fees > 0.0 {
        lines.push(format!("{}: {}", s.other_fees, m(other_fees)));
    }
    if order.coupon_discount > 0.0 {
        lines.push(format!(
            "{}: {}",
            s.coupon
                .replace("{code}", order.discount_code.unwrap_or_default()),
            m(-order.coupon_discount)
        ));
    }
    lines.push(String::new());
    lines.push(format!("{}:", s.items));
    for (name, variant, quantity) in &order.items {
        lines.push(match variant {
            Some(v) if !v.is_empty() => format!("{quantity} X {name} ({v})"),
            _ => format!("{quantity} X {name}"),
        });
    }
    lines.join("\n")
}

/// `buildOrderStatusText`.
pub fn status_text(
    status: &str,
    message: Option<&str>,
    refunded: bool,
    payment_expired: bool,
    language: &str,
) -> String {
    let s = strings(language);
    let mut lines = vec![
        s.status_updated.clone(),
        String::new(),
        format!(
            "{}: {}",
            s.status,
            s.statuses.get(status).cloned().unwrap_or_default()
        ),
    ];
    if payment_expired {
        lines.extend([String::new(), s.payment_expired.clone()]);
    }
    if refunded {
        lines.extend([String::new(), s.refunded.clone()]);
    }
    if let Some(message) = message.filter(|m| !m.is_empty()) {
        lines.extend([String::new(), message.to_string()]);
    }
    lines.join("\n")
}

/// `CartSummary` (major units).
pub struct CartSummary<'a> {
    pub item_count: i64,
    pub currency: &'a str,
    pub subtotal: f64,
    pub discount_total: f64,
    pub shipping_total: f64,
    pub service_fee_total: f64,
    pub tax_total: f64,
    pub total: f64,
    pub discount: Option<(&'a str, f64)>,
    pub skipped: usize,
}

/// `buildCartSummaryText`.
pub fn cart_summary_text(
    cart: &CartSummary<'_>,
    language: &str,
    payments: &Payments<'_>,
) -> String {
    let s = strings(language);
    let m = |amount: f64| money(amount, cart.currency, payments);
    let mut lines = vec![
        format!("{}: {}", s.number_of_items, cart.item_count),
        format!("{}: {}", s.subtotal, m(cart.subtotal)),
    ];
    if cart.discount_total > 0.0 {
        lines.push(format!("{}: {}", s.discount, m(-cart.discount_total)));
    }
    lines.push(format!("{}: {}", s.delivery, m(cart.shipping_total)));
    lines.push(format!(
        "{}: {}",
        s.other_fees,
        m(cart.service_fee_total + cart.tax_total)
    ));
    if let Some((code, amount)) = cart.discount {
        lines.push(format!(
            "{}: {}",
            s.coupon.replace("{code}", code),
            m(-amount)
        ));
    }
    lines.push(format!("{}: {}", s.total, m(cart.total)));
    if cart.skipped > 0 {
        lines.extend([String::new(), skipped_items(s, cart.skipped)]);
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_language_loads_and_falls_back_to_english() {
        assert_eq!(STRINGS.len(), 21);
        assert_eq!(DATES.len(), 21);
        assert_eq!(language(Some("de")), "de");
        assert_eq!(language(Some("xx")), "en");
        assert_eq!(language(None), "en");
        assert_eq!(
            skipped_items(strings("en"), 1),
            "1 item is not sold in this store and was left out."
        );
        assert_eq!(
            skipped_items(strings("en"), 3),
            "3 items are not sold in this store and were left out."
        );
    }

    #[test]
    fn dates_match_intl() {
        // 2026-04-07T03:04Z.
        let at = 1_775_531_040_000;
        assert_eq!(
            format_date(at, "en", "Asia/Kolkata"),
            "7 April 26 at 08:34 GMT+5:30"
        );
        assert_eq!(format_date(at, "en", "UTC"), "7 April 26 at 03:04 GMT");
        assert_eq!(
            format_date(at, "de", "America/New_York"),
            "6. April 26 um 23:04 GMT-4"
        );
        assert_eq!(
            format_date(at, "ja", "Asia/Tokyo"),
            "26年4月7日 12:04 GMT+9"
        );
        assert_eq!(
            format_date(at, "ar", "Asia/Kolkata"),
            "٧ أبريل ٢٦ في ٠٨:٣٤ ص غرينتش+٥:٣٠"
        );
    }

    #[test]
    fn status_text_lines() {
        assert_eq!(
            status_text("cancelled", Some("Sorry"), true, false, "en"),
            "The status of your order has been updated.\n\nStatus: Cancelled\n\nYour payment has been refunded.\n\nSorry"
        );
    }
}
