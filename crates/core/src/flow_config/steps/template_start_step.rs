//! Port of `packages/flow-config/src/steps/template-start-step.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

pub fn template_step_type_aliases() -> &'static Value {
    static V: LazyLock<Value> = LazyLock::new(|| {
        serde_json::from_str(
            r#"{"sendWaTemplateMessage":["WA_TM01"],"sendMessengerTemplateMessage":[]}"#,
        )
        .expect("valid JSON")
    });
    &V
}

/// `resolveStepTypeMatches(stepType)`: the type plus its legacy aliases.
pub fn resolve_step_type_matches(step_type: &str) -> Vec<String> {
    let mut out = vec![step_type.to_owned()];
    if let Some(aliases) = template_step_type_aliases()[step_type].as_array() {
        out.extend(aliases.iter().filter_map(|a| a.as_str().map(str::to_owned)));
    }
    out
}

/// `findTemplateStartStep(nodes, stepType)`: the template a start node sends.
pub fn find_template_start_step(nodes: &Value, step_type: &str) -> Option<(String, String)> {
    let nodes = nodes.as_array()?;
    let matches = resolve_step_type_matches(step_type);
    for node in nodes.iter().filter(|n| n["data"]["isStartNode"] == true) {
        for step in node["data"]["details"]["steps"]
            .as_array()
            .into_iter()
            .flatten()
        {
            let Some(candidate) = step["stepType"].as_str().filter(|s| !s.is_empty()) else {
                continue;
            };
            if !matches.iter().any(|m| m == candidate) {
                continue;
            }
            if let Some(template_id) = step["template"]["id"].as_str().filter(|s| !s.is_empty()) {
                return Some((candidate.to_owned(), template_id.to_owned()));
            }
        }
    }
    None
}
