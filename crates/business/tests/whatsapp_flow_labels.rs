//! Q.P7-V4: ports `integrations/whatsapp/__tests__/flow-response-fields.test.ts`
//! (`parseFlowResponseFields`, the file kinds of `parseFlowScreens`) and
//! `apps/builder/__tests__/mini-app-flow-response-labels.test.ts` (the Mini App
//! Flow JSON → labels → `buildWhatsappFlowResponseFields`).
//!
//! Adapted: serde_json maps are sorted (no `preserve_order`), so the keys of
//! one payload come in key order, not the Flow JSON's order. `createNode` is
//! not ported: the Mini App inputs are written with its default props.

use chatbotx_business::whatsapp::build_flow_response_fields;
use chatbotx_business::whatsapp_flow::{
    FlowResponseLabel, parse_flow_response_fields, parse_flow_screens,
};
use chatbotx_core::mini_app::{parse_definition, to_flow_json};
use serde_json::{Map, Value, json};

fn footer(payload: Value) -> Value {
    json!({ "type": "Footer", "label": "Done",
        "on-click-action": { "name": "complete", "payload": payload } })
}

fn navigate(label: &str, next: &str) -> Value {
    json!({ "type": "Footer", "label": label, "on-click-action": { "name": "navigate",
        "next": { "type": "screen", "name": next }, "payload": {} } })
}

fn flow_json(screens: Vec<Value>) -> Value {
    json!({ "version": "7.3", "screens": screens })
}

fn screen(id: &str, children: Vec<Value>) -> Value {
    json!({ "id": id, "title": id, "terminal": true,
        "layout": { "type": "SingleColumnLayout", "children": children } })
}

fn field(key: &str, label: &str, options: Value, labelled: bool) -> FlowResponseLabel {
    FlowResponseLabel {
        key: key.into(),
        label: label.into(),
        options: options.as_object().cloned().unwrap_or_default(),
        labelled,
    }
}

fn keys_and_labels(fields: &[FlowResponseLabel]) -> Vec<(&str, &str)> {
    fields
        .iter()
        .map(|f| (f.key.as_str(), f.label.as_str()))
        .collect()
}

// ---- parseFlowResponseFields ----------------------------------------------

#[test]
fn labels_a_key_that_matches_the_inputs_name() {
    let fields = parse_flow_response_fields(&flow_json(vec![screen(
        "WELCOME",
        vec![
            json!({ "type": "TextInput", "name": "text_input", "label": "Full name" }),
            footer(json!({ "text_input": "${form.text_input}" })),
        ],
    )]));
    assert_eq!(
        fields,
        vec![field("text_input", "Full name", json!({}), true)]
    );
}

#[test]
fn traces_a_renamed_key_through_its_form_reference() {
    let fields = parse_flow_response_fields(&flow_json(vec![screen(
        "WELCOME",
        vec![
            json!({ "type": "TextInput", "name": "name_input", "label": "Họ tên" }),
            footer(json!({ "ho_ten": "${form.name_input}" })),
        ],
    )]));
    assert_eq!(fields, vec![field("ho_ten", "Họ tên", json!({}), true)]);
}

#[test]
fn traces_a_screen_form_reference_to_an_earlier_screen() {
    let fields = parse_flow_response_fields(&flow_json(vec![
        screen(
            "FIRST",
            vec![json!({ "type": "Form", "name": "form", "children": [
                { "type": "TextInput", "name": "phone", "label": "Phone number" },
            ] })],
        ),
        screen(
            "LAST",
            vec![footer(
                json!({ "customer_phone": "${screen.FIRST.form.phone}" }),
            )],
        ),
    ]));
    assert_eq!(
        fields,
        vec![field("customer_phone", "Phone number", json!({}), true)]
    );
}

#[test]
fn labels_each_answer_from_the_screen_it_names_when_names_repeat() {
    let fields = parse_flow_response_fields(&flow_json(vec![
        screen(
            "FIRST",
            vec![
                json!({ "type": "TextInput", "name": "note", "label": "Note about you" }),
                navigate("Next", "LAST"),
            ],
        ),
        screen(
            "LAST",
            vec![
                json!({ "type": "TextInput", "name": "note", "label": "Note for delivery" }),
                footer(json!({ "about": "${screen.FIRST.form.note}",
                    "delivery": "${form.note}", "LAST": "${data.unused}" })),
            ],
        ),
    ]));
    let mut got = keys_and_labels(&fields);
    got.sort_unstable();
    assert_eq!(
        got,
        vec![
            // A navigate target named like a key is not an input.
            ("LAST", "LAST"),
            ("about", "Note about you"),
            ("delivery", "Note for delivery"),
        ]
    );
}

