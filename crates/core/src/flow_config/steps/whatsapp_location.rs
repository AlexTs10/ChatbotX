//! Port of `packages/flow-config/src/steps/whatsapp-location.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

pub const WHATSAPP_LOCATION_TEXT_MAX: usize = 1000;

pub const WHATSAPP_LOCATION_LATITUDE_LIMIT: usize = 90;

pub const WHATSAPP_LOCATION_LONGITUDE_LIMIT: usize = 180;

// HAND-PORT isValidWhatsappLocationCoordinate:
// (value,limit)=>Number.isFinite(value)&&Math.abs(value)<=limit

// HAND-PORT isWhatsappLocationUnset:
// ({latitude,longitude})=>latitude===0&&longitude===0

// HAND-PORT formatWhatsappLocationCoordinate:
// value=>{const text=String(value);const match=EXPONENT_NOTATION_PATTERN.exec(text);if(!match){return text}const[,sign="",leading="",fraction="",exponentText="0"]=match;const digits=leading+fraction;const pointIndex=1+Number(exponentText);if(pointIndex<=0){return`${sign}0.${"0".repeat(-pointIndex)}${digits}`}return digits.length<=pointIndex?`${sign}${digits.padEnd(pointIndex,"0")}`:`${sign}${digits.slice(0,pointIndex)}.${digits.slice(pointIndex)}`}

// HAND-PORT parseWhatsappLocationCoordinate:
// text=>DECIMAL_COORDINATE_PATTERN.test(text.trim())?Number(text.trim().replace(",",".")):Number.NaN

schema!(pub fn whatsapp_location_step_schema = z::object(vec![("id", z::string().regex(r#"^\d+$"#, "").describe("Step id (numeric string), unique within the flow.")),
("nodeId", z::string().optional().describe("Id of the node containing this step. Optional.")),
("stepType", z::literal(json!("whatsappLocation")).describe("Step type discriminator: \"whatsappLocation\".")),
("name", z::string().trim().max(1000).optional().describe("Optional place name shown on the map pin.")),
("address", z::string().trim().max(1000).optional().describe("Optional street address shown under the name.")),
("latitude", z::number().gte(-90 as f64).lte(90 as f64).describe("Latitude in decimal degrees, from -90 to 90.")),
("longitude", z::number().gte(-180 as f64).lte(180 as f64).describe("Longitude in decimal degrees, from -180 to 180."))]).refine(whatsapp_location_step_schema_refine, Some("Set a location"), Some(&["latitude"])));

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

// HAND-PORT whatsappLocationStepDefaultFn:
// (props={})=>({name:"",address:"",latitude:0,longitude:0,...props,id:createId(),stepType:stepTypes.enum.whatsappLocation})

// step=>!isWhatsappLocationUnset(step)
fn whatsapp_location_step_schema_refine(value: &Value) -> bool {
    todo!("P3.4")
}

// value=>isValidWhatsappLocationCoordinate(parseWhatsappLocationCoordinate(value),limit)
fn whatsapp_location_form_schema_latitude_refine(value: &Value) -> bool {
    todo!("P3.4")
}

// value=>isValidWhatsappLocationCoordinate(parseWhatsappLocationCoordinate(value),limit)
fn whatsapp_location_form_schema_longitude_refine(value: &Value) -> bool {
    todo!("P3.4")
}

// values=>!isWhatsappLocationUnset({latitude:parseWhatsappLocationCoordinate(values.latitude),longitude:parseWhatsappLocationCoordinate(values.longitude)})
fn whatsapp_location_form_schema_refine(value: &Value) -> bool {
    todo!("P3.4")
}

// ---- stage 1 API (hand-ported in stage 2) ----

pub fn whatsapp_location_step_default_fn(props: &Value) -> Value {
    let _ = props;
    todo!("P3.4")
}

pub fn format_whatsapp_location_coordinate(value: f64) -> String {
    todo!("P3.4")
}

pub fn is_whatsapp_location_unset(location: &Value) -> bool {
    todo!("P3.4")
}

pub fn parse_whatsapp_location_coordinate(text: &str) -> Option<f64> {
    todo!("P3.4")
}
