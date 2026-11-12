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
("duration", z::number().coerce().int().gte(1 as f64)),
("unit", quick_reply_settings_delay_units()),
("target", quick_reply_next_step_schema().nullable())]));

schema!(pub fn quick_reply_retry_settings_schema = z::object(vec![("id", z::string().regex(r#"^\d+$"#, "")),
("enabled", z::boolean()),
("message", z::string().trim().max(255)),
("maxRetries", z::number().coerce().int().gte(0 as f64).lte(5 as f64)),
("target", quick_reply_next_step_schema().nullable())]));

schema!(pub fn quick_reply_settings_schema = z::object(vec![("followUp", quick_reply_follow_up_settings_schema()),
("retry", quick_reply_retry_settings_schema())]));

default_fn!(pub fn quick_reply_settings_default_fn = json!({"followUp": {"id": create_id(), "enabled": false, "duration": 1, "unit": "days", "target": null}, "retry": {"id": create_id(), "enabled": false, "message": "", "maxRetries": 3, "target": null}}));

// HAND-PORT refineQuickReplySettings:
// (details,ctx)=>{const settings=details.quickReplySettings;if(!settings||(details.quickReplies??[]).length===0){return}const{followUp,retry}=settings;if(followUp.enabled){if(!followUp.target){ctx.addIssue({code:"custom",path:["quickReplySettings","followUp","target"],message:flowValidationCodes.quickReplyNextStepRequired})}if(Number(followUp.duration)*delayUnitToMs(followUp.unit)>FOLLOW_UP_MAX_DELAY_MS){ctx.addIssue({code:"custom",path:["quickReplySettings","followUp","duration"],message:`Follow-up delay cannot exceed ${FOLLOW_UP_MAX_DELAY_DAYS} days`})}}if(retry.enabled){if(!retry.target){ctx.addIssue({code:"custom",path:["quickReplySettings","retry","target"],message:flowValidationCodes.quickReplyNextStepRequired})}if(Number(retry.maxRetries)>=1&&retry.message.trim().length===0){ctx.addIssue({code:"custom",path:["quickReplySettings","retry","message"],message:flowValidationCodes.quickReplyRetryMessageRequired})}if(hasGetUserDataStep(details)){ctx.addIssue({code:"custom",path:["quickReplySettings","retry","enabled"],message:flowValidationCodes.quickReplyRetryWithGetUserData})}}}

// HAND-PORT resolveActiveQuickReplySettings:
// rawDetails=>{const details=asSettingsDetails(rawDetails);const settings=details?.quickReplySettings;if(!(details&&settings)||(details.quickReplies??[]).length===0){return{}}const active={};if(settings.followUp?.enabled&&settings.followUp.target){active.followUp={...settings.followUp,duration:Number(settings.followUp.duration),target:settings.followUp.target}}if(settings.retry?.enabled&&settings.retry.target&&!hasGetUserDataStep(details)){active.retry={...settings.retry,maxRetries:Number(settings.retry.maxRetries),target:settings.retry.target}}return active}

// HAND-PORT quickReplyNextStepFollowsEdge:
// target=>target===null||target.buttonType===buttonTypes.enum.sendMessage||target.buttonType===buttonTypes.enum.performAction||target.buttonType===buttonTypes.enum.startAnotherNode

// HAND-PORT listQuickReplySettingsHandles:
// rawDetails=>{const details=asSettingsDetails(rawDetails);const settings=details?.quickReplySettings;if(!(details&&settings)||(details.quickReplies??[]).length===0){return[]}const handles=[];if(settings.followUp?.enabled&&quickReplyNextStepFollowsEdge(settings.followUp.target)){handles.push({kind:"followUp",id:settings.followUp.id})}if(settings.retry?.enabled&&!hasGetUserDataStep(details)&&quickReplyNextStepFollowsEdge(settings.retry.target)){handles.push({kind:"retry",id:settings.retry.id})}return handles}

// HAND-PORT computeQuickReplyFollowUpTriggerAt:
// settings=>addMilliseconds(Date.now(),Number(settings.duration)*delayUnitToMs(settings.unit))

// ---- stage 1 API (hand-ported in stage 2) ----

pub fn compute_quick_reply_follow_up_trigger_at(
    settings: &Value,
    clock: &dyn crate::Clock,
) -> chrono::DateTime<chrono::Utc> {
    todo!("P3.4")
}

pub fn list_quick_reply_settings_handles(raw_details: &Value) -> Vec<Value> {
    todo!("P3.4")
}

pub fn quick_reply_next_step_follows_edge(target: &Value) -> bool {
    todo!("P3.4")
}

pub fn resolve_active_quick_reply_settings(raw_details: Option<&Value>) -> Value {
    todo!("P3.4")
}