#[test]
fn does_not_borrow_another_screens_label_for_a_dynamic_one() {
    let fields = parse_flow_response_fields(&flow_json(vec![
        screen(
            "FIRST",
            vec![json!({ "type": "TextInput", "name": "city", "label": "Billing city" })],
        ),
        screen(
            "LAST",
            vec![
                json!({ "type": "TextInput", "name": "city", "label": "${data.city_label}" }),
                footer(json!({ "shipping_city": "${form.city}" })),
            ],
        ),
    ]));
    assert_eq!(
        (fields[0].label.as_str(), fields[0].labelled),
        ("Shipping city", false)
    );
}

#[test]
fn reads_static_option_titles_and_skips_dynamic_ones() {
    let fields = parse_flow_response_fields(&flow_json(vec![screen(
        "WELCOME",
        vec![
            json!({ "type": "Dropdown", "name": "plan", "label": "Plan",
                "data-source": [{ "id": "0_basic", "title": "Basic" }, { "id": "1_pro", "title": "Pro" }] }),
            json!({ "type": "RadioButtonsGroup", "name": "slot", "label": "Slot",
                "data-source": "${data.slots}" }),
            footer(json!({ "plan": "${form.plan}", "slot": "${form.slot}" })),
        ],
    )]));
    assert_eq!(
        fields,
        vec![
            field(
                "plan",
                "Plan",
                json!({ "0_basic": "Basic", "1_pro": "Pro" }),
                true
            ),
            field("slot", "Slot", json!({}), true),
        ]
    );
}

#[test]
fn humanizes_keys_with_no_component_or_a_dynamic_label() {
    let fields = parse_flow_response_fields(&flow_json(vec![screen(
        "WELCOME",
        vec![
            json!({ "type": "TextInput", "name": "city", "label": "${data.city_label}" }),
            footer(
                json!({ "city": "${form.city}", "source_channel": "${data.channel}",
                "campaign": "spring" }),
            ),
        ],
    )]));
    let got: Vec<_> = fields
        .iter()
        .map(|f| (f.key.as_str(), f.label.as_str(), f.labelled))
        .collect();
    assert_eq!(
        got,
        vec![
            ("campaign", "Campaign", false),
            ("city", "City", false),
            ("source_channel", "Source channel", false),
        ]
    );
}

#[test]
fn returns_no_fields_for_an_endpoint_flow_or_bad_json() {
    let endpoint = flow_json(vec![screen(
        "WELCOME",
        vec![
            json!({ "type": "TextInput", "name": "email", "label": "Email" }),
            json!({ "type": "Footer", "label": "Next",
                "on-click-action": { "name": "data_exchange", "payload": {} } }),
        ],
    )]);
    for json in [
        endpoint,
        Value::Null,
        json!({ "version": "7.3" }),
        json!("not json"),
    ] {
        assert!(parse_flow_response_fields(&json).is_empty(), "{json}");
    }
}

// ---- file answers (parseFlowScreens) --------------------------------------

fn kinds_of(flow: &Value) -> Vec<(String, String)> {
    parse_flow_screens(flow)[0]["output"]
        .as_array()
        .unwrap()
        .iter()
        .map(|o| {
            (
                o["value"].as_str().unwrap().into(),
                o["kind"].as_str().unwrap().into(),
            )
        })
        .collect()
}

fn pairs(items: &[(&str, &str)]) -> Vec<(String, String)> {
    items
        .iter()
        .map(|(a, b)| ((*a).into(), (*b).into()))
        .collect()
}

fn photo_flow() -> Value {
    flow_json(vec![screen(
        "UPLOAD",
        vec![
            json!({ "type": "PhotoPicker", "name": "photo_picker", "label": "Photos" }),
            json!({ "type": "TextInput", "name": "name_input", "label": "Name" }),
            footer(
                json!({ "photo_picker": "${form.photo_picker}", "full_name": "${form.name_input}" }),
            ),
        ],
    )])
}

