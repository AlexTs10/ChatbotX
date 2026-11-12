//! Port of `packages/flow-config/src/steps/wa-template-utils.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

pub const WA_TEMPLATE_STATUS_BUTTON_COUNT: usize = 2;

fn info(mut base: Value, card_index: Option<&Value>, param_index: Option<usize>) -> Value {
    if let Some(card) = card_index.filter(|c| !c.is_null()) {
        base["cardIndex"] = card.clone();
    }
    if let Some(index) = param_index {
        base["paramIndex"] = json!(index);
    }
    base
}

fn extract_button_parameter_infos(buttons: &[Value], card_index: Option<&Value>) -> Vec<Value> {
    let allow_payment_request = card_index.is_none();
    let mut params: Vec<Value> = Vec::new();
    for (button_idx, button) in buttons.iter().enumerate() {
        let button_type = crate::js::to_upper(str_field(button, "type"));
        let param_index = Some(params.len());
        let simple = |name: &str, sub: &str, format: Option<&str>| {
            let mut v = json!({ "type": "button", "index": 0, "paramName": name, "buttonIndex": button_idx, "buttonSubType": sub });
            if let Some(f) = format {
                v["format"] = json!(f);
            }
            v
        };
        match button_type.as_str() {
            "URL" if str_field(button, "url").contains("{{1}}") => {
                params.push(info(simple("1", "url", None), card_index, param_index));
            }
            "COPY_CODE" => params.push(info(
                simple("coupon_code", "copy_code", Some("coupon_code")),
                card_index,
                param_index,
            )),
            "FLOW" => {
                let mut v = simple("flow", "flow", Some("flow"));
                if let Some(id) = to_optional_string(button.get("flow_id")) {
                    v["flowSourceId"] = json!(id);
                }
                if let Some(screen) = to_optional_string(button.get("navigate_screen")) {
                    v["navigateScreenId"] = json!(screen);
                }
                params.push(info(v, card_index, param_index));
            }
            "CATALOG" => params.push(info(
                simple("catalog", "catalog", Some("catalog")),
                card_index,
                param_index,
            )),
            "MPM" => params.push(info(
                simple("mpm", "mpm", Some("mpm")),
                card_index,
                param_index,
            )),
            "PAYMENT_REQUEST" if allow_payment_request => {
                if let Some(method) = resolve_wa_payment_request_method(button) {
                    let mut v = simple(
                        "payment_request",
                        "payment_request",
                        Some("payment_request"),
                    );
                    v["paymentMethod"] = json!(method);
                    params.push(info(v, card_index, param_index));
                }
            }
            _ => {}
        }
    }
    params
}

fn placeholder_names(text: &str) -> Vec<String> {
    static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let re = RE.get_or_init(|| {
        regex::Regex::new(r"\{\{([0-9]+|[a-zA-Z_]+)\}\}").expect("placeholder regex")
    });
    re.captures_iter(text).map(|c| c[1].to_owned()).collect()
}

fn node_steps(node: &Value) -> &[Value] {
    let details = &node["data"]["details"];
    if !details.is_object() {
        return &[];
    }
    arr_field(details, "steps")
}

/// `findSendWaTemplateStep(nodes, stepId)`.
pub fn find_send_wa_template_step(nodes: &[Value], step_id: &str) -> Option<Value> {
    nodes
        .iter()
        .flat_map(node_steps)
        .find(|step| step["stepType"] == "sendWaTemplateMessage" && step["id"] == step_id)
        .cloned()
}

fn extract_carousel_parameter_infos(cards: &[Value]) -> Vec<Value> {
    let mut params = Vec::new();
    for card in cards {
        let card_index = &card["card_index"];
        for comp in arr_field(card, "components") {
            match comp["type"].as_str() {
                Some("HEADER") => {
                    if let Some(f @ ("IMAGE" | "VIDEO")) = comp["format"].as_str() {
                        let lower = f.to_lowercase();
                        params.push(json!({
                            "type": "carousel", "index": 0, "paramName": lower, "format": lower, "cardIndex": card_index,
                        }));
                    }
                }
                Some("BODY") if truthy(comp.get("text")) => {
                    for (idx, name) in placeholder_names(str_field(comp, "text"))
                        .into_iter()
                        .enumerate()
                    {
                        params.push(json!({
                            "type": "carousel", "index": idx, "paramName": name, "format": "text", "cardIndex": card_index,
                        }));
                    }
                }
                Some("BUTTONS") if comp["buttons"].is_array() => {
                    params.extend(extract_button_parameter_infos(
                        arr_field(comp, "buttons"),
                        Some(card_index),
                    ));
                }
                _ => {}
            }
        }
    }
    params
}

