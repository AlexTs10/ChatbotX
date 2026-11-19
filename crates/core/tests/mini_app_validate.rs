//! Port of `packages/mini-app/__tests__/validate.test.ts`.

mod mini_app_fixtures;

use chatbotx_core::mini_app::validate;
use mini_app_fixtures::{children, two_screen_definition};
use serde_json::{Value, json};

fn codes(definition: &Value) -> Vec<&'static str> {
    validate(definition).issues.iter().map(|i| i.code).collect()
}

#[test]
fn validate_mini_app_accepts_a_valid_two_screen_app() {
    let result = validate(&two_screen_definition());
    assert!(result.valid);
    assert!(result.issues.is_empty(), "{:?}", result.issues);
}

#[test]
fn validate_mini_app_requires_a_screen_that_completes_the_flow() {
    let mut d = two_screen_definition();
    children(&mut d, 1).last_mut().unwrap()["props"]
        .as_object_mut()
        .unwrap()
        .remove("on-click-action");
    assert!(codes(&d).contains(&"no_terminal_screen"));
}

#[test]
fn validate_mini_app_enforces_per_screen_limits_and_footer_position() {
    let mut d = two_screen_definition();
    let welcome = children(&mut d, 0);
    welcome
        .push(json!({ "id": "n_late", "type": "TextBody", "props": { "text": "after footer" } }));
    for i in 0..3 {
        welcome.insert(0, json!({ "id": format!("n_link_{i}"), "type": "EmbeddedLink", "props": {
            "text": "Link", "on-click-action": { "name": "open_url", "url": "https://example.com" } } }));
    }
    let result = codes(&d);
    assert!(result.contains(&"footer_not_last"));
    assert!(result.contains(&"component_limit"));
}

#[test]
fn validate_mini_app_reports_property_limits_with_the_max_length() {
    let mut d = two_screen_definition();
    d["screens"][0]["children"][1]["props"]["label"] = json!("This label is far too long for Meta");
    let result = validate(&d);
    let issue = result
        .issues
        .iter()
        .find(|i| i.property.as_deref() == Some("label"))
        .unwrap();
    assert_eq!(issue.code, "property_too_long");
    assert_eq!(issue.params.as_ref().unwrap()["max"], json!(20));
    assert_eq!(issue.node_id.as_deref(), Some("n_name"));
}

#[test]
fn validate_mini_app_rejects_duplicate_names_across_screens_and_unknown_references() {
    let mut d = two_screen_definition();
    d["screens"][1]["children"][1]["props"]["name"] = json!("full_name");
    d["screens"][1]["children"][0]["props"]["text"] = json!("Hi ${form.nobody} ${data.x}");
    let result = codes(&d);
    assert!(result.contains(&"name_duplicate"));
    assert!(result.contains(&"reference_unknown"));
    assert!(result.contains(&"data_reference_unsupported"));
}

fn nested(depth: usize) -> Value {
    let then = if depth > 1 {
        nested(depth - 1)
    } else {
        json!({ "id": format!("n_leaf_{depth}"), "type": "TextBody", "props": { "text": "x" } })
    };
    json!({ "id": format!("n_if_{depth}"), "type": "If", "props": { "condition": "${form.full_name} == 'a'" },
        "slots": { "then": [then], "else": [] } })
}

#[test]
fn validate_mini_app_checks_if_nesting_allowed_children_and_footer_branches() {
    let mut d = two_screen_definition();
    children(&mut d, 0).insert(1, nested(4));
    children(&mut d, 0).insert(1, json!({ "id": "n_if_footer", "type": "If", "props": { "condition": "true" },
        "slots": { "then": [{ "id": "n_photo", "type": "PhotoPicker", "props": { "name": "photo", "label": "Photo" } }], "else": [] } }));
    let result = codes(&d);
    assert!(result.contains(&"if_too_deep"));
    assert!(result.contains(&"child_not_allowed"));
}

#[test]
fn validate_mini_app_flags_navigation_to_a_missing_screen_and_screens_without_an_exit() {
    let mut d = two_screen_definition();
    d["screens"][0]["children"][2]["props"]["on-click-action"] =
        json!({ "name": "navigate", "next": "s_missing" });
    let result = codes(&d);
    assert!(result.contains(&"navigate_target_missing"));
    assert!(result.contains(&"screen_unreachable"));
}

#[test]
fn validate_mini_app_accepts_several_files_while_the_response_stays_within_metas_limit() {
    let mut d = two_screen_definition();
    children(&mut d, 1).insert(
        0,
        json!({ "id": "n_docs", "type": "DocumentPicker",
        "props": { "name": "docs", "label": "Docs", "max-uploaded-documents": 3 } }),
    );
    assert!(!codes(&d).contains(&"picker_multiple_in_complete"));
}

#[test]
fn validate_mini_app_rejects_properties_meta_does_not_declare_for_the_component() {
    let mut d = two_screen_definition();
    d["screens"][0]["children"][1]["props"]["enabled"] = json!(false);
    d["screens"][0]["children"][1]["props"]["placeholder"] = json!("x");
    let result = validate(&d);
    let issue = result
        .issues
        .iter()
        .find(|i| i.code == "property_unknown")
        .unwrap();
    assert_eq!(
        issue.params.as_ref().unwrap()["name"],
        "enabled, placeholder"
    );
}

#[test]
fn validate_mini_app_rejects_required_on_file_pickers() {
    let mut d = two_screen_definition();
    children(&mut d, 0).insert(
        1,
        json!({ "id": "n_photo", "type": "PhotoPicker",
        "props": { "name": "photo", "label": "Photo", "required": true } }),
    );
    assert!(codes(&d).contains(&"property_unknown"));
}