#[test]
fn tags_the_key_of_a_photo_or_document_picker_as_a_file() {
    let flow = flow_json(vec![screen(
        "UPLOAD",
        vec![
            json!({ "type": "DocumentPicker", "name": "doc_input", "label": "ID card" }),
            footer(json!({ "id_card": "${form.doc_input}" })),
        ],
    )]);
    assert_eq!(kinds_of(&flow), pairs(&[("id_card", "file")]));
}

#[test]
fn tags_every_other_key_as_a_value() {
    assert_eq!(
        kinds_of(&photo_flow()),
        pairs(&[("full_name", "value"), ("photo_picker", "file")])
    );
}

#[test]
fn follows_a_reference_to_a_picker_on_an_earlier_screen() {
    let flow = flow_json(vec![
        screen(
            "UPLOAD",
            vec![
                json!({ "type": "PhotoPicker", "name": "photos", "label": "Photos" }),
                navigate("Next", "DONE"),
            ],
        ),
        screen(
            "DONE",
            vec![footer(json!({ "receipt": "${screen.UPLOAD.form.photos}" }))],
        ),
    ]);
    assert_eq!(kinds_of(&flow), pairs(&[("receipt", "file")]));
}

#[test]
fn marks_a_literal_or_data_payload_value_as_a_value() {
    let flow = flow_json(vec![screen(
        "UPLOAD",
        vec![
            json!({ "type": "PhotoPicker", "name": "photos", "label": "Photos" }),
            footer(json!({ "source": "web", "from_data": "${data.photos}" })),
        ],
    )]);
    assert_eq!(
        kinds_of(&flow),
        pairs(&[("from_data", "value"), ("source", "value")])
    );
}

#[test]
fn exposes_the_kind_on_the_outputs_of_each_screen() {
    assert_eq!(
        parse_flow_screens(&photo_flow())[0]["output"],
        json!([
            { "value": "full_name", "label": "Full name", "kind": "value" },
            { "value": "photo_picker", "label": "Photos", "kind": "file" },
        ])
    );
}

#[test]
fn does_not_call_a_literal_answer_a_file_because_an_input_of_that_name_is_a_picker() {
    let flow = flow_json(vec![screen(
        "UPLOAD",
        vec![
            json!({ "type": "PhotoPicker", "name": "photos", "label": "Photos" }),
            footer(json!({ "photos": "none" })),
        ],
    )]);
    assert_eq!(
        parse_flow_screens(&flow)[0]["output"],
        json!([{ "value": "photos", "label": "Photos", "kind": "value" }])
    );
}

#[test]
fn keeps_a_key_that_two_screens_fill_differently_apart_per_screen() {
    let flow = flow_json(vec![
        screen(
            "TEXT_PATH",
            vec![
                json!({ "type": "TextInput", "name": "note", "label": "Note" }),
                footer(json!({ "answer": "${form.note}" })),
            ],
        ),
        screen(
            "DOCUMENT_PATH",
            vec![
                json!({ "type": "DocumentPicker", "name": "doc", "label": "Document" }),
                footer(json!({ "answer": "${form.doc}" })),
            ],
        ),
    ]);
    let screens = parse_flow_screens(&flow);
    let kind = |id: &str| {
        let s = screens.iter().find(|s| s["id"] == id).unwrap();
        s["output"][0]["kind"].clone()
    };
    assert_eq!(kind("TEXT_PATH"), "value");
    assert_eq!(kind("DOCUMENT_PATH"), "file");
}

#[test]
fn a_key_two_branches_from_one_start_fill_differently_is_a_file_answer() {
    let flow = flow_json(vec![
        json!({ "id": "START", "title": "START", "layout": { "type": "SingleColumnLayout",
            "children": [navigate("DOCS", "DOCS"), navigate("TEXT", "TEXT")] } }),
        screen(
            "DOCS",
            vec![
                json!({ "type": "DocumentPicker", "name": "doc", "label": "Document" }),
                footer(json!({ "answer": "${form.doc}" })),
            ],
        ),
        screen(
            "TEXT",
            vec![
                json!({ "type": "TextInput", "name": "note", "label": "Note" }),
                footer(json!({ "answer": "${form.note}" })),
            ],
        ),
    ]);
    assert_eq!(kinds_of(&flow), pairs(&[("answer", "file")]));
}

