//! Port of `packages/flow-config/src/steps/template-parameter-values.ts`.

use std::collections::BTreeMap;
use std::sync::OnceLock;

use crate::flow_config::prelude::*;

schema!(pub fn template_parameter_kinds = z::enum_(&["text", "image", "video", "document", "latitude", "longitude", "location_name", "location_address", "url_suffix", "coupon_code", "catalog_product", "expiration_ms", "pix_code", "boleto_digitable_line", "payment_link_url", "payment_amount", "card_last_four_digits", "card_credential_id"]));

/// `TemplateParameterSpec`: one value the caller has to fill.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TemplateParameterSpec {
    /// The key to send in `templateParams`.
    pub key: String,
    /// `header` | `body` | `button` | `carousel` | `limited_time_offer`.
    pub component: String,
    pub kind: String,
    pub required: bool,
    /// The placeholder as written in the template, e.g. `{{1}}`.
    pub placeholder: Option<String>,
}

/// `AppliedTemplateParameters`.
#[derive(Debug, Clone, PartialEq)]
pub struct AppliedTemplateParameters {
    pub params: Value,
    /// Required keys the caller did not send (or sent empty).
    pub missing: Vec<String>,
    /// Keys the template does not have.
    pub unknown: Vec<String>,
    /// Keys whose value has the wrong shape, e.g. a non-numeric expiry.
    pub invalid: Vec<String>,
    /// Parts the flat keys cannot express (multi-product buttons).
    pub unsupported: Vec<String>,
}

/// `TemplateParameterValues`.
pub type TemplateParameterValues = BTreeMap<String, String>;

/// Where a slot writes: a top-level params array or one of a carousel card's.
#[derive(Clone)]
enum Target {
    Top(&'static str),
    Card {
        card_index: Value,
        key: &'static str,
    },
}

fn target_array<'a>(params: &'a mut Value, target: &Target) -> Option<&'a mut Vec<Value>> {
    match target {
        Target::Top(key) => params.get_mut(*key)?.as_array_mut(),
        Target::Card { card_index, key } => params
            .get_mut("carousel")?
            .as_array_mut()?
            .iter_mut()
            .find(|entry| entry.get("card_index") == Some(card_index))?
            .get_mut(*key)?
            .as_array_mut(),
    }
}

fn button_entry<'a>(
    params: &'a mut Value,
    target: &Target,
    button_index: usize,
) -> Option<&'a mut Value> {
    target_array(params, target)?
        .iter_mut()
        .find(|entry| entry.get("index").and_then(Value::as_u64) == Some(button_index as u64))
}

