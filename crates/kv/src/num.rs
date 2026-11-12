//! Number rules of the original stack: Lua 5.1 `tonumber` (Redis scripts),
//! JS `Number(text)` / `String(number)`, and the Redis integer parser
//! (`INCRBY`, `HINCRBY`).

/// Lua 5.1 `tonumber(text)`: surrounding whitespace, decimal with optional
/// exponent, `0x` hex. `None` = Lua `nil`.
pub(crate) fn lua_tonumber(text: &str) -> Option<f64> {
    let t = text.trim_matches(|c: char| c.is_ascii_whitespace());
    if t.is_empty() {
        return None;
    }
    let (negative, body) = match t.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, t.strip_prefix('+').unwrap_or(t)),
    };
    if let Some(hex) = body.strip_prefix("0x").or_else(|| body.strip_prefix("0X")) {
        let n = u64::from_str_radix(hex, 16).ok()? as f64;
        return Some(if negative { -n } else { n });
    }
    t.parse::<f64>().ok()
}

/// The `toint` helper of the live-counter scripts: a number equal to its floor.
pub(crate) fn lua_toint(n: f64) -> Option<f64> {
    if !n.is_nan() && n == n.floor() {
        Some(n)
    } else {
        None
    }
}

/// `toint(tonumber(text))`.
pub(crate) fn lua_toint_text(text: &str) -> Option<f64> {
    lua_tonumber(text).and_then(lua_toint)
}

/// JS `Number(text)`; `None` when the result is not finite (`getNumber`).
pub(crate) fn js_number(text: &str) -> Option<f64> {
    let t = text.trim();
    if t.is_empty() {
        return Some(0.0);
    }
    let parsed = if let Some(hex) = t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")) {
        u64::from_str_radix(hex, 16).ok().map(|n| n as f64)
    } else if t.eq_ignore_ascii_case("inf")
        || t.eq_ignore_ascii_case("infinity")
        || t.eq_ignore_ascii_case("nan")
        || t.ends_with(|c: char| c.is_ascii_alphabetic())
    {
        // Rust accepts `inf`/`nan` spellings that JS reads as NaN; JS
        // `Infinity` is not finite either.
        None
    } else {
        t.parse::<f64>().ok()
    };
    parsed.filter(|n| n.is_finite())
}

/// JS `String(number)` for the values the stores write.
pub(crate) fn js_number_string(n: f64) -> String {
    if n.is_nan() {
        return "NaN".to_owned();
    }
    if n.is_infinite() {
        return if n > 0.0 { "Infinity" } else { "-Infinity" }.to_owned();
    }
    if n == n.trunc() && n.abs() < 1e21 {
        // Integral: no `.0` (and `-0` prints as `0`).
        return format!("{}", n as i128);
    }
    format!("{n}")
}

/// Redis `string2ll`: optional `-`, no `+`, no leading zeros, no spaces.
pub(crate) fn redis_int(text: &str) -> Option<i64> {
    let digits = text.strip_prefix('-').unwrap_or(text);
    let valid = match digits.as_bytes() {
        [] => false,
        [b'0'] => digits.len() == text.len(),
        [first, rest @ ..] => (b'1'..=b'9').contains(first) && rest.iter().all(u8::is_ascii_digit),
    };
    if valid {
        text.parse::<i64>().ok()
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lua_tonumber_follows_lua_5_1() {
        assert_eq!(lua_tonumber(" 12 "), Some(12.0));
        assert_eq!(lua_tonumber("1.5"), Some(1.5));
        assert_eq!(lua_tonumber("1e3"), Some(1000.0));
        assert_eq!(lua_tonumber("0x10"), Some(16.0));
        assert_eq!(lua_tonumber("-0x10"), Some(-16.0));
        assert_eq!(lua_tonumber("bad"), None);
        assert_eq!(lua_tonumber(""), None);
        assert_eq!(lua_toint_text("1.5"), None);
        assert_eq!(lua_toint_text("-2"), Some(-2.0));
        assert_eq!(lua_toint(f64::NAN), None);
    }

    #[test]
    fn js_number_and_string_follow_js() {
        assert_eq!(js_number(""), Some(0.0));
        assert_eq!(js_number(" 7 "), Some(7.0));
        assert_eq!(js_number("1.25"), Some(1.25));
        assert_eq!(js_number("\"x\""), None);
        assert_eq!(js_number("Infinity"), None);
        assert_eq!(js_number("inf"), None);
        assert_eq!(js_number_string(1.0), "1");
        assert_eq!(js_number_string(-0.0), "0");
        assert_eq!(js_number_string(1.5), "1.5");
        assert_eq!(js_number_string(1_760_000_000_250.0), "1760000000250");
    }

    #[test]
    fn redis_int_is_strict() {
        assert_eq!(redis_int("0"), Some(0));
        assert_eq!(redis_int("-12"), Some(-12));
        assert_eq!(redis_int("+1"), None);
        assert_eq!(redis_int("01"), None);
        assert_eq!(redis_int("-0"), None);
        assert_eq!(redis_int("1.0"), None);
        assert_eq!(redis_int(" 1"), None);
        assert_eq!(redis_int("99999999999999999999"), None);
    }
}
