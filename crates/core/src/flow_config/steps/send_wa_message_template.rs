//! Port of `packages/flow-config/src/steps/send-wa-message-template.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

schema!(pub fn button_sub_types = z::enum_(&["url", "quick_reply", "copy_code", "flow", "catalog", "mpm", "payment_request"]));

schema!(pub fn wa_template_button_param_schema = z::object(vec![("sub_type", button_sub_types().optional().describe("Button kind: url, quick_reply, copy_code, flow, catalog, mpm or payment_request.")),
("index", z::number().optional().describe("Zero-based position of the button in the template.")),
("text", z::string().optional().describe("Variable value for a `url` button (the dynamic URL suffix).")),
("coupon_code", z::string().optional().describe("Coupon code for a `copy_code` button.")),
("payload", z::string().optional().describe("Payload returned when a `quick_reply` button is tapped.")),
("flow_token", z::string().optional().describe("Token that identifies the WhatsApp Flow session (`flow` button).")),
("flow_action_data", z::record(z::string(), z::unknown()).optional().describe("Initial data passed to the first screen of the WhatsApp Flow.")),
("flowSourceId", z::string().optional().describe("Meta id of the WhatsApp Flow, from `whatsappFlows.list`.")),
("navigateScreenId", z::string().optional().describe("Screen id the Flow opens on, from `whatsappFlows.getScreens`.")),
("fieldMappings", z::array(whatsapp_flow_field_mapping_schema()).optional().describe("Maps Flow response fields to custom fields.")),
("thumbnail_product_retailer_id", z::string().optional().describe("Retailer id of the product shown as thumbnail (`catalog` button).")),
("sections", z::array(z::object(vec![("title", z::string().optional().describe("Section title (`mpm` button).")),
("product_items", z::array(z::object(vec![("product_retailer_id", z::string().describe("Retailer id of a catalog product."))])).optional().describe("Products listed in the section."))])).optional().describe("Product sections of a multi-product (`mpm`) button.")),
("payment_request", payment_request_schema().optional().describe("Payment data of a `payment_request` button. `method` must match the template button (Pix, Boleto, payment link or saved card)."))]).super_refine(wa_template_button_param_schema_super_refine));

schema!(pub fn wa_template_carousel_card_schema = z::object(vec![("card_index", z::number().describe("Zero-based position of the card in the carousel.")),
("header", z::array(z::object(vec![("type", z::enum_(&["image", "video"]).describe("Card media kind.")),
("image", z::object(vec![("link", z::string().describe("Public https URL of the card image."))]).optional().describe("Image for an `image` card header.")),
("video", z::object(vec![("link", z::string().describe("Public https URL of the card video."))]).optional().describe("Video for a `video` card header."))])).optional().describe("Card header media.")),
("body", z::array(z::object(vec![("type", z::literal(json!("text")).optional().describe("Always `text`.")),
("text", z::string().describe("Value that replaces the card variable."))])).optional().describe("Card body variables in order.")),
("button", z::array(wa_template_button_param_schema().nullable()).transform(wa_template_carousel_card_schema_button_transform).optional().describe("Parameters for the card's dynamic buttons, by `index`."))]).super_refine(wa_template_carousel_card_schema_super_refine));

schema!(pub fn wa_template_params_schema = z::object(vec![("header", z::array(z::object(vec![("type", z::enum_(&["text", "image", "video", "document", "location"]).describe("Header parameter kind; fill the matching field.")),
("text", z::string().optional().describe("Value for a `text` header variable.")),
("parameter_name", z::string().optional().describe("Placeholder name, only for NAMED templates ({{order_id}}).")),
("image", z::object(vec![("link", z::string().describe("Public https URL of the image."))]).optional().describe("Media for an `image` header.")),
("video", z::object(vec![("link", z::string().describe("Public https URL of the video."))]).optional().describe("Media for a `video` header.")),
("document", z::object(vec![("link", z::string().describe("Public https URL of the document."))]).optional().describe("Media for a `document` header.")),
("location", z::object(vec![("latitude", z::string().optional().describe("Latitude, e.g. `10.7769`.")),
("longitude", z::string().optional().describe("Longitude, e.g. `106.7009`.")),
("name", z::string().optional().describe("Place name shown to the customer.")),
("address", z::string().optional().describe("Street address."))]).optional().describe("Place for a `location` header."))])).optional().describe("Header parameters; one entry for a templated header.")),
("body", z::array(z::object(vec![("type", z::literal(json!("text")).optional().describe("Always `text` for a body variable.")),
("text", z::string().describe("Value that replaces the variable.")),
("parameter_name", z::string().optional().describe("Placeholder name, only for NAMED templates; omit for positional `{{1}}`."))])).optional().describe("Body variables in order: {{1}}, {{2}}, ...")),
("button", z::array(wa_template_button_param_schema().nullable()).transform(wa_template_carousel_card_schema_button_transform).optional().describe("Parameters for the template's dynamic buttons, by `index`.")),
("carousel", z::array(wa_template_carousel_card_schema()).optional().describe("Cards of a carousel template, by `card_index`.")),
("limited_time_offer", z::object(vec![("expiration_time_ms", z::number().describe("Offer expiry as a Unix timestamp in milliseconds."))]).optional().describe("Only for a limited-time-offer template."))]));

