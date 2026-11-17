//! `handlers/message/outgoing-message/send-wa-template.ts` and
//! `payment-request-parameter.ts`: a `sendWaTemplateMessage` step as the
//! Cloud API `template` message.

use chatbotx_core::flow_config::extract_metadata;
use chatbotx_core::flow_config::steps::{
    encode_template_flow_token, payment_amount_to_minor_units, wa_payment_setting_type_by_method,
};
use serde_json::{Map, Value, json};

use crate::channel::{Category, ChannelError, ChannelResult, SendFailure, SendFlowStep};

use super::whatsapp::WaMessage;

/// `TemplateBuildContext`.
struct BuildCtx<'a> {
    flow_id: &'a str,
    flow_version_id: Option<&'a str>,
    step_id: &'a str,
    broadcast_id: Option<String>,
    pix_redirect: bool,
}

fn s<'a>(v: &'a Value, k: &str) -> &'a str {
    v.get(k).and_then(Value::as_str).unwrap_or("")
}

fn is_blank(v: &Value, k: &str) -> bool {
    s(v, k).trim().is_empty()
}

fn payload_invalid(message: &str) -> SendFailure {
    SendFailure::Channel(ChannelError::new(message, Category::PayloadInvalid))
}

/// `toPaymentSetting` + `toAmountFields` (`buildPaymentRequestParameter`).
fn payment_request_parameter(request: &Value, pix_redirect: bool) -> ChannelResult<Value> {
    let method = s(request, "method");
    let kind = wa_payment_setting_type_by_method()[method].clone();
    let setting = match method {
        "pix" => {
            let mut code = json!({ "code": request["code"] });
            if pix_redirect {
                code["flow_type"] = json!("APPSWITCH");
            }
            json!({ "type": kind, "pix_dynamic_code": code })
        }
        "boleto" => {
            json!({ "type": kind, "boleto": { "digitable_line": request["digitableLine"] } })
        }
        "payment_link" => json!({ "type": kind, "payment_link": { "uri": request["url"] } }),
        _ => {
            let mut card = json!({ "last_four_digits": request["lastFourDigits"] });
            if !is_blank(request, "credentialId") {
                card["credential_id"] = request["credentialId"].clone();
            }
            json!({ "type": kind, "offsite_card_pay": card })
        }
    };
    let mut action = json!({ "payment_setting": setting });
    let amount = s(request, "amount");
    if !amount.trim().is_empty() {
        let value = payment_amount_to_minor_units(amount, Some(2))
            .ok_or_else(|| payload_invalid("Payment request amount is not a valid BRL amount"))?;
        action["currency"] = json!("BRL");
        action["total_amount"] = json!({ "value": value, "offset": 100 });
    }
    Ok(json!({ "type": "action", "action": { "payment_request": action } }))
}

/// `buildTemplateFlowToken`.
fn template_flow_token(
    param: &Value,
    ctx: &BuildCtx,
    card_index: Option<&Value>,
) -> Option<String> {
    let button_index = param.get("index").filter(|i| !i.is_null())?;
    let mut token = Map::new();
    if let (Some(broadcast), true) = (&ctx.broadcast_id, ctx.flow_id.is_empty()) {
        token.insert("origin".into(), json!("b"));
        token.insert("broadcastId".into(), json!(broadcast));
    } else if !ctx.flow_id.is_empty() && !ctx.step_id.is_empty() {
        token.insert("origin".into(), json!("s"));
        token.insert("flowId".into(), json!(ctx.flow_id));
        if let Some(version) = ctx.flow_version_id {
            token.insert("flowVersionId".into(), json!(version));
        }
        token.insert("stepId".into(), json!(ctx.step_id));
    } else {
        tracing::warn!(
            step_id = ctx.step_id,
            "WhatsApp template FLOW button sent without generated token: origin could not be resolved"
        );
        return None;
    }
    token.insert("buttonIndex".into(), button_index.clone());
    if let Some(card) = card_index {
        token.insert("cardIndex".into(), card.clone());
    }
    Some(encode_template_flow_token(&Value::Object(token)))
}

