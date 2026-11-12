//! `packages/utils/src/coordinates.ts`.

use std::sync::OnceLock;

use regex::Regex;

/// `COORDINATE_PAIR_PATTERN` (ASCII digits, as in JavaScript).
pub const COORDINATE_PAIR_PATTERN: &str =
    r"^\s*(-?[0-9]+(?:\.[0-9]+)?)\s*,\s*(-?[0-9]+(?:\.[0-9]+)?)\s*$";

/// A parsed `lat,lng` pair.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CoordinatePair {
    pub latitude: f64,
    pub longitude: f64,
}

/// `parseCoordinatePair`: full precision; `None` when not a pair or out of
/// range.
pub fn parse_coordinate_pair(text: &str) -> Option<CoordinatePair> {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| Regex::new(COORDINATE_PAIR_PATTERN).expect("valid regex"));
    let caps = re.captures(text)?;
    let latitude: f64 = caps[1].parse().ok()?;
    let longitude: f64 = caps[2].parse().ok()?;
    if latitude.abs() > 90.0 || longitude.abs() > 180.0 {
        return None;
    }
    Some(CoordinatePair {
        latitude,
        longitude,
    })
}