#[derive(Clone)]
enum Apply {
    /// Every occurrence of `token` in `tokens` gets `entry.text = value`.
    Text {
        target: Target,
        tokens: Vec<String>,
        token: String,
    },
    /// `entry[0][format] = { link: value }`.
    Media { target: Target, format: String },
    /// `header[0].location[field] = value`.
    Location { field: &'static str },
    /// The button entry's `field = value`.
    ButtonField {
        target: Target,
        button_index: usize,
        field: &'static str,
    },
    /// The button entry's `payment_request[field] = value`, validated.
    Payment {
        target: Target,
        button_index: usize,
        field: String,
    },
    /// `limited_time_offer = { expiration_time_ms }`.
    Offer,
}

struct Slot {
    spec: TemplateParameterSpec,
    apply: Apply,
}

fn spec(
    key: String,
    component: &str,
    kind: &str,
    required: bool,
    placeholder: Option<String>,
) -> TemplateParameterSpec {
    TemplateParameterSpec {
        key,
        component: component.to_string(),
        kind: kind.to_string(),
        required,
        placeholder,
    }
}

/// Writes the value into `params`; `false` when the value is invalid.
fn apply_slot(apply: &Apply, params: &mut Value, value: &str) -> bool {
    match apply {
        Apply::Text {
            target,
            tokens,
            token,
        } => {
            for (index, candidate) in tokens.iter().enumerate() {
                if candidate == token
                    && let Some(entry) =
                        target_array(params, target).and_then(|items| items.get_mut(index))
                    && let Some(object) = entry.as_object_mut()
                {
                    object.insert("text".into(), json!(value));
                }
            }
            true
        }
        Apply::Media { target, format } => {
            if let Some(object) = target_array(params, target)
                .and_then(|items| items.first_mut())
                .and_then(Value::as_object_mut)
            {
                object.insert(format.clone(), json!({ "link": value }));
            }
            true
        }
        Apply::Location { field } => {
            if let Some(location) = target_array(params, &Target::Top("header"))
                .and_then(|items| items.first_mut())
                .and_then(|entry| entry.get_mut("location"))
                .and_then(Value::as_object_mut)
            {
                location.insert((*field).to_string(), json!(value));
            }
            true
        }
        Apply::ButtonField {
            target,
            button_index,
            field,
        } => {
            if let Some(object) =
                button_entry(params, target, *button_index).and_then(Value::as_object_mut)
            {
                object.insert((*field).to_string(), json!(value));
            }
            true
        }
        Apply::Payment {
            target,
            button_index,
            field,
        } => {
            let Some(entry) = button_entry(params, target, *button_index) else {
                return true;
            };
            let Some(request) = entry.get("payment_request").filter(|r| truthy(Some(r))) else {
                return true;
            };
            let mut candidate = request.as_object().cloned().unwrap_or_default();
            candidate.insert(field.clone(), json!(value));
            let next = payment_request_schema().safe_parse(&Value::Object(candidate));
            let Some(data) = next.data.filter(|_| next.success) else {
                return false;
            };
            let has_invalid_field = collect_payment_request_issues(
                &data,
                CollectPaymentRequestIssuesOptions {
                    allow_variables: true,
                },
            )
            .iter()
            .any(|issue| issue.field == *field && issue.kind == "invalid");
            if has_invalid_field {
                return false;
            }
            if let Some(object) = entry.as_object_mut() {
                object.insert("payment_request".into(), data);
            }
            true
        }
        Apply::Offer => {
            let expiration = crate::js::to_number(value);
            if !(expiration.is_finite() && expiration.fract() == 0.0 && expiration > 0.0) {
                return false;
            }
            if let Some(object) = params.as_object_mut() {
                object.insert(
                    "limited_time_offer".into(),
                    json!({ "expiration_time_ms": num_value(expiration) }),
                );
            }
            true
        }
    }
}

fn placeholder_tokens(text: Option<&str>) -> Vec<String> {
    static RE: OnceLock<regex::Regex> = OnceLock::new();
    let re =
        RE.get_or_init(|| regex::Regex::new(r"\{\{([0-9]+|[a-zA-Z_]+)\}\}").expect("valid regex"));
    text.map(|text| re.captures_iter(text).map(|c| c[1].to_string()).collect())
        .unwrap_or_default()
}

/// One slot per distinct token; a token used twice fills every occurrence.
fn text_slots(prefix: &str, component: &str, text: Option<&str>, target: Target) -> Vec<Slot> {
    let tokens = placeholder_tokens(text);
    let mut distinct: Vec<&String> = Vec::new();
    for token in &tokens {
        if !distinct.contains(&token) {
            distinct.push(token);
        }
    }
    distinct
        .into_iter()
        .map(|token| Slot {
            spec: spec(
                format!("{prefix}.{token}"),
                component,
                "text",
                true,
                Some(format!("{{{{{token}}}}}")),
            ),
            apply: Apply::Text {
                target: target.clone(),
                tokens: tokens.clone(),
                token: token.clone(),
            },
        })
        .collect()
}

/// Flat-key shape of each payment field: `(kind, key suffix)`.
fn payment_field_slot_config(field: &str) -> (&'static str, &'static str) {
    match field {
        "code" => ("pix_code", ""),
        "digitableLine" => ("boleto_digitable_line", ""),
        "url" => ("payment_link_url", ""),
        "lastFourDigits" => ("card_last_four_digits", ""),
        "credentialId" => ("card_credential_id", ".credential_id"),
        _ => ("payment_amount", ".amount"),
    }
}

/// The flat keys of one payment request button (the method comes from the template).
fn payment_request_slots(
    key: &str,
    component: &str,
    button: &Value,
    target: &Target,
    button_index: usize,
    unsupported: &mut Vec<String>,
) -> Vec<Slot> {
    let Some(method) = resolve_wa_payment_request_method(button) else {
        unsupported.push(key.to_string());
        return Vec::new();
    };
    let required_fields: Vec<String> =
        arr_field(&payment_request_method_rules()[method], "requiredFields")
            .iter()
            .filter_map(|f| f.as_str().map(str::to_string))
            .collect();
    payment_request_field_names(method)
        .into_iter()
        .map(|field| {
            let (kind, key_suffix) = payment_field_slot_config(&field);
            Slot {
                spec: spec(
                    format!("{key}{key_suffix}"),
                    component,
                    kind,
                    required_fields.contains(&field),
                    None,
                ),
                apply: Apply::Payment {
                    target: target.clone(),
                    button_index,
                    field,
                },
            }
        })
        .collect()
}

/// Buttons are stored densely: only parameterized buttons get an entry.
fn wa_button_slots(
    prefix: &str,
    component: &str,
    buttons: &[Value],
    target: Target,
    unsupported: &mut Vec<String>,
    allow_payment_request: bool,
) -> Vec<Slot> {
    let mut slots = Vec::new();
    for (button_index, button) in buttons.iter().enumerate() {
        let key = format!("{prefix}.{button_index}");
        let field_slot =
            |kind: &str, required: bool, placeholder: Option<String>, field: &'static str| Slot {
                spec: spec(key.clone(), component, kind, required, placeholder),
                apply: Apply::ButtonField {
                    target: target.clone(),
                    button_index,
                    field,
                },
            };
        match crate::js::to_upper(str_field(button, "type")).as_str() {
            "URL" => {
                if button["url"]
                    .as_str()
                    .is_some_and(|url| url.contains("{{1}}"))
                {
                    slots.push(field_slot(
                        "url_suffix",
                        true,
                        Some("{{1}}".to_string()),
                        "text",
                    ));
                }
            }
            "COPY_CODE" => slots.push(field_slot("coupon_code", true, None, "coupon_code")),
            "CATALOG" => slots.push(field_slot(
                "catalog_product",
                false,
                None,
                "thumbnail_product_retailer_id",
            )),
            "MPM" => unsupported.push(key.clone()),
            "PAYMENT_REQUEST" => {
                if allow_payment_request {
                    slots.extend(payment_request_slots(
                        &key,
                        component,
                        button,
                        &target,
                        button_index,
                        unsupported,
                    ));
                } else {
                    unsupported.push(key.clone());
                }
            }
            // FLOW and static buttons need no value: the skeleton carries what the send needs.
            _ => {}
        }
    }
    slots
}