fn sub_type(param: &Value) -> &str {
    param
        .get("sub_type")
        .and_then(Value::as_str)
        .filter(|t| !t.is_empty())
        .unwrap_or("url")
}

/// `buildButtonParameter`.
fn button_parameter(
    param: &Value,
    ctx: &BuildCtx,
    card_index: Option<&Value>,
) -> ChannelResult<Value> {
    Ok(match sub_type(param) {
        "copy_code" => json!({ "type": "coupon_code", "coupon_code": s(param, "coupon_code") }),
        "quick_reply" => json!({ "type": "payload", "payload": s(param, "payload") }),
        "flow" => {
            let mut action = Map::new();
            if let Some(token) = template_flow_token(param, ctx, card_index) {
                action.insert("flow_token".into(), json!(token));
            }
            if let Some(data) = param
                .get("flow_action_data")
                .filter(|d| d.as_object().is_some_and(|o| !o.is_empty()))
            {
                action.insert("flow_action_data".into(), data.clone());
            }
            json!({ "type": "action", "action": action })
        }
        "catalog" => json!({ "type": "action", "action": {
            "thumbnail_product_retailer_id": s(param, "thumbnail_product_retailer_id") } }),
        "mpm" => json!({ "type": "action", "action": {
            "sections": param.get("sections").cloned().unwrap_or_else(|| json!([])) } }),
        "payment_request" => {
            let request = param
                .get("payment_request")
                .filter(|r| r.is_object())
                .ok_or_else(|| {
                    payload_invalid("WhatsApp payment request button has no payment data")
                })?;
            payment_request_parameter(request, ctx.pix_redirect)?
        }
        _ => json!({ "type": "text", "text": s(param, "text") }),
    })
}

/// `hasButtonParamContent`.
fn has_content(param: &Value) -> bool {
    match sub_type(param) {
        "url" => !is_blank(param, "text"),
        "quick_reply" => !is_blank(param, "payload"),
        "copy_code" => !is_blank(param, "coupon_code"),
        "catalog" => !is_blank(param, "thumbnail_product_retailer_id"),
        "mpm" => param["sections"].as_array().is_some_and(|a| !a.is_empty()),
        "payment_request" => param.get("payment_request").is_some_and(|p| !p.is_null()),
        _ => true,
    }
}

/// `resolveButtonParams`: no holes, no blank quick reply / catalog, first
/// entry with content wins among duplicate explicit `(sub_type, index)`.
fn resolve_button_params(buttons: &[Value]) -> Vec<(Value, i64)> {
    let resolved: Vec<(Value, i64, bool)> = buttons
        .iter()
        .enumerate()
        .filter(|(_, p)| p.is_object())
        .map(|(position, p)| {
            let explicit = p["index"].as_i64();
            (
                p.clone(),
                explicit.unwrap_or(position as i64),
                explicit.is_some(),
            )
        })
        .filter(|(p, _, _)| !(matches!(sub_type(p), "quick_reply" | "catalog") && !has_content(p)))
        .collect();
    let mut winners: Vec<(String, usize)> = Vec::new();
    for (position, (p, index, explicit)) in resolved.iter().enumerate() {
        if !explicit {
            continue;
        }
        let key = format!("{}:{index}", sub_type(p));
        match winners.iter_mut().find(|(k, _)| *k == key) {
            None => winners.push((key, position)),
            Some((_, current)) => {
                if !has_content(&resolved[*current].0) && has_content(p) {
                    *current = position;
                }
            }
        }
    }
    resolved
        .iter()
        .enumerate()
        .filter(|(position, (p, index, explicit))| {
            !explicit
                || winners
                    .iter()
                    .any(|(k, w)| *k == format!("{}:{index}", sub_type(p)) && w == position)
        })
        .map(|(_, (p, index, _))| (p.clone(), *index))
        .collect()
}

