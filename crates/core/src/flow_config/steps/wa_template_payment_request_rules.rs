//! Port of `packages/flow-config/src/steps/wa-template-payment-request-rules.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

pub fn wa_payment_setting_type_by_method() -> &'static Value {
    static V: LazyLock<Value> = LazyLock::new(|| {
        serde_json::from_str(r#"{"pix":"pix_dynamic_code","boleto":"boleto","payment_link":"payment_link","card_on_file":"offsite_card_pay"}"#).expect("valid JSON")
    });
    &V
}

const METHODS: [&str; 4] = ["pix", "boleto", "payment_link", "card_on_file"];

fn setting_type_of(method: &str) -> &'static str {
    match method {
        "pix" => "pix_dynamic_code",
        "boleto" => "boleto",
        "payment_link" => "payment_link",
        _ => "offsite_card_pay",
    }
}

fn label_of(method: &str) -> &'static str {
    match method {
        "pix" => "Copy Pix code",
        "boleto" => "Copy Boleto code",
        "payment_link" => "Open payment link",
        _ => "Review payment",
    }
}

fn normalize_label(label: Option<&str>) -> String {
    crate::js::to_lower(crate::js::trim(label.unwrap_or("")))
}

fn find_method_by_label(label: Option<&str>) -> Option<&'static str> {
    let normalized = normalize_label(label);
    METHODS
        .into_iter()
        .find(|m| normalize_label(Some(label_of(m))) == normalized)
}

/// `resolveWaPaymentRequestMethod(button)`: a known `payment_setting.type`
/// decides unless the fixed English label names another method; without a
/// type only the fixed labels are trusted.
pub fn resolve_wa_payment_request_method(button: &Value) -> Option<&'static str> {
    let label = button["text"].as_str();
    let Some(setting_type) = button["payment_setting"]
        .get("type")
        .and_then(Value::as_str)
    else {
        return find_method_by_label(label);
    };
    let normalized = crate::js::to_lower(crate::js::trim(setting_type));
    let method = METHODS
        .into_iter()
        .find(|m| setting_type_of(m) == normalized)?;
    match find_method_by_label(label) {
        None => Some(method),
        Some(l) if l == method => Some(method),
        Some(_) => None,
    }
}

/// `validatePaymentRequestParams(params, ctx, basePath)`.
pub fn validate_payment_request_params(
    params: &Value,
    ctx: &mut RefineCtx,
    base_path: &[PathSegment],
) {
    for (position, button) in arr_field(params, "button").iter().enumerate() {
        let Some(request) = button.get("payment_request").filter(|r| truthy(Some(r))) else {
            continue;
        };
        for issue in collect_payment_request_issues(
            request,
            CollectPaymentRequestIssuesOptions {
                allow_variables: true,
            },
        ) {
            let message = if issue.kind == "required" {
                "waTemplatePaymentRequestIncomplete"
            } else {
                "waTemplatePaymentRequestInvalid"
            };
            let mut path = base_path.to_vec();
            path.extend(path![
                "button",
                position,
                "payment_request",
                issue.field.as_str()
            ]);
            ctx.issue(path, message);
        }
    }
}

/// `findTemplatePaymentButtons(components)`: (index, method) per payment button.
pub fn find_template_payment_buttons(components: &[Value]) -> Vec<(usize, Option<&'static str>)> {
    components
        .iter()
        .filter(|c| c["type"] == "BUTTONS")
        .flat_map(|c| arr_field(c, "buttons").iter())
        .enumerate()
        .filter(|(_, b)| crate::js::to_upper(str_field(b, "type")) == "PAYMENT_REQUEST")
        .map(|(index, b)| (index, resolve_wa_payment_request_method(b)))
        .collect()
}

/// `findPaymentRequestMismatches(params, components)`.
pub fn find_payment_request_mismatches(params: &Value, components: &[Value]) -> Vec<Value> {
    let template_buttons = find_template_payment_buttons(components);
    let sent: Vec<&Value> = arr_field(params, "button")
        .iter()
        .filter(|b| {
            b["sub_type"] == "payment_request" && b.get("index").is_some_and(|i| !i.is_null())
        })
        .collect();
    let index_of = |entry: &Value| entry["index"].as_u64().map(|i| i as usize);
    let mut out = Vec::new();
    for (index, method) in &template_buttons {
        let entries: Vec<&&Value> = sent
            .iter()
            .filter(|e| index_of(e) == Some(*index))
            .collect();
        let reason = match method {
            None => Some("unsupported_template_button"),
            Some(_) if entries.is_empty() => Some("missing"),
            Some(_) if entries.len() > 1 => Some("duplicate"),
            Some(m) => (entries[0]["payment_request"]["method"] != *m).then_some("method_mismatch"),
        };
        if let Some(reason) = reason {
            out.push(json!({ "index": index, "reason": reason }));
        }
    }
    for entry in &sent {
        if !template_buttons
            .iter()
            .any(|(i, _)| Some(*i) == index_of(entry))
        {
            out.push(json!({ "index": entry["index"], "reason": "extra" }));
        }
    }
    out
}
