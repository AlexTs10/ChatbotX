//! Port of `packages/flow-config/src/steps/send-messenger-message-template.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

schema!(pub fn messenger_template_button_param_schema = z::object(vec![("sub_type", z::enum_(&["url", "phone_number"]).describe("Button kind: `url` or `phone_number`.")),
("index", z::number().optional().describe("Zero-based position of the button in the template.")),
("text", z::string().optional().describe("Variable value for a `url` button (the dynamic URL suffix). For a Mini App or Minigame link, leave it blank to send each recipient's own token (`{{mini_app_token}}` / `{{minigame_play_token}}`); a fixed value would not identify the contact.")),
("payload", z::string().optional().describe("Legacy; ignored by the sender. POSTBACK payloads come from the step's `buttons` configuration, so configure buttons there."))]));

schema!(pub fn messenger_template_params_schema = z::object(vec![("header", z::array(z::object(vec![("type", z::enum_(&["text", "image"]).describe("Header parameter kind; fill the matching field.")),
("text", z::string().optional().describe("Value for a `text` header variable.")),
("parameter_name", z::string().optional().describe("Placeholder name, only for NAMED templates.")),
("image", z::object(vec![("link", z::string().describe("Public https URL of the image."))]).optional().describe("Image for an `image` header."))])).optional().describe("Header parameters; one entry for a templated header.")),
("body", z::array(z::object(vec![("text", z::string().describe("Value that replaces the variable.")),
("parameter_name", z::string().optional().describe("Placeholder name, only for NAMED templates."))])).optional().describe("Body variables in order: {{1}}, {{2}}, ...")),
("button", z::array(messenger_template_button_param_schema()).optional().describe("Parameters for the template's dynamic buttons, by `index`."))]));

schema!(pub fn send_messenger_template_message_step_schema = z::object(vec![("id", z::string().regex(r#"^\d+$"#, "").describe("Step id (numeric string), unique within the flow.")),
("nodeId", z::string().optional().describe("Id of the node containing this step. Optional.")),
("stepType", z::literal(json!("sendMessengerTemplateMessage")).describe("Step type discriminator: \"sendMessengerTemplateMessage\".")),
("template", z::object(vec![("id", z::string().trim().min(1)),
("name", z::string()),
("language", z::string()),
("parameterFormat", z::enum_(&["POSITIONAL", "NAMED"]).default(json!("POSITIONAL"))),
("params", messenger_template_params_schema()),
("inboxId", z::string().optional()),
("integrationMessengerId", z::string().optional())])),
("buttons", z::array(button_step_schema()).super_refine(refine_reply_only_buttons).default(json!([])))]));

/// `sendMessengerTemplateMessageStepDefaultFn(props)`.
pub fn send_messenger_template_message_step_default_fn(props: &Value) -> Value {
    template_step_default(
        props,
        json!({ "id": "", "name": "", "language": "", "parameterFormat": "POSITIONAL", "params": {} }),
        json!([]),
        "sendMessengerTemplateMessage",
    )
}

/// `{ template: { ...defaults, ...props.template }, buttons, ...rest, id, stepType }`.
pub(crate) fn template_step_default(
    props: &Value,
    template: Value,
    buttons: Value,
    step_type: &str,
) -> Value {
    let template = spread_props(template, props.get("template").unwrap_or(&Value::Null));
    let mut rest = props.clone();
    if let Some(obj) = rest.as_object_mut() {
        obj.remove("template");
    }
    spread3(
        json!({ "template": template, "buttons": buttons }),
        &rest,
        json!({ "id": create_id(), "stepType": step_type }),
    )
}

fn re(pattern: &str) -> regex::Regex {
    regex::Regex::new(pattern).expect("valid regex")
}

fn placeholder_re() -> &'static regex::Regex {
    static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    RE.get_or_init(|| re(r"\{\{([0-9]+|[a-zA-Z_]+)\}\}"))
}

fn extract_placeholder_names(text: Option<&str>) -> Vec<String> {
    text.map(|t| {
        placeholder_re()
            .captures_iter(t)
            .map(|c| c[1].to_owned())
            .collect()
    })
    .unwrap_or_default()
}

fn has_placeholder(text: Option<&str>) -> bool {
    !extract_placeholder_names(text).is_empty()
}

const TOKEN_URL_PAGES: &[(&str, &str)] = &[
    ("mini-apps", "mini_app_token"),
    ("minigames", "minigame_play_token"),
];

fn token_url_pattern(page: &str, placeholder: &str) -> regex::Regex {
    re(&format!(
        r"/{page}\?(?:[^#]*&)?token=\{{\{{{placeholder}\}}\}}$"
    ))
}

struct TokenUrlSuffix {
    variable: &'static str,
    pasted: regex::Regex,
    saved: regex::Regex,
}

fn token_url_suffixes() -> &'static [TokenUrlSuffix] {
    static V: std::sync::OnceLock<Vec<TokenUrlSuffix>> = std::sync::OnceLock::new();
    V.get_or_init(|| {
        TOKEN_URL_PAGES
            .iter()
            .map(|(page, variable)| TokenUrlSuffix {
                variable,
                pasted: token_url_pattern(page, &format!(r"\s*{variable}\s*")),
                saved: token_url_pattern(page, r"[A-Za-z0-9_]+"),
            })
            .collect()
    })
}