fn lower_format(component: &Value) -> String {
    component["format"]
        .as_str()
        .map(crate::js::to_lower)
        .unwrap_or_default()
}

fn wa_header_slots(component: &Value) -> Vec<Slot> {
    let format = lower_format(component);
    match format.as_str() {
        "text" => text_slots(
            "header",
            "header",
            component["text"].as_str(),
            Target::Top("header"),
        ),
        "image" | "video" | "document" => vec![Slot {
            spec: spec("header".to_string(), "header", &format, true, None),
            apply: Apply::Media {
                target: Target::Top("header"),
                format: format.clone(),
            },
        }],
        "location" => [
            ("latitude", "latitude", true),
            ("longitude", "longitude", true),
            ("name", "location_name", false),
            ("address", "location_address", false),
        ]
        .into_iter()
        .map(|(field, kind, required)| Slot {
            spec: spec(format!("header.{field}"), "header", kind, required, None),
            apply: Apply::Location { field },
        })
        .collect(),
        _ => Vec::new(),
    }
}

fn wa_carousel_slots(component: &Value, unsupported: &mut Vec<String>) -> Vec<Slot> {
    let mut slots = Vec::new();
    for card in arr_field(component, "cards") {
        let card_index = card.get("card_index").cloned().unwrap_or(Value::Null);
        let prefix = format!("card.{}", crate::js::string_of(&card_index));
        let target = |key: &'static str| Target::Card {
            card_index: card_index.clone(),
            key,
        };
        for card_component in arr_field(card, "components") {
            let kind = crate::js::to_upper(str_field(card_component, "type"));
            let format = lower_format(card_component);
            if kind == "HEADER" && (format == "image" || format == "video") {
                slots.push(Slot {
                    spec: spec(format!("{prefix}.header"), "carousel", &format, true, None),
                    apply: Apply::Media {
                        target: target("header"),
                        format: format.clone(),
                    },
                });
            } else if kind == "BODY" {
                slots.extend(text_slots(
                    &format!("{prefix}.body"),
                    "carousel",
                    card_component["text"].as_str(),
                    target("body"),
                ));
            } else if kind == "BUTTONS"
                && let Some(buttons) = card_component["buttons"].as_array()
            {
                slots.extend(wa_button_slots(
                    &format!("{prefix}.button"),
                    "carousel",
                    buttons,
                    target("button"),
                    unsupported,
                    false,
                ));
            }
        }
    }
    slots
}

