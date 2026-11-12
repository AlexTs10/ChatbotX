//! Port of `packages/flow-config/src/steps/get-user-data.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

pub fn reply_format() -> &'static Value {
    static V: LazyLock<Value> = LazyLock::new(|| {
        serde_json::from_str(r#"{"number":"RF01","text":"RF02","email":"RF03","phone":"RF04","image":"RF05","file":"RF06","link":"RF07","location":"RF08","date":"RF09","datetime":"RF10","anyInput":"RF11"}"#).expect("valid JSON")
    });
    &V
}

pub fn input_failure_reasons() -> &'static Value {
    static V: LazyLock<Value> = LazyLock::new(|| {
        serde_json::from_str(r#"{"timeout":"timeout","userSkipped":"user_skipped","invalidInputAttempts":"invalid_input_attempts"}"#).expect("valid JSON")
    });
    &V
}

schema!(pub fn get_user_data_step_schema = z::object(vec![("id", z::string().regex(r#"^\d+$"#, "").describe("Step id (numeric string), unique within the flow.")),
("stepType", z::literal(json!("getUserData")).describe("Step type discriminator: \"getUserData\".")),
("message", z::string().trim().min(1).max(255)),
("replyFormat", z::string().pipe(z::enum_(&["RF01", "RF02", "RF03", "RF04", "RF05", "RF06", "RF07", "RF08", "RF09", "RF10", "RF11"]))),
("outputFieldId", z::string().trim().min(1)),
("retryMessage", z::string().trim().max(255).default(json!("")).describe("Sent when the reply is invalid, before the question is asked again. Empty sends only the question.")),
("skipButtonLabel", z::string().trim().max(255).default(json!("Skip")).describe("Label of the Skip button sent with the question. Tapping it takes the Skip branch. Empty hides the button. Channels cap the shown label at 20 characters.")),
("showHint", z::boolean().optional().describe("Phone/email reply formats only: offer the contact's stored phone/email as a quick reply on Messenger and Instagram. With no stored value, Messenger falls back to its native phone/email quick reply. Other channels send the plain prompt.")),
("autoSkip", z::boolean().default(json!(false)).describe("Take the Skip branch on its own when the contact has not answered within autoSkipTimeValue autoSkipTimeUnit of the first question (retries do not restart the clock), or after autoSkipFailAttempts invalid replies.")),
("autoSkipTimeUnit", z::string().pipe(wait_step_delay_units()).default(json!("minutes")).describe("Unit of autoSkipTimeValue: \"seconds\", \"minutes\", \"hours\" or \"days\".")),
("autoSkipTimeValue", z::number().coerce().int().gte(1 as f64).lte(100 as f64).default(json!(3)).describe("Time to wait for an answer before auto skipping (1-100).")),
("autoSkipFailAttempts", z::number().coerce().int().gte(1 as f64).lte(100 as f64).default(json!(3)).describe("Invalid replies allowed before auto skipping (1-100).")),
("states", z::tuple(vec![success_state_schema(),
skip_state_schema()]))]));

// HAND-PORT encodeContactHintQuickReplyPayload:
// function encodeContactHintQuickReplyPayload(value){return`${CONTACT_HINT_QUICK_REPLY_PREFIX}${value}`}

// HAND-PORT decodeContactHintQuickReplyPayload:
// function decodeContactHintQuickReplyPayload(payload){if(!payload?.startsWith(CONTACT_HINT_QUICK_REPLY_PREFIX)){return null}return payload.slice(CONTACT_HINT_QUICK_REPLY_PREFIX.length)||null}

// HAND-PORT getUserDataStepDefaultFn:
// (props={})=>({message:"",replyFormat:ReplyFormat.text,outputFieldId:"",retryMessage:"",skipButtonLabel:"Skip",showHint:false,autoSkip:false,autoSkipTimeUnit:waitStepDelayUnits.enum.minutes,autoSkipTimeValue:3,autoSkipFailAttempts:3,states:[successStateDefaultFn(),skipStateDefaultFn()],...props,id:createId(),stepType:stepTypes.enum.getUserData})

// ---- stage 1 API (hand-ported in stage 2) ----

pub fn get_user_data_step_default_fn(props: &Value) -> Value {
    let _ = props;
    todo!("P3.4")
}

pub fn decode_contact_hint_quick_reply_payload(payload: Option<&str>) -> Option<String> {
    todo!("P3.4")
}

pub fn encode_contact_hint_quick_reply_payload(value: &str) -> String {
    todo!("P3.4")
}