/// `findMessengerTokenVariable`: the token variable a text embeds.
pub fn find_messenger_token_variable(text: Option<&str>) -> Option<&'static str> {
    static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let pattern = RE.get_or_init(|| {
        let names: Vec<&str> = TOKEN_URL_PAGES.iter().map(|(_, v)| *v).collect();
        re(&format!(r"\{{\{{\s*({})\s*\}}\}}", names.join("|")))
    });
    let name = pattern.captures(text?)?.get(1)?.as_str().to_owned();
    TOKEN_URL_PAGES.iter().map(|(_, v)| *v).find(|v| *v == name)
}

/// `normalizeMessengerTokenUrl`: a pasted token URL becomes `…{{1}}`.
pub fn normalize_messenger_token_url(url: &str) -> Option<(String, &'static str)> {
    static TRAILING: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let entry = token_url_suffixes()
        .iter()
        .find(|e| e.pasted.is_match(url))?;
    let trailing = TRAILING.get_or_init(|| re(r"\{\{[^{}]*\}\}$"));
    Some((trailing.replace(url, "{{1}}").into_owned(), entry.variable))
}

fn mini_app_url_pattern() -> &'static regex::Regex {
    static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    RE.get_or_init(|| token_url_pattern("mini-apps", r"[A-Za-z0-9_]+"))
}

fn mini_app_id(url: &str) -> Option<String> {
    static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let pattern = RE.get_or_init(|| re(r"[?&]id=([0-9]+)(?:&|$)"));
    pattern.captures(url).map(|c| c[1].to_owned())
}

fn component_buttons(component: &Value) -> Option<&Vec<Value>> {
    if component["type"] != "BUTTONS" {
        return None;
    }
    component["buttons"].as_array()
}

/// `readMessengerMiniAppButtons`: URL buttons that open a Mini App.
pub fn read_messenger_mini_app_buttons(components: &[Value]) -> Vec<Value> {
    let mut out = Vec::new();
    for component in components {
        let Some(buttons) = component_buttons(component) else {
            continue;
        };
        for (index, button) in buttons.iter().enumerate() {
            let url = str_field(button, "url");
            let id = mini_app_id(url);
            if crate::js::to_upper(str_field(button, "type")) == "URL"
                && id.is_some()
                && mini_app_url_pattern().is_match(url)
            {
                out.push(
                    json!({ "index": index, "label": str_field(button, "text"), "miniAppId": id }),
                );
            }
        }
    }
    out
}

/// `suggestMessengerUrlSuffixVariable`.
pub fn suggest_messenger_url_suffix_variable(url: Option<&str>) -> String {
    token_url_suffixes()
        .iter()
        .find(|e| e.saved.is_match(url.unwrap_or("")))
        .map(|e| format!("{{{{{}}}}}", e.variable))
        .unwrap_or_default()
}

fn with_parameter_name(mut param: Value, name: &str, parameter_format: &str) -> Value {
    if parameter_format == "NAMED" {
        param["parameter_name"] = json!(name);
    }
    param
}

/// `extractMessengerTemplateParams(components, parameterFormat)`.
pub fn extract_messenger_template_params(components: &[Value], parameter_format: &str) -> Value {
    let mut params = Map::new();
    for component in components {
        match component["type"].as_str() {
            Some("HEADER") => {
                let names = extract_placeholder_names(component["text"].as_str());
                if !names.is_empty() {
                    let header: Vec<Value> = names
                        .iter()
                        .map(|n| {
                            with_parameter_name(
                                json!({ "type": "text", "text": "" }),
                                n,
                                parameter_format,
                            )
                        })
                        .collect();
                    params.insert("header".into(), Value::Array(header));
                }
            }
            Some("BODY") => {
                let names = extract_placeholder_names(component["text"].as_str());
                if !names.is_empty() {
                    let body: Vec<Value> = names
                        .iter()
                        .map(|n| with_parameter_name(json!({ "text": "" }), n, parameter_format))
                        .collect();
                    params.insert("body".into(), Value::Array(body));
                }
            }
            Some("BUTTONS") => {
                let Some(buttons) = component["buttons"].as_array() else {
                    continue;
                };
                let mut button_params = Vec::new();
                for (idx, button) in buttons.iter().enumerate() {
                    let button_type = crate::js::to_upper(str_field(button, "type"));
                    let url = button["url"].as_str().filter(|u| !u.is_empty());
                    if button_type == "URL"
                        && let Some(url) = url
                    {
                        let text = if has_placeholder(Some(url)) {
                            suggest_messenger_url_suffix_variable(Some(url))
                        } else {
                            url.to_owned()
                        };
                        button_params
                            .push(json!({ "sub_type": "url", "index": idx, "text": text }));
                    }
                    if button_type == "PHONE_NUMBER" {
                        button_params.push(json!({ "sub_type": "phone_number", "index": idx }));
                    }
                }
                if !button_params.is_empty() {
                    params.insert("button".into(), Value::Array(button_params));
                }
            }
            _ => {}
        }
    }
    Value::Object(params)
}

