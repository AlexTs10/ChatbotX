//! Port of `packages/flow-config/src/steps/quick-reply-settings.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

schema!(pub fn quick_reply_settings_delay_units = z::enum_(&["minutes", "hours", "days"]));

pub const QUICK_REPLY_MAX_RETRIES: usize = 5;

pub const QUICK_REPLY_DEFAULT_RETRIES: usize = 3;

pub const QUICK_REPLY_RETRY_MESSAGE_MAX: usize = 255;

pub const QUICK_REPLY_NEXT_STEP_HIDDEN_BUTTON_TYPES: &[&str] = &[
    "openWebsite",
    "openMiniApp",
    "callPhoneNumber",
    "copyText",
    "whatsappOptionList",
];

schema!(pub fn quick_reply_next_step_schema = z::discriminated_union("buttonType", vec![z::object(vec![("buttonType", z::literal(json!("sendMessage"))),
("beforeStep", start_another_node_step_schema())]),
z::object(vec![("buttonType", z::literal(json!("performAction"))),
("beforeStep", start_another_node_step_schema())]),
z::object(vec![("buttonType", z::literal(json!("startAnotherNode"))),
("beforeStep", start_another_node_step_schema())]),
z::object(vec![("buttonType", z::literal(json!("startExternalFlow"))),
("beforeStep", start_external_flow_step_schema())]),
z::object(vec![("buttonType", z::literal(json!("startExternalNode"))),
("beforeStep", start_external_node_step_schema())])]));

schema!(pub fn quick_reply_follow_up_settings_schema = z::object(vec![("id", z::string().regex(r#"^\d+$"#, "")),
("enabled", z::boolean()),
("duration", z::number().coerce().int().gte(1_f64)),
("unit", quick_reply_settings_delay_units()),
("target", quick_reply_next_step_schema().nullable())]));

schema!(pub fn quick_reply_retry_settings_schema = z::object(vec![("id", z::string().regex(r#"^\d+$"#, "")),
("enabled", z::boolean()),
("message", z::string().trim().max(255)),
("maxRetries", z::number().coerce().int().gte(0_f64).lte(5_f64)),
("target", quick_reply_next_step_schema().nullable())]));

schema!(pub fn quick_reply_settings_schema = z::object(vec![("followUp", quick_reply_follow_up_settings_schema()),
("retry", quick_reply_retry_settings_schema())]));

default_fn!(pub fn quick_reply_settings_default_fn = json!({"followUp": {"id": create_id(), "enabled": false, "duration": 1, "unit": "days", "target": null}, "retry": {"id": create_id(), "enabled": false, "message": "", "maxRetries": 3, "target": null}}));

fn has_get_user_data_step(details: &Value) -> bool {
    arr_field(details, "steps")
        .iter()
        .any(|step| step["stepType"] == "getUserData")
}

/// `refineQuickReplySettings(details, ctx)` (on the send-message details).
pub fn refine_quick_reply_settings(details: &Value, ctx: &mut RefineCtx) {
    let settings = &details["quickReplySettings"];
    if !truthy(Some(settings)) || arr_field(details, "quickReplies").is_empty() {
        return;
    }
    let follow_up = &settings["followUp"];
    let retry = &settings["retry"];
    if truthy(follow_up.get("enabled")) {
        if !truthy(follow_up.get("target")) {
            ctx.issue(
                path!["quickReplySettings", "followUp", "target"],
                "quickReplyNextStepRequired",
            );
        }
        let max_ms = FOLLOW_UP_MAX_DELAY_DAYS as f64 * 86_400_000.0;
        if js_number(follow_up.get("duration"))
            * delay_unit_to_ms(str_field(follow_up, "unit")) as f64
            > max_ms
        {
            ctx.issue(
                path!["quickReplySettings", "followUp", "duration"],
                format!("Follow-up delay cannot exceed {FOLLOW_UP_MAX_DELAY_DAYS} days"),
            );
        }
    }
    if truthy(retry.get("enabled")) {
        if !truthy(retry.get("target")) {
            ctx.issue(
                path!["quickReplySettings", "retry", "target"],
                "quickReplyNextStepRequired",
            );
        }
        if js_number(retry.get("maxRetries")) >= 1.0
            && crate::js::trim(str_field(retry, "message")).is_empty()
        {
            ctx.issue(
                path!["quickReplySettings", "retry", "message"],
                "quickReplyRetryMessageRequired",
            );
        }
        if has_get_user_data_step(details) {
            ctx.issue(
                path!["quickReplySettings", "retry", "enabled"],
                "quickReplyRetryWithGetUserData",
            );
        }
    }
}

fn settings_details(raw: Option<&Value>) -> Option<(&Value, &Value)> {
    let details = raw.filter(|d| d.is_object())?;
    let settings = details
        .get("quickReplySettings")
        .filter(|s| truthy(Some(s)))?;
    if arr_field(details, "quickReplies").is_empty() {
        return None;
    }
    Some((details, settings))
}

/// `resolveActiveQuickReplySettings(details)`: enabled sections with a target.
pub fn resolve_active_quick_reply_settings(raw_details: Option<&Value>) -> Value {
    let Some((details, settings)) = settings_details(raw_details) else {
        return json!({});
    };
    let mut active = Map::new();
    let follow_up = &settings["followUp"];
    if truthy(follow_up.get("enabled")) && truthy(follow_up.get("target")) {
        let mut out = follow_up.clone();
        out["duration"] = num_value(js_number(follow_up.get("duration")));
        active.insert("followUp".into(), out);
    }
    let retry = &settings["retry"];
    if truthy(retry.get("enabled"))
        && truthy(retry.get("target"))
        && !has_get_user_data_step(details)
    {
        let mut out = retry.clone();
        out["maxRetries"] = num_value(js_number(retry.get("maxRetries")));
        active.insert("retry".into(), out);
    }
    Value::Object(active)
}

/// `quickReplyNextStepFollowsEdge(target)`: null or a node jump.
pub fn quick_reply_next_step_follows_edge(target: &Value) -> bool {
    target.is_null()
        || matches!(
            target["buttonType"].as_str(),
            Some("sendMessage" | "performAction" | "startAnotherNode")
        )
}

/// `listQuickReplySettingsHandles(details)`.
pub fn list_quick_reply_settings_handles(raw_details: &Value) -> Vec<Value> {
    let Some((details, settings)) = settings_details(Some(raw_details)) else {
        return Vec::new();
    };
    let mut handles = Vec::new();
    let follow_up = &settings["followUp"];
    if truthy(follow_up.get("enabled")) && quick_reply_next_step_follows_edge(&follow_up["target"])
    {
        handles.push(json!({ "kind": "followUp", "id": follow_up["id"] }));
    }
    let retry = &settings["retry"];
    if truthy(retry.get("enabled"))
        && !has_get_user_data_step(details)
        && quick_reply_next_step_follows_edge(&retry["target"])
    {
        handles.push(json!({ "kind": "retry", "id": retry["id"] }));
    }
    handles
}

/// `computeQuickReplyFollowUpTriggerAt(settings)`: now + duration × unit.
pub fn compute_quick_reply_follow_up_trigger_at(
    settings: &Value,
    clock: &dyn crate::Clock,
) -> chrono::DateTime<chrono::Utc> {
    let ms =
        js_number(settings.get("duration")) * delay_unit_to_ms(str_field(settings, "unit")) as f64;
    chrono::DateTime::from_timestamp_millis(clock.now_ms() + ms as i64).unwrap_or_default()
}