/// `buildButtonComponents`.
fn button_components(
    buttons: &[Value],
    ctx: &BuildCtx,
    card_index: Option<&Value>,
) -> ChannelResult<Vec<Value>> {
    resolve_button_params(buttons)
        .into_iter()
        .map(|(param, index)| {
            let kind = sub_type(&param).to_owned();
            // Meta's `payment_request` examples send the index as a string.
            let index = if kind == "payment_request" {
                json!(index.to_string())
            } else {
                json!(index)
            };
            Ok(json!({ "type": "button", "sub_type": kind, "index": index,
                "parameters": [button_parameter(&param, ctx, card_index)?] }))
        })
        .collect()
}

/// `buildTextParameter`: `parameter_name` only for NAMED placeholders.
fn text_parameter(param: &Value) -> Value {
    let mut out = json!({ "type": "text", "text": s(param, "text") });
    if !s(param, "parameter_name").is_empty() {
        out["parameter_name"] = param["parameter_name"].clone();
    }
    out
}

fn media_link<'a>(param: &'a Value, kind: &str) -> Option<&'a str> {
    param
        .get(kind)
        .and_then(|m| m.get("link"))
        .and_then(Value::as_str)
        .filter(|l| !l.is_empty())
}

/// `value || ""`.
fn or_empty(value: &Value) -> Value {
    let falsy = match value {
        Value::Null => true,
        Value::Bool(b) => !b,
        Value::Number(n) => n.as_f64() == Some(0.0),
        Value::String(t) => t.is_empty(),
        _ => false,
    };
    if falsy { json!("") } else { value.clone() }
}

fn header_parameter(param: &Value) -> Value {
    let kind = s(param, "type");
    match kind {
        "text" if !s(param, "text").is_empty() => text_parameter(param),
        "image" | "video" | "document" if media_link(param, kind).is_some() => {
            json!({ "type": kind, kind: { "link": media_link(param, kind) } })
        }
        "location" if param.get("location").is_some_and(Value::is_object) => {
            let l = &param["location"];
            json!({ "type": "location", "location": {
                "latitude": or_empty(&l["latitude"]), "longitude": or_empty(&l["longitude"]),
                "name": s(l, "name"), "address": s(l, "address") } })
        }
        _ => json!({ "type": "text", "text": "" }),
    }
}

fn non_empty_array<'a>(v: &'a Value, k: &str) -> Option<&'a Vec<Value>> {
    v.get(k).and_then(Value::as_array).filter(|a| !a.is_empty())
}

/// `buildCarouselComponent`.
fn carousel_component(cards: &[Value], ctx: &BuildCtx) -> ChannelResult<Value> {
    let mut out = Vec::with_capacity(cards.len());
    for card in cards {
        let mut components = Vec::new();
        if let Some(header) = non_empty_array(card, "header") {
            let h = &header[0];
            let kind = s(h, "type");
            if matches!(kind, "image" | "video")
                && let Some(link) = media_link(h, kind)
            {
                components.push(json!({ "type": "header",
                    "parameters": [{ "type": kind, kind: { "link": link } }] }));
            }
        }
        if let Some(body) = non_empty_array(card, "body") {
            components.push(json!({ "type": "body", "parameters": body.iter()
                .map(|p| json!({ "type": "text", "text": p["text"] })).collect::<Vec<_>>() }));
        }
        if let Some(buttons) = non_empty_array(card, "button") {
            components.extend(button_components(buttons, ctx, card.get("card_index"))?);
        }
        out.push(json!({ "card_index": card["card_index"], "components": components }));
    }
    Ok(json!({ "type": "carousel", "cards": out }))
}

