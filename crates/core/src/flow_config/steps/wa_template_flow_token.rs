//! Port of `packages/flow-config/src/steps/wa-template-flow-token.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

pub fn template_flow_origin() -> &'static Value {
    static V: LazyLock<Value> = LazyLock::new(|| {
        serde_json::from_str(r#"{"Broadcast":"b","FlowStep":"s"}"#).expect("valid JSON")
    });
    &V
}

schema!(pub fn broadcast_template_flow_token_schema = z::object(vec![("origin", z::literal(json!("b"))),
("broadcastId", z::string().regex(r#"^\d+$"#, "").refine(broadcast_template_flow_token_schema_broadcast_id_refine, Some("Value exceeds the signed bigint range"), None)),
("buttonIndex", z::union(vec![z::number().int().gte(0 as f64),
z::string().regex(r#"^\d+$"#, "").transform(number)]).pipe(z::number().int().gte(0 as f64))),
("cardIndex", z::union(vec![z::number().int().gte(0 as f64),
z::string().regex(r#"^\d+$"#, "").transform(number)]).pipe(z::number().int().gte(0 as f64)).optional())]));

schema!(pub fn flow_step_template_flow_token_schema = z::object(vec![("origin", z::literal(json!("s"))),
("flowId", z::string().regex(r#"^\d+$"#, "").refine(broadcast_template_flow_token_schema_broadcast_id_refine, Some("Value exceeds the signed bigint range"), None)),
("flowVersionId", z::string().regex(r#"^\d+$"#, "").refine(broadcast_template_flow_token_schema_broadcast_id_refine, Some("Value exceeds the signed bigint range"), None).optional()),
("stepId", z::string().regex(r#"^\d+$"#, "").refine(broadcast_template_flow_token_schema_broadcast_id_refine, Some("Value exceeds the signed bigint range"), None)),
("buttonIndex", z::union(vec![z::number().int().gte(0 as f64),
z::string().regex(r#"^\d+$"#, "").transform(number)]).pipe(z::number().int().gte(0 as f64))),
("cardIndex", z::union(vec![z::number().int().gte(0 as f64),
z::string().regex(r#"^\d+$"#, "").transform(number)]).pipe(z::number().int().gte(0 as f64)).optional())]));

schema!(pub fn template_flow_token_schema = z::discriminated_union("origin", vec![broadcast_template_flow_token_schema(),
flow_step_template_flow_token_schema()]));

// HAND-PORT isTemplateFlowToken:
// token=>typeof token==="string"&&token.startsWith(`${TOKEN_PREFIX}:`)

// HAND-PORT encodeTemplateFlowToken:
// token=>{if(token.origin===TemplateFlowOrigin.Broadcast){return[TOKEN_PREFIX,token.origin,token.broadcastId,token.buttonIndex,token.cardIndex??""].join(":")}return[TOKEN_PREFIX,token.origin,token.flowId,token.flowVersionId??"",token.stepId,token.buttonIndex,token.cardIndex??""].join(":")}

// HAND-PORT decodeTemplateFlowToken:
// rawToken=>{if(!isTemplateFlowToken(rawToken)){return null}const token=rawToken;const parts=token.split(":");try{if(parts[0]!==TOKEN_PREFIX){return null}if(parts[1]===TemplateFlowOrigin.Broadcast){if(!(parts.length===4||parts.length===5)){return null}const[,origin,broadcastId,buttonIndex,cardIndex]=parts;return templateFlowTokenSchema.parse({origin,broadcastId,buttonIndex,cardIndex:cardIndex||void 0})}if(parts[1]===TemplateFlowOrigin.FlowStep){if(!(parts.length===6||parts.length===7)){return null}const[,origin,flowId,flowVersionId,stepId,buttonIndex,cardIndex]=parts;return templateFlowTokenSchema.parse({origin,flowId,flowVersionId:flowVersionId||void 0,stepId,buttonIndex,cardIndex:cardIndex||void 0})}}catch{return null}return null}

// value=>{try{return BigInt(value)<=MAX_SIGNED_BIGINT}catch{return false}}
fn broadcast_template_flow_token_schema_broadcast_id_refine(value: &Value) -> bool {
    todo!("P3.4")
}

// function Number() { [native code] }
fn number(value: Value, ctx: &mut RefineCtx) -> Value {
    todo!("P3.4")
}

// ---- stage 1 API (hand-ported in stage 2) ----

pub fn decode_template_flow_token(raw_token: Option<&str>) -> Option<Value> {
    todo!("P3.4")
}

pub fn encode_template_flow_token(token: &Value) -> String {
    todo!("P3.4")
}

pub fn is_template_flow_token(token: Option<&str>) -> bool {
    todo!("P3.4")
}
