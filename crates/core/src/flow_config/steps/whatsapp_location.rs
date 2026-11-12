//! Port of `packages/flow-config/src/steps/whatsapp-location.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

pub const WHATSAPP_LOCATION_TEXT_MAX: usize = 1000;

pub const WHATSAPP_LOCATION_LATITUDE_LIMIT: usize = 90;

pub const WHATSAPP_LOCATION_LONGITUDE_LIMIT: usize = 180;

schema!(pub fn whatsapp_location_step_schema = z::object(vec![("id", z::string().regex(r#"^\d+$"#, "").describe("Step id (numeric string), unique within the flow.")),
("nodeId", z::string().optional().describe("Id of the node containing this step. Optional.")),
("stepType", z::literal(json!("whatsappLocation")).describe("Step type discriminator: \"whatsappLocation\".")),
("name", z::string().trim().max(1000).optional().describe("Optional place name shown on the map pin.")),
("address", z::string().trim().max(1000).optional().describe("Optional street address shown under the name.")),
("latitude", z::number().gte(-90_f64).lte(90_f64).describe("Latitude in decimal degrees, from -90 to 90.")),
("longitude", z::number().gte(-180_f64).lte(180_f64).describe("Longitude in decimal degrees, from -180 to 180."))]).refine(whatsapp_location_step_schema_refine, Some("Set a location"), Some(&["latitude"])));

pub fn whatsapp_location_form_errors() -> &'static Value {
    static V: LazyLock<Value> = LazyLock::new(|| {
        serde_json::from_str(r#"{"tooLong":"tooLong","invalidLatitude":"invalidLatitude","invalidLongitude":"invalidLongitude","locationRequired":"locationRequired"}"#).expect("valid JSON")
    });
    &V
}

schema!(pub fn whatsapp_location_form_schema = z::object(vec![("name", z::string().trim().max(1000).msg("tooLong")),
("address", z::string().trim().max(1000).msg("tooLong")),
("latitude", z::string().trim().refine(whatsapp_location_form_schema_latitude_refine, Some("invalidLatitude"), None)),
("longitude", z::string().trim().refine(whatsapp_location_form_schema_longitude_refine, Some("invalidLongitude"), None))]).refine(whatsapp_location_form_schema_refine, Some("locationRequired"), Some(&["latitude"])));

/// `whatsappLocationStepDefaultFn(props)`.
pub fn whatsapp_location_step_default_fn(props: &Value) -> Value {
    spread3(
        json!({"name": "", "address": "", "latitude": 0, "longitude": 0}),
        props,
        json!({"id": create_id(), "stepType": "whatsappLocation"}),
    )
}

/// `isValidWhatsappLocationCoordinate(value, limit)`.
pub fn is_valid_whatsapp_location_coordinate(value: f64, limit: f64) -> bool {
    value.is_finite() && value.abs() <= limit
}

/// `isWhatsappLocationUnset({ latitude, longitude })`: exactly 0, 0.
pub fn is_whatsapp_location_unset(location: &Value) -> bool {
    location["latitude"].as_f64() == Some(0.0) && location["longitude"].as_f64() == Some(0.0)
}

fn coordinate_is_zero(value: f64) -> bool {
    value == 0.0
}

/// `formatWhatsappLocationCoordinate`: `String(value)` without exponent
/// notation.
pub fn format_whatsapp_location_coordinate(value: f64) -> String {
    let text = crate::js::number_to_string(value);
    static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let re = RE.get_or_init(|| {
        regex::Regex::new(r"^(-?)([0-9])(?:\.([0-9]+))?e([+-][0-9]+)$").expect("exp regex")
    });
    let Some(m) = re.captures(&text) else {
        return text;
    };
    let sign = m.get(1).map_or("", |x| x.as_str());
    let leading = m.get(2).map_or("", |x| x.as_str());
    let fraction = m.get(3).map_or("", |x| x.as_str());
    let exponent: i64 = m.get(4).map_or("0", |x| x.as_str()).parse().unwrap_or(0);
    let digits = format!("{leading}{fraction}");
    let point = 1 + exponent;
    if point <= 0 {
        return format!("{sign}0.{}{digits}", "0".repeat((-point) as usize));
    }
    let point = point as usize;
    if digits.len() <= point {
        format!("{sign}{digits:0<point$}")
    } else {
        format!("{sign}{}.{}", &digits[..point], &digits[point..])
    }
}

/// `parseWhatsappLocationCoordinate`: decimal (`.` or `,`), else NaN (`None`
/// here stands for NaN).
pub fn parse_whatsapp_location_coordinate(text: &str) -> Option<f64> {
    static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let re =
        RE.get_or_init(|| regex::Regex::new(r"^-?[0-9]+(?:[.,][0-9]+)?$").expect("coord regex"));
    let trimmed = crate::js::trim(text);
    if !re.is_match(trimmed) {
        return None;
    }
    Some(crate::js::to_number(&trimmed.replacen(',', ".", 1)))
}

fn parsed_or_nan(value: &Value) -> f64 {
    parse_whatsapp_location_coordinate(value.as_str().unwrap_or("")).unwrap_or(f64::NAN)
}

fn whatsapp_location_step_schema_refine(step: &Value) -> bool {
    !is_whatsapp_location_unset(step)
}

fn whatsapp_location_form_schema_latitude_refine(value: &Value) -> bool {
    is_valid_whatsapp_location_coordinate(
        parsed_or_nan(value),
        WHATSAPP_LOCATION_LATITUDE_LIMIT as f64,
    )
}

fn whatsapp_location_form_schema_longitude_refine(value: &Value) -> bool {
    is_valid_whatsapp_location_coordinate(
        parsed_or_nan(value),
        WHATSAPP_LOCATION_LONGITUDE_LIMIT as f64,
    )
}

fn whatsapp_location_form_schema_refine(values: &Value) -> bool {
    !(coordinate_is_zero(parsed_or_nan(&values["latitude"]))
        && coordinate_is_zero(parsed_or_nan(&values["longitude"])))
}
