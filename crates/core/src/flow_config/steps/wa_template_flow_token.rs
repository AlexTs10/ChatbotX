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
("buttonIndex", z::union(vec![z::number().int().gte(0_f64),
z::string().regex(r#"^\d+$"#, "").transform(number)]).pipe(z::number().int().gte(0_f64))),
("cardIndex", z::union(vec![z::number().int().gte(0_f64),
z::string().regex(r#"^\d+$"#, "").transform(number)]).pipe(z::number().int().gte(0_f64)).optional())]));

schema!(pub fn flow_step_template_flow_token_schema = z::object(vec![("origin", z::literal(json!("s"))),
("flowId", z::string().regex(r#"^\d+$"#, "").refine(broadcast_template_flow_token_schema_broadcast_id_refine, Some("Value exceeds the signed bigint range"), None)),
("flowVersionId", z::string().regex(r#"^\d+$"#, "").refine(broadcast_template_flow_token_schema_broadcast_id_refine, Some("Value exceeds the signed bigint range"), None).optional()),
("stepId", z::string().regex(r#"^\d+$"#, "").refine(broadcast_template_flow_token_schema_broadcast_id_refine, Some("Value exceeds the signed bigint range"), None)),
("buttonIndex", z::union(vec![z::number().int().gte(0_f64),
z::string().regex(r#"^\d+$"#, "").transform(number)]).pipe(z::number().int().gte(0_f64))),
("cardIndex", z::union(vec![z::number().int().gte(0_f64),
z::string().regex(r#"^\d+$"#, "").transform(number)]).pipe(z::number().int().gte(0_f64)).optional())]));

schema!(pub fn template_flow_token_schema = z::discriminated_union("origin", vec![broadcast_template_flow_token_schema(),
flow_step_template_flow_token_schema()]));

fn broadcast_template_flow_token_schema_broadcast_id_refine(value: &Value) -> bool {
    crate::flow_config::util::is_within_signed_bigint_range(value)
}

fn number(value: Value, _ctx: &mut RefineCtx) -> Value {
    // `.transform(Number)` on a digits-only string.
    num_value(js_number(Some(&value)))
}

const TOKEN_PREFIX: &str = "watf";

/// `isTemplateFlowToken`.
pub fn is_template_flow_token(token: Option<&str>) -> bool {
    token.is_some_and(|t| t.starts_with("watf:"))
}

fn segment(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        Value::Number(n) => crate::js::number_to_string(n.as_f64().unwrap_or(0.0)),
        other => other.to_string(),
    }
}

/// `encodeTemplateFlowToken`.
pub fn encode_template_flow_token(token: &Value) -> String {
    let opt = |key: &str| token.get(key).map(segment).unwrap_or_default();
    if token["origin"] == "b" {
        return [
            TOKEN_PREFIX.to_owned(),
            segment(&token["origin"]),
            opt("broadcastId"),
            opt("buttonIndex"),
            opt("cardIndex"),
        ]
        .join(":");
    }
    [
        TOKEN_PREFIX.to_owned(),
        segment(&token["origin"]),
        opt("flowId"),
        opt("flowVersionId"),
        opt("stepId"),
        opt("buttonIndex"),
        opt("cardIndex"),
    ]
    .join(":")
}

/// `decodeTemplateFlowToken`: the token, or `None` when malformed.
pub fn decode_template_flow_token(raw_token: Option<&str>) -> Option<Value> {
    if !is_template_flow_token(raw_token) {
        return None;
    }
    let parts: Vec<&str> = raw_token?.split(':').collect();
    if parts[0] != TOKEN_PREFIX {
        return None;
    }
    let opt = |s: &str| if s.is_empty() { Value::Null } else { json!(s) };
    let mut input = match parts.get(1).copied() {
        Some("b") => {
            if !(parts.len() == 4 || parts.len() == 5) {
                return None;
            }
            json!({
                "origin": parts[1],
                "broadcastId": parts[2],
                "buttonIndex": parts[3],
                "cardIndex": opt(parts.get(4).copied().unwrap_or("")),
            })
        }
        Some("s") => {
            if !(parts.len() == 6 || parts.len() == 7) {
                return None;
            }
            json!({
                "origin": parts[1],
                "flowId": parts[2],
                "flowVersionId": opt(parts[3]),
                "stepId": parts[4],
                "buttonIndex": parts[5],
                "cardIndex": opt(parts.get(6).copied().unwrap_or("")),
            })
        }
        _ => return None,
    };
    // `x || undefined`: an empty optional segment is absent.
    if let Some(obj) = input.as_object_mut() {
        obj.retain(|_, v| !v.is_null());
    }
    template_flow_token_schema().parse(&input).ok()
}