pub const WA_TEMPLATE_STATUS_BRANCHES: &[&str] = &["delivered", "failed"];

pub fn wa_template_status_button_labels() -> &'static Value {
    static V: LazyLock<Value> = LazyLock::new(|| {
        serde_json::from_str(r#"{"delivered":"Delivered","failed":"Failed"}"#).expect("valid JSON")
    });
    &V
}

schema!(pub fn send_wa_template_message_step_schema = z::object(vec![("id", z::string().regex(r#"^\d+$"#, "").describe("Step id (numeric string), unique within the flow.")),
("nodeId", z::string().optional().describe("Id of the node containing this step. Optional.")),
("stepType", z::literal(json!("sendWaTemplateMessage"))),
("template", z::object(vec![("id", z::string().trim().min(1)),
("name", z::string()),
("language", z::string()),
("inboxId", z::string().regex(r#"^\d+$"#, "").nullable().optional()),
("params", wa_template_params_schema())])),
("buttons", z::array(button_step_schema()).super_refine(refine_no_open_mini_app_buttons).super_refine(refine_reply_only_buttons).default(json!([])).transform(send_wa_template_message_step_schema_buttons_transform))]));

/// `sendWaTemplateMessageStepDefaultFn(props)`.
pub fn send_wa_template_message_step_default_fn(props: &Value) -> Value {
    template_step_default(
        props,
        json!({ "id": "", "name": "", "language": "", "inboxId": null, "params": {} }),
        json!([
            button_step_default_fn(&json!({ "label": "Delivered" })),
            button_step_default_fn(&json!({ "label": "Failed" })),
        ]),
        "sendWaTemplateMessage",
    )
}

fn wa_template_button_param_schema_super_refine(button: &Value, ctx: &mut RefineCtx) {
    let is_payment_button = button["sub_type"] == "payment_request";
    if is_payment_button != button.get("payment_request").is_some() {
        ctx.issue(
            path!["payment_request"],
            "waTemplatePaymentRequestMisplaced",
        );
    }
    if is_payment_button && button.get("index").is_none() {
        ctx.issue(path!["index"], "waTemplatePaymentRequestMisplaced");
    }
}

fn wa_template_carousel_card_schema_button_transform(
    buttons: Value,
    _ctx: &mut RefineCtx,
) -> Value {
    match buttons {
        Value::Array(items) => Value::Array(items.into_iter().filter(|b| !b.is_null()).collect()),
        other => other,
    }
}

fn wa_template_carousel_card_schema_super_refine(card: &Value, ctx: &mut RefineCtx) {
    if arr_field(card, "button")
        .iter()
        .any(|b| b.get("payment_request").is_some())
    {
        ctx.issue(path!["button"], "waTemplatePaymentRequestMisplaced");
    }
}

fn send_wa_template_message_step_schema_buttons_transform(
    buttons: Value,
    _ctx: &mut RefineCtx,
) -> Value {
    if buttons.as_array().is_some_and(Vec::is_empty) {
        return json!([
            button_step_default_fn(&json!({ "label": "Delivered" })),
            button_step_default_fn(&json!({ "label": "Failed" })),
        ]);
    }
    buttons
}

/// `toOptionalString`: `String(value)` unless null/undefined.
pub fn to_optional_string(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::Null => None,
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(crate::js::number_to_string(n.as_f64().unwrap_or(0.0))),
        other => Some(crate::js::string_of(other)),
    }
}

fn wa_placeholder_re() -> &'static regex::Regex {
    static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    RE.get_or_init(|| regex::Regex::new(r"\{\{([0-9]+|[a-zA-Z_]+)\}\}").expect("placeholder regex"))
}

/// `isNamedTemplateToken`: not purely digits.
pub fn is_named_template_token(token: &str) -> bool {
    token.is_empty() || !token.bytes().all(|b| b.is_ascii_digit())
}

fn extract_text_params(text: &str) -> Vec<Value> {
    wa_placeholder_re()
        .captures_iter(text)
        .map(|c| {
            let token = &c[1];
            if is_named_template_token(token) {
                json!({ "type": "text", "text": "", "parameter_name": token })
            } else {
                json!({ "type": "text", "text": "" })
            }
        })
        .collect()
}

fn extract_button_params(buttons: &[Value], allow_payment_request: bool) -> Vec<Value> {
    let mut out = Vec::new();
    for (idx, button) in buttons.iter().enumerate() {
        let button_type = crate::js::to_upper(str_field(button, "type"));
        match button_type.as_str() {
            "URL" if str_field(button, "url").contains("{{1}}") => {
                out.push(json!({ "sub_type": "url", "index": idx, "text": "" }));
            }
            "COPY_CODE" => {
                out.push(json!({ "sub_type": "copy_code", "index": idx, "coupon_code": "" }))
            }
            "FLOW" => {
                let mut param = json!({ "sub_type": "flow", "index": idx, "fieldMappings": [] });
                if let Some(id) = to_optional_string(button.get("flow_id")) {
                    param["flowSourceId"] = json!(id);
                }
                if let Some(screen) = to_optional_string(button.get("navigate_screen")) {
                    param["navigateScreenId"] = json!(screen);
                }
                out.push(param);
            }
            "CATALOG" => out.push(
                json!({ "sub_type": "catalog", "index": idx, "thumbnail_product_retailer_id": "" }),
            ),
            "MPM" => out.push(json!({ "sub_type": "mpm", "index": idx, "sections": [] })),
            "PAYMENT_REQUEST" if allow_payment_request => {
                if let Some(method) = resolve_wa_payment_request_method(button) {
                    out.push(json!({
                        "sub_type": "payment_request",
                        "index": idx,
                        "payment_request": create_empty_payment_request(method),
                    }));
                }
            }
            _ => {}
        }
    }
    out
}

fn extract_carousel_params(cards: &[Value]) -> Vec<Value> {
    cards
        .iter()
        .map(|card| {
            let mut params = json!({ "card_index": card["card_index"] });
            for comp in arr_field(card, "components") {
                match comp["type"].as_str() {
                    Some("HEADER") => match comp["format"].as_str() {
                        Some("IMAGE") => {
                            params["header"] = json!([{ "type": "image", "image": { "link": "" } }])
                        }
                        Some("VIDEO") => {
                            params["header"] = json!([{ "type": "video", "video": { "link": "" } }])
                        }
                        _ => {}
                    },
                    Some("BODY") if truthy(comp.get("text")) => {
                        let count = wa_placeholder_re()
                            .find_iter(str_field(comp, "text"))
                            .count();
                        if count > 0 {
                            params["body"] = Value::Array(
                                (0..count)
                                    .map(|_| json!({ "type": "text", "text": "" }))
                                    .collect(),
                            );
                        }
                    }
                    Some("BUTTONS") if comp["buttons"].is_array() => {
                        params["button"] =
                            Value::Array(extract_button_params(arr_field(comp, "buttons"), false));
                    }
                    _ => {}
                }
            }
            params
        })
        .collect()
}

/// `extractTemplateParams(components)`: the blank send-time params a
/// template needs.
pub fn extract_template_params(components: &[Value]) -> Value {
    let mut params = Map::new();
    for component in components {
        match component["type"].as_str() {
            Some("HEADER") => {
                let format = component["format"].as_str().unwrap_or("");
                if format == "TEXT" && truthy(component.get("text")) {
                    let header = extract_text_params(str_field(component, "text"));
                    if !header.is_empty() {
                        params.insert("header".into(), Value::Array(header));
                    }
                } else if format == "LOCATION" {
                    params.insert(
                        "header".into(),
                        json!([{ "type": "location", "location": { "latitude": "", "longitude": "", "name": "", "address": "" } }]),
                    );
                } else if ["IMAGE", "VIDEO", "DOCUMENT"].contains(&format) {
                    let lower = format.to_lowercase();
                    let mut header = Map::new();
                    header.insert("type".into(), json!(lower));
                    header.insert(lower, json!({ "link": "" }));
                    params.insert("header".into(), json!([Value::Object(header)]));
                }
            }
            Some("BODY") if truthy(component.get("text")) => {
                let body = extract_text_params(str_field(component, "text"));
                if !body.is_empty() {
                    params.insert("body".into(), Value::Array(body));
                }
            }
            Some("BUTTONS") if component["buttons"].is_array() => {
                let buttons = extract_button_params(arr_field(component, "buttons"), true);
                if !buttons.is_empty() {
                    params.insert("button".into(), Value::Array(buttons));
                }
            }
            Some("CAROUSEL") if component["cards"].is_array() => {
                params.insert(
                    "carousel".into(),
                    Value::Array(extract_carousel_params(arr_field(component, "cards"))),
                );
            }
            Some("LIMITED_TIME_OFFER")
                if truthy(component["limited_time_offer"].get("has_expiration")) =>
            {
                params.insert(
                    "limited_time_offer".into(),
                    json!({ "expiration_time_ms": 0 }),
                );
            }
            _ => {}
        }
    }
    Value::Object(params)
}
