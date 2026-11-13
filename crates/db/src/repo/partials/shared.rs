//! Port of `partials/shared.ts` (`roundToMoneyScale`).

/// `MONEY_SCALE`: the scale of a `money()` (`numeric(19,4)`) column.
pub const MONEY_SCALE: usize = 4;

/// `roundToMoneyScale`: `amount` rounded the way a `money()` column stores it: half away
/// from zero on the decimal digits the number is written as (`1.12345` -> `1.1235`),
/// never on the binary float.
pub fn round_to_money_scale(amount: f64) -> f64 {
    let _ = amount;
    todo!("P3.2b")
}