#[test]
fn validate_mini_app_checks_footer_captions_chips_options_and_navigation_list_badges() {
    let mut d = two_screen_definition();
    children(&mut d, 1).last_mut().unwrap()["props"]["left-caption"] = json!("Left");
    children(&mut d, 0).insert(1, json!({ "id": "n_chips", "type": "ChipsSelector",
        "props": { "name": "chips", "label": "Chips", "data-source": [{ "id": "a", "title": "A" }] } }));
    let result = codes(&d);
    assert!(result.contains(&"footer_captions_invalid"));
    assert!(result.contains(&"property_too_short"));
}

// ---- files in the response message (Meta sends at most 10 files) --------------

/// `RouteSpec`: a screen with an optional PhotoPicker (`Some(None)` leaves the
/// maximum out), navigation targets and an optional complete footer.
struct Route {
    key: &'static str,
    files: Option<Option<u64>>,
    next: &'static [&'static str],
    complete: bool,
}

const fn route(
    key: &'static str,
    files: Option<Option<u64>>,
    next: &'static [&'static str],
    complete: bool,
) -> Route {
    Route {
        key,
        files,
        next,
        complete,
    }
}

fn routed(specs: &[Route]) -> Value {
    let screens: Vec<Value> = specs
        .iter()
        .map(|spec| {
            let mut children = Vec::new();
            if let Some(max) = spec.files {
                let mut props = json!({ "name": format!("photo_{}", spec.key), "label": "Photo" });
                if let Some(max) = max {
                    props["max-uploaded-photos"] = json!(max);
                }
                children.push(json!({ "id": format!("p_{}", spec.key), "type": "PhotoPicker", "props": props }));
            }
            for (index, target) in spec.next.iter().enumerate() {
                children.push(json!({ "id": format!("l_{}_{index}", spec.key), "type": "EmbeddedLink",
                    "props": { "text": "Go", "on-click-action": { "name": "navigate", "next": target } } }));
            }
            if spec.complete {
                children.push(json!({ "id": format!("d_{}", spec.key), "type": "Footer",
                    "props": { "label": "Done", "on-click-action": { "name": "complete" } } }));
            }
            json!({ "key": spec.key, "id": spec.key.to_uppercase(), "title": spec.key, "children": children })
        })
        .collect();
    json!({ "screens": screens })
}

fn is_too_many(specs: &[Route]) -> bool {
    codes(&routed(specs)).contains(&"picker_multiple_in_complete")
}

const fn files(n: u64) -> Option<Option<u64>> {
    Some(Some(n))
}

#[test]
fn files_a_single_picker_of_up_to_10_files_is_accepted_11_rejected() {
    assert!(!is_too_many(&[route("sa", files(10), &[], true)]));
    assert!(is_too_many(&[route("sa", files(11), &[], true)]));
}

#[test]
fn files_adds_the_pickers_of_screens_visited_one_after_the_other() {
    assert!(is_too_many(&[
        route("sa", files(6), &["sb"], false),
        route("sb", files(5), &[], true)
    ]));
    assert!(!is_too_many(&[
        route("sa", files(5), &["sb"], false),
        route("sb", files(5), &[], true)
    ]));
}

#[test]
fn files_adds_exclusive_branches_together_as_the_payload_references_both() {
    let branches = |sc: u64| {
        [
            route("sa", None, &["sb", "sc"], false),
            route("sb", files(6), &["sd"], false),
            route("sc", files(sc), &["sd"], false),
            route("sd", None, &[], true),
        ]
    };
    assert!(!is_too_many(&branches(4)));
    assert!(is_too_many(&branches(5)));
}

#[test]
fn files_counts_screens_that_can_loop_back_to_each_other_once() {
    let looped = |last: u64| {
        [
            route("sa", files(3), &["sb"], false),
            route("sb", files(3), &["sa", "sc"], false),
            route("sc", files(last), &[], true),
        ]
    };
    assert!(!is_too_many(&looped(4)));
    assert!(is_too_many(&looped(5)));
}

#[test]
fn files_treats_a_picker_without_a_maximum_as_the_web_limit() {
    assert!(is_too_many(&[route("sa", Some(None), &[], true)]));
}

#[test]
fn files_reports_the_completing_screen_and_metas_maximum() {
    let result = validate(&routed(&[
        route("sa", files(6), &["sb"], false),
        route("sb", files(6), &[], true),
    ]));
    let issue = result
        .issues
        .iter()
        .find(|i| i.code == "picker_multiple_in_complete")
        .expect("issue");
    assert_eq!(issue.screen_key.as_deref(), Some("sb"));
    assert_eq!(
        issue.params.as_ref().and_then(|p| p.get("max")),
        Some(&json!(10))
    );
}

#[test]
fn files_checks_a_completing_screen_the_first_screen_cannot_reach() {
    assert!(is_too_many(&[
        route("sa", None, &[], true),
        route("sb", files(11), &[], true)
    ]));
}

#[test]
fn files_counts_routes_that_start_on_a_later_screen() {
    assert!(is_too_many(&[
        route("sa", None, &[], true),
        route("sb", files(6), &["sc"], false),
        route("sc", files(5), &[], true),
    ]));
}

#[test]
fn files_accepts_a_detached_completing_screen_within_the_limit() {
    assert!(!is_too_many(&[
        route("sa", None, &[], true),
        route("sb", files(10), &[], true)
    ]));
}

#[test]
fn files_survives_navigation_to_a_screen_that_does_not_exist() {
    validate(&routed(&[route("sa", files(1), &["sx"], true)]));
}
