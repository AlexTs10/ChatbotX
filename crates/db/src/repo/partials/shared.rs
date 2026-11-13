//! Port of `partials/shared.ts` (`roundToMoneyScale`).

/// `MONEY_SCALE`: the scale of a `money()` (`numeric(19,4)`) column.
pub const MONEY_SCALE: usize = 4;

/// `roundToMoneyScale`: `amount` rounded the way a `money()` column stores it: half away
/// from zero on the decimal digits the number is written as (`1.12345` -> `1.1235`),
/// never on the binary float.
pub fn round_to_money_scale(amount: f64) -> f64 {
    let abs = amount.abs();
    // JS `String` writes exponent notation only below 1e-6 (rounds to zero) or at 1e21
    // and above (no fraction left to round); Rust `Display` never does.
    if abs > 0.0 && abs < 1e-6 {
        return 0.0;
    }
    if abs >= 1e21 {
        return amount;
    }
    let text = abs.to_string();
    let (whole, fraction) = text.split_once('.').unwrap_or((text.as_str(), ""));
    if fraction.len() <= MONEY_SCALE {
        return amount;
    }
    let Ok(kept) = format!("{whole}{}", &fraction[..MONEY_SCALE]).parse::<u128>() else {
        return amount;
    };
    let scaled = if fraction.as_bytes()[MONEY_SCALE] >= b'5' {
        kept + 1
    } else {
        kept
    };
    let factor = 10_u128.pow(4);
    let Ok(rounded) = format!("{}.{:04}", scaled / factor, scaled % factor).parse::<f64>() else {
        return amount;
    };
    // Never -0: a negative amount that rounds to nothing is zero.
    if amount < 0.0 && rounded > 0.0 {
        -rounded
    } else {
        rounded
    }
}
