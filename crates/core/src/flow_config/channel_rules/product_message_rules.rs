//! Port of `packages/flow-config/src/channel-rules/product-message-rules.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

pub const PRODUCT_MESSAGE_CONDITIONS: &[&str] =
    &["always", "never", "multipleProducts", "multipleSections"];

/// `productMessageConditionHolds[condition](context)`.
pub fn product_message_condition_holds(
    condition: &str,
    product_count: usize,
    section_count: usize,
) -> bool {
    match condition {
        "always" => true,
        "multipleProducts" => product_count > 1,
        "multipleSections" => section_count > 1,
        _ => false,
    }
}

fn has_text(value: &Value) -> bool {
    value
        .as_str()
        .is_some_and(|s| !crate::js::trim(s).is_empty())
}

fn evaluate_text(
    value: &Value,
    rule: Option<&Value>,
    counts: (usize, usize),
    path: Value,
) -> Option<Value> {
    let present = has_text(value);
    let Some(rule) = rule.filter(|r| r.is_object()) else {
        return present.then(|| json!({ "code": "sendProductsFieldNotAllowed", "path": path }));
    };
    let holds =
        |key: &str| product_message_condition_holds(str_field(rule, key), counts.0, counts.1);
    if present && !holds("allowedWhen") {
        return Some(json!({ "code": "sendProductsFieldNotAllowed", "path": path }));
    }
    if present
        && count_message_characters(Some(crate::js::trim(value.as_str().unwrap_or(""))))
            > rule["max"].as_u64().unwrap_or(0) as usize
    {
        return Some(json!({ "code": "sendProductsFieldTooLong", "path": path }));
    }
    if !present && holds("requiredWhen") {
        return Some(json!({ "code": "sendProductsFieldRequired", "path": path }));
    }
    None
}

/// `evaluateProductMessage(capability, shape)`: `[{ code, path }]`.
pub fn evaluate_product_message(capability: &Value, shape: &Value) -> Vec<Value> {
    let Some(rule) = capability["displays"]
        .get(str_field(shape, "display"))
        .filter(|r| r.is_object())
    else {
        return vec![json!({ "code": "sendProductsDisplayUnsupported", "path": ["display"] })];
    };
    let sections = arr_field(shape, "sections");
    let product_count: usize = sections
        .iter()
        .map(|s| s["productCount"].as_u64().unwrap_or(0) as usize)
        .sum();
    let section_count = sections.len();
    let counts = (product_count, section_count);
    let mut issues = Vec::new();
    let min = rule["minProducts"].as_u64().unwrap_or(0) as usize;
    let max = rule["maxProducts"].as_u64().unwrap_or(0) as usize;
    if product_count < min || product_count > max {
        issues.push(json!({ "code": "sendProductsProductCountOutOfRange", "path": ["sections"] }));
    }
    if section_count > rule["maxSections"].as_u64().unwrap_or(0) as usize {
        issues.push(json!({ "code": "sendProductsSectionCountOutOfRange", "path": ["sections"] }));
    }
    for field in ["header", "body", "footer"] {
        issues.extend(evaluate_text(
            &shape[field],
            rule.get(field),
            counts,
            json!([field]),
        ));
    }
    for (index, section) in sections.iter().enumerate() {
        issues.extend(evaluate_text(
            &section["title"],
            rule.get("sectionTitle"),
            counts,
            json!(["sections", index, "title"]),
        ));
    }
    issues
}

/// `toProductMessageShape(content)`.
pub fn to_product_message_shape(content: &Value) -> Value {
    let mut shape = json!({
        "display": content["display"],
        "sections": arr_field(content, "sections").iter().map(|s| {
            let mut out = json!({ "productCount": arr_field(s, "productIds").len() });
            if let Some(title) = s.get("title") {
                out["title"] = title.clone();
            }
            out
        }).collect::<Vec<_>>(),
    });
    for key in ["header", "body", "footer"] {
        if let Some(v) = content.get(key) {
            shape[key] = v.clone();
        }
    }
    shape
}

/// `refineProductMessage(capability)`.
pub fn refine_product_message(
    capability: Value,
) -> impl Fn(&Value, &mut RefineCtx) + Send + Sync + 'static {
    move |content, ctx| {
        for issue in evaluate_product_message(&capability, &to_product_message_shape(content)) {
            let path = issue["path"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|p| match p {
                    Value::Number(n) => PathSegment::Index(n.as_u64().unwrap_or(0) as usize),
                    other => PathSegment::Key(other.as_str().unwrap_or("").to_owned()),
                })
                .collect();
            ctx.issue(path, str_field(&issue, "code"));
        }
    }
}
