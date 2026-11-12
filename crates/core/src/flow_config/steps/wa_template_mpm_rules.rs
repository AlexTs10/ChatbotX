//! Port of `packages/flow-config/src/steps/wa-template-mpm-rules.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

pub fn wa_template_mpm_limits() -> &'static Value {
    static V: LazyLock<Value> = LazyLock::new(|| {
        serde_json::from_str(r#"{"maxSections":10,"maxProductsTotal":30}"#).expect("valid JSON")
    });
    &V
}

fn is_configured_retailer_id(value: &Value) -> bool {
    value
        .as_str()
        .is_some_and(|s| !crate::js::trim(s).is_empty())
}

/// `countMpmProducts(sections)`: configured product rows across sections.
pub fn count_mpm_products(sections: Option<&Value>) -> usize {
    sections
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|section| {
            arr_field(section, "product_items")
                .iter()
                .filter(|i| is_configured_retailer_id(&i["product_retailer_id"]))
                .count()
        })
        .sum()
}

fn section_has_blank_or_no_products(section: &Value) -> bool {
    let items = arr_field(section, "product_items");
    items.is_empty()
        || items
            .iter()
            .any(|i| !is_configured_retailer_id(&i["product_retailer_id"]))
}

fn mpm_buttons(buttons: &Value) -> Vec<&Value> {
    buttons
        .as_array()
        .into_iter()
        .flatten()
        .filter(|b| b["sub_type"] == "mpm")
        .collect()
}

/// `validateMpmParams(params, ctx, basePath)`.
pub fn validate_mpm_params(params: &Value, ctx: &mut RefineCtx, base_path: &[PathSegment]) {
    let mut targets: Vec<(Vec<PathSegment>, &Value)> = Vec::new();
    for button in mpm_buttons(&params["button"]) {
        let mut path = base_path.to_vec();
        path.push("button".into());
        targets.push((path, button));
    }
    for (card_index, card) in arr_field(params, "carousel").iter().enumerate() {
        for button in mpm_buttons(&card["button"]) {
            let mut path = base_path.to_vec();
            path.extend(path!["carousel", card_index, "button"]);
            targets.push((path, button));
        }
    }
    for (path, button) in targets {
        let sections = arr_field(button, "sections");
        let product_count = count_mpm_products(button.get("sections"));
        if product_count == 0 {
            ctx.issue(path, "waTemplateMpmNoProducts");
            continue;
        }
        if sections.iter().any(section_has_blank_or_no_products) {
            ctx.issue(path.clone(), "waTemplateMpmIncompleteProducts");
        }
        if sections.len() > 10 {
            ctx.issue(path.clone(), "waTemplateMpmTooManySections");
        }
        if product_count > 30 {
            ctx.issue(path, "waTemplateMpmTooManyProducts");
        }
    }
}