/// `buildTemplateComponents`.
fn template_components(params: &Value, ctx: &BuildCtx) -> ChannelResult<Vec<Value>> {
    let mut components = Vec::new();
    if let Some(header) = non_empty_array(params, "header") {
        components.push(json!({ "type": "header",
            "parameters": header.iter().map(header_parameter).collect::<Vec<_>>() }));
    }
    if let Some(body) = non_empty_array(params, "body") {
        components.push(json!({ "type": "body",
            "parameters": body.iter().map(text_parameter).collect::<Vec<_>>() }));
    }
    if let Some(buttons) = non_empty_array(params, "button") {
        components.extend(button_components(buttons, ctx, None)?);
    }
    if let Some(cards) = non_empty_array(params, "carousel") {
        components.push(carousel_component(cards, ctx)?);
    }
    if let Some(offer) = params.get("limited_time_offer").filter(|o| !o.is_null()) {
        components.push(json!({ "type": "limited_time_offer", "parameters": [{
            "type": "limited_time_offer",
            "limited_time_offer": { "expiration_time_ms": offer["expiration_time_ms"] } }] }));
    }
    Ok(components)
}

/// `convertFlowStepWaTemplate`: the raw `template` message.
pub fn convert_wa_template(data: &SendFlowStep, pix_redirect: bool) -> ChannelResult<WaMessage> {
    let template = &data.step["template"];
    let step_id = s(&data.step, "id");
    let ctx = BuildCtx {
        flow_id: &data.flow_id,
        flow_version_id: data.flow_version_id.as_deref(),
        step_id,
        broadcast_id: extract_metadata("broadcastId", data.metadata.as_ref()),
        pix_redirect,
    };
    let components = template_components(&template["params"], &ctx)?;
    Ok(WaMessage::raw(
        "template",
        json!({ "name": template["name"], "language": { "code": template["language"] },
            "components": components }),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn body_and_named_parameters() {
        let ctx = BuildCtx {
            flow_id: "",
            flow_version_id: None,
            step_id: "s",
            broadcast_id: None,
            pix_redirect: false,
        };
        let out = template_components(
            &json!({ "body": [{ "type": "text", "text": "Grace" }, { "type": "text", "text": "A1", "parameter_name": "order_id" }] }),
            &ctx,
        )
        .unwrap();
        assert_eq!(
            out,
            vec![json!({ "type": "body", "parameters": [
                { "type": "text", "text": "Grace" },
                { "type": "text", "text": "A1", "parameter_name": "order_id" }] })]
        );
    }

    #[test]
    fn button_dedupe_keeps_first_with_content_and_drops_blank_quick_replies() {
        let resolved = resolve_button_params(&[
            json!({ "sub_type": "url", "index": 0, "text": "" }),
            json!({ "sub_type": "url", "index": 0, "text": "abc" }),
            json!({ "sub_type": "quick_reply", "index": 1, "payload": "" }),
            Value::Null,
            json!({ "sub_type": "copy_code", "coupon_code": "X" }),
        ]);
        let picked: Vec<(String, i64)> = resolved
            .iter()
            .map(|(p, i)| (sub_type(p).to_owned(), *i))
            .collect();
        assert_eq!(picked, vec![("url".into(), 0), ("copy_code".into(), 4)]);
        assert_eq!(resolved[0].0["text"], "abc");
    }

    #[test]
    fn payment_request_parameter_with_amount() {
        let out = payment_request_parameter(
            &json!({ "method": "pix", "code": "000201", "amount": "12.5" }),
            true,
        )
        .unwrap();
        assert_eq!(
            out,
            json!({ "type": "action", "action": { "payment_request": {
                "payment_setting": { "type": "pix_dynamic_code", "pix_dynamic_code": { "code": "000201", "flow_type": "APPSWITCH" } },
                "currency": "BRL", "total_amount": { "value": 1250, "offset": 100 } } } })
        );
        assert!(
            payment_request_parameter(
                &json!({ "method": "pix", "code": "x", "amount": "abc" }),
                false
            )
            .is_err()
        );
    }

    #[test]
    fn flow_button_token_from_the_flow_step() {
        let ctx = BuildCtx {
            flow_id: "11",
            flow_version_id: Some("12"),
            step_id: "13",
            broadcast_id: None,
            pix_redirect: false,
        };
        let out = button_parameter(&json!({ "sub_type": "flow", "index": 0 }), &ctx, None).unwrap();
        assert_eq!(out["action"]["flow_token"], "watf:s:11:12:13:0:");
    }
}
