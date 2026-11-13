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

/// One zod issue: the failing path (`values.0`, empty for the root) and the message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ZodIssue {
    pub path: String,
    pub message: String,
}

/// A failed zod `safeParse` (`result.error`): the issues in the zod order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ZodError {
    pub issues: Vec<ZodIssue>,
}

impl ZodError {
    /// An error with one issue.
    pub fn one(path: impl Into<String>, message: impl Into<String>) -> ZodError {
        ZodError {
            issues: vec![ZodIssue {
                path: path.into(),
                message: message.into(),
            }],
        }
    }

    /// The issue messages (`result.error.issues.map((issue) => issue.message)`).
    pub fn messages(&self) -> Vec<&str> {
        self.issues.iter().map(|i| i.message.as_str()).collect()
    }
}

impl std::fmt::Display for ZodError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let parts: Vec<String> = self
            .issues
            .iter()
            .map(|i| {
                if i.path.is_empty() {
                    i.message.clone()
                } else {
                    format!("{}: {}", i.path, i.message)
                }
            })
            .collect();
        f.write_str(&parts.join("; "))
    }
}

impl std::error::Error for ZodError {}

impl From<ZodError> for crate::repo::Error {
    fn from(error: ZodError) -> crate::repo::Error {
        crate::repo::Error::msg(error.to_string())
    }
}

/// The serde half of a zod parse: a wrong or missing field is one issue at the root.
pub fn deserialize<T: serde::de::DeserializeOwned>(
    value: &serde_json::Value,
) -> std::result::Result<T, ZodError> {
    serde_json::from_value(value.clone()).map_err(|e| ZodError::one("", e.to_string()))
}

/// Collects zod issues in order; `finish` gives `Ok(value)` when there are none.
#[derive(Debug, Default)]
pub struct Issues(pub Vec<ZodIssue>);

impl Issues {
    /// `ctx.addIssue`.
    pub fn add(&mut self, path: impl Into<String>, message: impl Into<String>) {
        self.0.push(ZodIssue {
            path: path.into(),
            message: message.into(),
        });
    }

    /// `Ok(value)` without issues, else the error.
    pub fn finish<T>(self, value: T) -> std::result::Result<T, ZodError> {
        if self.0.is_empty() {
            Ok(value)
        } else {
            Err(ZodError { issues: self.0 })
        }
    }
}

/// The JS `string.length` (UTF-16 code units), which zod `min` / `max` count.
pub fn js_length(text: &str) -> usize {
    text.encode_utf16().count()
}