#[test]
fn a_key_the_branches_of_one_screen_fill_differently_is_a_file_answer() {
    let flow = flow_json(vec![screen(
        "UPLOAD",
        vec![
            json!({ "type": "TextInput", "name": "note", "label": "Note" }),
            json!({ "type": "DocumentPicker", "name": "doc", "label": "Document" }),
            json!({ "type": "If", "condition": "${form.note} != ''",
                "then": [footer(json!({ "answer": "${form.note}" }))],
                "else": [footer(json!({ "answer": "${form.doc}" }))] }),
        ],
    )]);
    assert_eq!(kinds_of(&flow), pairs(&[("answer", "file")]));
}

// ---- Mini App Flow JSON → response labels ---------------------------------

fn options(count: usize) -> Value {
    (1..=count)
        .map(|i| json!({ "id": format!("option_{i}"), "title": format!("Option {i}") }))
        .collect()
}

/// The starter definition plus one of each input `createNode` makes (its
/// default props and first unique name).
fn mini_app_flow_json() -> Value {
    let node =
        |id: &str, kind: &str, props: Value| json!({ "id": id, "type": kind, "props": props });
    let definition = json!({ "screens": [{ "key": "s_1", "id": "WELCOME", "title": "New screen", "children": [
        node("n_1", "TextHeading", json!({ "text": "Heading" })),
        node("n_2", "TextInput", json!({ "label": "Your answer", "input-type": "text", "required": false, "name": "text_input_1" })),
        node("n_3", "Dropdown", json!({ "label": "Choose one", "data-source": options(3), "required": false, "name": "dropdown_1" })),
        node("n_4", "CheckboxGroup", json!({ "label": "Choose any", "data-source": options(2), "required": false, "name": "checkbox_group_1" })),
        node("n_5", "OptIn", json!({ "label": "I agree to the terms", "required": false, "name": "opt_in_1" })),
        node("n_6", "DatePicker", json!({ "label": "Date", "required": false, "name": "date_picker_1" })),
        node("n_7", "Footer", json!({ "label": "Continue", "on-click-action": { "name": "complete" } })),
    ] }] });
    to_flow_json(&parse_definition(&definition).expect("definition"))
}

#[test]
fn every_mini_app_input_is_labelled_with_its_option_titles() {
    let labels = parse_flow_response_fields(&mini_app_flow_json());
    let got: Vec<_> = labels
        .iter()
        .map(|l| (l.key.as_str(), l.labelled))
        .collect();
    assert_eq!(
        got,
        vec![
            ("checkbox_group_1", true),
            ("date_picker_1", true),
            ("dropdown_1", true),
            ("opt_in_1", true),
            ("text_input_1", true),
        ]
    );
    let dropdown = labels.iter().find(|l| l.key == "dropdown_1").unwrap();
    assert_eq!(dropdown.options["option_1"], "Option 1");
}

#[test]
fn a_whatsapp_answer_to_that_mini_app_reads_like_the_form() {
    let labels = parse_flow_response_fields(&mini_app_flow_json());
    let response: Map<String, Value> = json!({
        "flow_token": "tok",
        "text_input_1": "An",
        "dropdown_1": "option_2",
        "checkbox_group_1": ["option_1", "option_2"],
        "opt_in_1": true,
        "date_picker_1": "2026-10-11",
    })
    .as_object()
    .cloned()
    .unwrap();
    let view = build_flow_response_fields(&response, &labels, None, &|_| None);
    assert_eq!(view["labelSource"], "whatsappFlow");
    let got: Vec<_> = view["fields"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| (f["kind"].clone(), f["value"].clone()))
        .collect();
    assert_eq!(
        got,
        vec![
            (json!("list"), json!(["Option 1", "Option 2"])),
            (json!("text"), json!("2026-10-11")),
            (json!("text"), json!("Option 2")),
            (json!("boolean"), json!(true)),
            (json!("text"), json!("An")),
        ]
    );
}

#[test]
fn a_response_with_no_flow_labels_reads_with_humanized_keys() {
    let response = json!({ "flow_token": "t", "full_name": "An" })
        .as_object()
        .cloned()
        .unwrap();
    let view = build_flow_response_fields(&response, &[], None, &|_| None);
    assert_eq!(
        view,
        json!({ "fields": [{ "key": "full_name", "label": "Full name", "kind": "text", "value": "An" }],
            "labelSource": "fallback" })
    );
}