/// `extractTemplateQuickReplyButtons`: quick replies with their template index.
pub fn extract_template_quick_reply_buttons(components: &[Value]) -> Vec<Value> {
    let Some(component) = components
        .iter()
        .find(|c| c["type"] == "BUTTONS" && truthy(c.get("buttons")))
    else {
        return Vec::new();
    };
    arr_field(component, "buttons")
        .iter()
        .enumerate()
        .filter(|(_, b)| crate::js::to_upper(str_field(b, "type")) == "QUICK_REPLY")
        .map(|(index, b)| json!({ "templateButtonIndex": index, "text": b["text"] }))
        .collect()
}

/// `splitWaTemplateStepButtons`: (status buttons, quick reply buttons).
pub fn split_wa_template_step_buttons(buttons: &[Value]) -> (&[Value], &[Value]) {
    let split = buttons.len().min(WA_TEMPLATE_STATUS_BUTTON_COUNT);
    buttons.split_at(split)
}

/// `seedWaTemplateStepButtons(existing, components)`.
pub fn seed_wa_template_step_buttons(existing: &[Value], components: &[Value]) -> Vec<Value> {
    let (status, previous) = split_wa_template_step_buttons(existing);
    let template_buttons: Vec<Value> = extract_template_quick_reply_buttons(components)
        .iter()
        .map(|qr| {
            let text = str_field(qr, "text");
            button_step_default_fn(
                &json!({ "label": crate::js::slice_utf16(text, 0, BUTTON_LABEL_MAX) }),
            )
        })
        .collect();
    let mut out = status.to_vec();
    out.extend(merge_template_buttons_with_existing(
        &template_buttons,
        previous,
    ));
    out
}

/// `bindWaTemplateQuickReplyButtons(components, stepButtons)`.
pub fn bind_wa_template_quick_reply_buttons(
    components: &[Value],
    step_buttons: &[Value],
) -> Vec<Value> {
    let (_, quick_replies) = split_wa_template_step_buttons(step_buttons);
    if quick_replies.is_empty() {
        return Vec::new();
    }
    extract_template_quick_reply_buttons(components)
        .iter()
        .enumerate()
        .filter_map(|(position, qr)| {
            let step_button = quick_replies.get(position)?;
            Some(json!({ "templateButtonIndex": qr["templateButtonIndex"], "stepButton": step_button }))
        })
        .collect()
}

/// `extractParameterInfos(components)`: the editor's parameter rows.
pub fn extract_parameter_infos(components: &[Value]) -> Vec<Value> {
    let mut params = Vec::new();
    for component in components {
        match component["type"].as_str() {
            Some("HEADER") => {
                let format = component["format"].as_str().unwrap_or("");
                if format == "TEXT" && truthy(component.get("text")) {
                    for (idx, name) in placeholder_names(str_field(component, "text"))
                        .into_iter()
                        .enumerate()
                    {
                        params.push(json!({ "type": "header", "index": idx, "paramName": name, "format": "text" }));
                    }
                } else if format == "LOCATION" {
                    params.push(json!({ "type": "header", "index": 0, "paramName": "location", "format": "location" }));
                } else if ["IMAGE", "VIDEO", "DOCUMENT"].contains(&format) {
                    params.push(json!({ "type": "header", "index": 0, "paramName": "1", "format": format.to_lowercase() }));
                }
            }
            Some("BODY") if truthy(component.get("text")) => {
                for (idx, name) in placeholder_names(str_field(component, "text"))
                    .into_iter()
                    .enumerate()
                {
                    params.push(json!({ "type": "body", "index": idx, "paramName": name }));
                }
            }
            Some("BUTTONS") if component["buttons"].is_array() => {
                params.extend(extract_button_parameter_infos(
                    arr_field(component, "buttons"),
                    None,
                ));
            }
            Some("CAROUSEL") if component["cards"].is_array() => {
                params.extend(extract_carousel_parameter_infos(arr_field(
                    component, "cards",
                )));
            }
            Some("LIMITED_TIME_OFFER")
                if truthy(component["limited_time_offer"].get("has_expiration")) =>
            {
                params.push(json!({
                    "type": "limited_time_offer", "index": 0, "paramName": "expiration_time_ms", "format": "limited_time_offer",
                }));
            }
            _ => {}
        }
    }
    params
}
