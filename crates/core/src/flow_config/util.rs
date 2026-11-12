//! Port of `packages/flow-config/src/util.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

// HAND-PORT extractMetadata:
// (key,metadata)=>{if(!metadata){return}return metadata[key]||void 0}

schema!(pub fn button_payload_schema = z::object(vec![("f", z::string().regex(r#"^\d+$"#, "").regex(r#"^\d+$"#, "").refine(is_within_signed_bigint_range, Some("Value exceeds the signed bigint range"), None)),
("fv", z::string().regex(r#"^\d+$"#, "").regex(r#"^\d+$"#, "").refine(is_within_signed_bigint_range, Some("Value exceeds the signed bigint range"), None).optional()),
("b", z::string().regex(r#"^\d+$"#, "").regex(r#"^\d+$"#, "").refine(is_within_signed_bigint_range, Some("Value exceeds the signed bigint range"), None).optional()),
("br", z::string().regex(r#"^\d+$"#, "").regex(r#"^\d+$"#, "").refine(is_within_signed_bigint_range, Some("Value exceeds the signed bigint range"), None).optional()),
("ss", z::string().regex(r#"^\d+$"#, "").regex(r#"^\d+$"#, "").refine(is_within_signed_bigint_range, Some("Value exceeds the signed bigint range"), None).optional()),
("cid", z::string().regex(r#"^\d+$"#, "").regex(r#"^\d+$"#, "").refine(is_within_signed_bigint_range, Some("Value exceeds the signed bigint range"), None).optional()),
("ca", z::string().regex(r#"^\d+$"#, "").regex(r#"^\d+$"#, "").refine(is_within_signed_bigint_range, Some("Value exceeds the signed bigint range"), None).optional()),
("n", z::string().regex(r#"^\d+$"#, "").regex(r#"^\d+$"#, "").refine(is_within_signed_bigint_range, Some("Value exceeds the signed bigint range"), None).optional())]).transform(button_payload_schema_transform));

pub const ZALO_POSTBACK_TEXT_PREFIX: &str = "postback_";

// HAND-PORT encodeButtonPayload:
// props=>{const parts=[props.flowId,props.flowVersionId??"",props.buttonId??"",props.broadcastId??"",props.sequenceStepId??"",props.contactInboxId??"",props.commentAutomationId??"",props.nodeId??""];while(parts.length>2&&parts.at(-1)===""){parts.pop()}return parts.join(":")}

// HAND-PORT decodeButtonPayload:
// (payload,options)=>{try{if(options?.allowBareFlowId&&BARE_FLOW_ID_REGEX.test(payload)&&BigInt(payload)<=MAX_SIGNED_BIGINT){return buttonPayloadSchema.parse({f:payload})}if(payload.includes(":")){const[f,fv,b,br,ss,cid,ca,n]=payload.split(":");return buttonPayloadSchema.parse({f,fv:fv||void 0,b:b||void 0,br:br||void 0,ss:ss||void 0,cid:cid||void 0,ca:ca||void 0,n:n||void 0})}return buttonPayloadSchema.parse(JSON.parse(atob(payload)))}catch{return null}}

// HAND-PORT isMagicLinkUrl:
// url=>{try{const urlObj=new URL(url);return MAGIC_LINK_PATHNAME_REGEX.test(urlObj.pathname)}catch{return false}}

// HAND-PORT appendCodeToMagicLink:
// (url,code)=>{if(!isMagicLinkUrl(url)){return url}const urlObj=new URL(url);urlObj.searchParams.set("code",code);return urlObj.toString()}

// HAND-PORT getNodeFromButton:
// function getNodeFromButton(nodes,buttonId){const found=findButtonInNodes(nodes,buttonId);return{button:found?.button??null,nodeId:found?.runtimeNodeId??null}}

// HAND-PORT getNodeFromQuickReply:
// function getNodeFromQuickReply(nodes,quickReplyId){for(const node of nodes){if(!("quickReplies"in node.data.details&&node.data.details.quickReplies)){continue}const quickReply=node.data.details.quickReplies.find(reply=>reply.id===quickReplyId);if(quickReply){return{quickReply,nodeId:node.id}}}return{quickReply:null,nodeId:null}}

pub fn flow_action_target_types() -> &'static Value {
    static V: LazyLock<Value> = LazyLock::new(|| {
        serde_json::from_str(r#"{"button":"button","quickReply":"quickReply"}"#)
            .expect("valid JSON")
    });
    &V
}

// HAND-PORT resolveFlowActionTarget:
// function resolveFlowActionTarget(nodes,actionId){const{button,nodeId}=getNodeFromButton(nodes,actionId);if(button){return{details:button,nodeId,targetType:flowActionTargetTypes.button}}const{quickReply,nodeId:quickReplyNodeId}=getNodeFromQuickReply(nodes,actionId);if(quickReply){return{details:quickReply,nodeId:quickReplyNodeId,targetType:flowActionTargetTypes.quickReply}}return null}

// data=>({flowId:data.f,...data.fv?{flowVersionId:data.fv}:{},...data.b?{buttonId:data.b}:{},...data.br?{broadcastId:data.br}:{},...data.ss?{sequenceStepId:data.ss}:{},...data.cid?{contactInboxId:data.cid}:{},...data.ca?{commentAutomationId:data.ca}:{},...data.n?{nodeId:data.n}:{}})
fn button_payload_schema_transform(value: Value, ctx: &mut RefineCtx) -> Value {
    todo!("P3.4")
}

// value=>{try{return BigInt(value)<=MAX_SIGNED_BIGINT}catch{return false}}
fn is_within_signed_bigint_range(value: &Value) -> bool {
    todo!("P3.4")
}

// ---- stage 1 API (hand-ported in stage 2) ----

/// `ButtonPayload` (decoded flow button payload).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ButtonPayload {
    pub flow_id: String,
    pub flow_version_id: Option<String>,
    pub button_id: Option<String>,
    pub broadcast_id: Option<String>,
    pub sequence_step_id: Option<String>,
    pub contact_inbox_id: Option<String>,
    pub comment_automation_id: Option<String>,
    pub node_id: Option<String>,
}

/// `FlowActionTarget`.
#[derive(Debug, Clone, PartialEq)]
pub struct FlowActionTarget {
    pub details: Value,
    pub node_id: Option<String>,
    pub target_type: &'static str,
}

pub fn decode_button_payload(payload: &str, allow_bare_flow_id: bool) -> Option<ButtonPayload> {
    todo!("P3.4")
}

pub fn encode_button_payload(props: &ButtonPayload) -> String {
    todo!("P3.4")
}

pub fn resolve_flow_action_target(nodes: &[Value], action_id: &str) -> Option<FlowActionTarget> {
    todo!("P3.4")
}