/// `extractMessengerParameterInfos(components, parameterFormat)`.
pub fn extract_messenger_parameter_infos(
    components: &[Value],
    parameter_format: &str,
) -> Vec<Value> {
    let display = |name: &str, idx: usize| {
        if parameter_format == "NAMED" {
            name.to_owned()
        } else {
            (idx + 1).to_string()
        }
    };
    let mut out = Vec::new();
    for component in components {
        match component["type"].as_str() {
            Some("HEADER") => {
                for (idx, name) in extract_placeholder_names(component["text"].as_str())
                    .iter()
                    .enumerate()
                {
                    out.push(json!({ "type": "header", "index": idx, "paramName": display(name, idx), "format": "text" }));
                }
            }
            Some("BODY") => {
                for (idx, name) in extract_placeholder_names(component["text"].as_str())
                    .iter()
                    .enumerate()
                {
                    out.push(
                        json!({ "type": "body", "index": idx, "paramName": display(name, idx) }),
                    );
                }
            }
            Some("BUTTONS") => {
                let Some(buttons) = component["buttons"].as_array() else {
                    continue;
                };
                for (button_idx, button) in buttons.iter().enumerate() {
                    let button_type = crate::js::to_upper(str_field(button, "type"));
                    let names = extract_placeholder_names(button["url"].as_str());
                    if button_type == "URL"
                        && let Some(suffix) = names.first()
                    {
                        out.push(json!({
                            "type": "button",
                            "index": 0,
                            "paramName": display(suffix, 0),
                            "buttonIndex": button_idx,
                            "buttonSubType": "url",
                        }));
                    }
                }
            }
            _ => {}
        }
    }
    out
}

/// `extractMessengerFlowButtons(components, { includeMiniApp })`.
pub fn extract_messenger_flow_buttons(components: &[Value], include_mini_app: bool) -> Vec<Value> {
    let mini_apps: std::collections::HashMap<u64, Value> = if include_mini_app {
        read_messenger_mini_app_buttons(components)
            .into_iter()
            .map(|b| (b["index"].as_u64().unwrap_or(0), b))
            .collect()
    } else {
        std::collections::HashMap::new()
    };
    let mut out = Vec::new();
    for component in components {
        let Some(buttons) = component_buttons(component) else {
            continue;
        };
        for (index, button) in buttons.iter().enumerate() {
            let button_type = crate::js::to_upper(str_field(button, "type"));
            if let Some(mini_app) = mini_apps.get(&(index as u64)) {
                let mut b = button_step_default_fn(&json!({ "label": mini_app["label"] }));
                b["buttonType"] = json!("openMiniApp");
                b["beforeStep"] =
                    open_mini_app_step_default_fn(&json!({ "miniAppId": mini_app["miniAppId"] }));
                out.push(b);
            } else if (button_type == "POSTBACK" || button_type == "QUICK_REPLY")
                && str_field(button, "payload").contains("{{")
            {
                out.push(button_step_default_fn(
                    &json!({ "label": str_field(button, "text") }),
                ));
            }
        }
    }
    out
}

fn is_open_mini_app(button: Option<&Value>) -> bool {
    button.is_some_and(|b| b["buttonType"] == "openMiniApp")
}

/// `mergeMessengerFlowButtonsWithExisting`: like the template merge, but a
/// slot that switched between reply and Mini App takes the template button,
/// and a Mini App button follows the template's Mini App.
pub fn merge_messenger_flow_buttons_with_existing(
    template_buttons: &[Value],
    existing: &[Value],
) -> Vec<Value> {
    let merged = merge_template_buttons_with_existing(template_buttons, existing);
    merged
        .into_iter()
        .enumerate()
        .map(|(index, button)| {
            let Some(template_button) = template_buttons.get(index) else {
                return button;
            };
            if existing.get(index).is_none() {
                return button;
            }
            if is_open_mini_app(Some(&button)) != is_open_mini_app(Some(template_button)) {
                return template_button.clone();
            }
            if button["buttonType"] == "openMiniApp"
                && template_button["buttonType"] == "openMiniApp"
                && button["beforeStep"]["miniAppId"] != template_button["beforeStep"]["miniAppId"]
            {
                let mut out = button.clone();
                out["beforeStep"]["miniAppId"] = template_button["beforeStep"]["miniAppId"].clone();
                return out;
            }
            button
        })
        .collect()
}