fn collect_wa_slots(components: &[Value]) -> (Vec<Slot>, Vec<String>) {
    let mut slots = Vec::new();
    let mut unsupported = Vec::new();
    for component in components {
        match crate::js::to_upper(str_field(component, "type")).as_str() {
            "HEADER" => slots.extend(wa_header_slots(component)),
            "BODY" => slots.extend(text_slots(
                "body",
                "body",
                component["text"].as_str(),
                Target::Top("body"),
            )),
            "BUTTONS" => slots.extend(wa_button_slots(
                "button",
                "button",
                arr_field(component, "buttons"),
                Target::Top("button"),
                &mut unsupported,
                true,
            )),
            "CAROUSEL" => slots.extend(wa_carousel_slots(component, &mut unsupported)),
            "LIMITED_TIME_OFFER"
                if truthy(component["limited_time_offer"].get("has_expiration")) =>
            {
                slots.push(Slot {
                    spec: spec(
                        "offer.expiration_ms".to_string(),
                        "limited_time_offer",
                        "expiration_ms",
                        true,
                        None,
                    ),
                    apply: Apply::Offer,
                });
            }
            _ => {}
        }
    }
    (slots, unsupported)
}

fn collect_messenger_slots(components: &[Value]) -> Vec<Slot> {
    let mut slots = Vec::new();
    for component in components {
        match crate::js::to_upper(str_field(component, "type")).as_str() {
            "HEADER" => slots.extend(text_slots(
                "header",
                "header",
                component["text"].as_str(),
                Target::Top("header"),
            )),
            "BODY" => slots.extend(text_slots(
                "body",
                "body",
                component["text"].as_str(),
                Target::Top("body"),
            )),
            "BUTTONS" => {
                for (button_index, button) in arr_field(component, "buttons").iter().enumerate() {
                    let url = button["url"].as_str();
                    let Some(suffix) = placeholder_tokens(url).into_iter().next() else {
                        continue;
                    };
                    if crate::js::to_upper(str_field(button, "type")) != "URL" {
                        continue;
                    }
                    slots.push(Slot {
                        // A Mini App or Minigame link defaults to the recipient's token variable.
                        spec: spec(
                            format!("button.{button_index}"),
                            "button",
                            "url_suffix",
                            suggest_messenger_url_suffix_variable(url).is_empty(),
                            Some(format!("{{{{{suffix}}}}}")),
                        ),
                        apply: Apply::ButtonField {
                            target: Target::Top("button"),
                            button_index,
                            field: "text",
                        },
                    });
                }
            }
            _ => {}
        }
    }
    slots
}

fn apply_slots(
    mut params: Value,
    slots: Vec<Slot>,
    unsupported: Vec<String>,
    values: &TemplateParameterValues,
) -> AppliedTemplateParameters {
    let mut missing = Vec::new();
    let mut invalid = Vec::new();
    for slot in &slots {
        // Blank only counts as missing; the value itself is sent as given.
        let value = values.get(&slot.spec.key).map(String::as_str).unwrap_or("");
        if crate::js::trim(value).is_empty() {
            if slot.spec.required {
                missing.push(slot.spec.key.clone());
            }
            continue;
        }
        if !apply_slot(&slot.apply, &mut params, value) {
            invalid.push(slot.spec.key.clone());
        }
    }
    let unknown = values
        .keys()
        .filter(|key| !slots.iter().any(|slot| &slot.spec.key == *key))
        .cloned()
        .collect();
    AppliedTemplateParameters {
        params,
        missing,
        unknown,
        invalid,
        unsupported,
    }
}

/// `describeWaTemplateParameters(components)`: the keys an API caller fills.
pub fn describe_wa_template_parameters(components: &[Value]) -> Vec<TemplateParameterSpec> {
    collect_wa_slots(components)
        .0
        .into_iter()
        .map(|slot| slot.spec)
        .collect()
}

/// `applyWaTemplateParameterValues(components, values)`: WhatsApp send params from flat pairs.
pub fn apply_wa_template_parameter_values(
    components: &[Value],
    values: &TemplateParameterValues,
) -> AppliedTemplateParameters {
    let (slots, unsupported) = collect_wa_slots(components);
    apply_slots(
        extract_template_params(components),
        slots,
        unsupported,
        values,
    )
}

/// `describeMessengerTemplateParameters(components)`.
pub fn describe_messenger_template_parameters(components: &[Value]) -> Vec<TemplateParameterSpec> {
    collect_messenger_slots(components)
        .into_iter()
        .map(|slot| slot.spec)
        .collect()
}

/// `applyMessengerTemplateParameterValues(components, parameterFormat, values)`.
pub fn apply_messenger_template_parameter_values(
    components: &[Value],
    parameter_format: &str,
    values: &TemplateParameterValues,
) -> AppliedTemplateParameters {
    apply_slots(
        extract_messenger_template_params(components, parameter_format),
        collect_messenger_slots(components),
        Vec::new(),
        values,
    )
}
