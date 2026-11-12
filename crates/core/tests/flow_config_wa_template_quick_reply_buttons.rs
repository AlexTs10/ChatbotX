//! Port of `packages/flow-config/__tests__/wa-template-quick-reply-buttons.test.ts`.

mod flow_config_support;
use flow_config_support::*;

fn buttons_component(buttons: Value) -> Value {
    json!({ "type": "BUTTONS", "buttons": buttons })
}

// The failing production shape: a static (non-parameterized) button BEFORE a
// parameterized one, so dense param positions diverge from template indexes.
fn static_url_then_quick_reply() -> Vec<Value> {
    vec![buttons_component(json!([
        { "type": "URL", "text": "Clique aqui", "url": "https://example.com/a" },
        { "type": "QUICK_REPLY", "text": "Parar mensagens" },
    ]))]
}

#[test]
fn extracttemplateparams_quick_replies_are_zero_config_creates_no_param_entry_for_quick_reply_buttons()
 {
    let params = fc::extract_template_params(&static_url_then_quick_reply());
    assert!(params.get("button").is_none());
}

#[test]
fn extracttemplateparams_quick_replies_are_zero_config_still_creates_entries_for_dynamic_url_buttons_with_template_index_preserved()
 {
    let params = fc::extract_template_params(&[buttons_component(json!([
        { "type": "QUICK_REPLY", "text": "Stop" },
        { "type": "URL", "text": "Shop", "url": "https://example.com/{{1}}" },
    ]))]);

    assert_eq!(
        params["button"],
        json!([{ "sub_type": "url", "index": 1, "text": "" }])
    );
}

#[test]
fn extractparameterinfos_dense_paramindex_for_button_fields_emits_no_parameter_info_for_quick_reply_buttons()
 {
    assert!(fc::extract_parameter_infos(&static_url_then_quick_reply()).is_empty());
}

#[test]
fn extractparameterinfos_dense_paramindex_for_button_fields_paramindex_is_the_dense_position_while_buttonindex_stays_the_template_index()
 {
    let infos = fc::extract_parameter_infos(&[buttons_component(json!([
        { "type": "PHONE_NUMBER", "text": "Call", "phone_number": "+15550001111" },
        { "type": "URL", "text": "Shop", "url": "https://example.com/{{1}}" },
        { "type": "COPY_CODE", "text": "Copy" },
    ]))]);

    assert_eq!(infos.len(), 2);
    assert_match_object(
        &infos[0],
        &json!({ "buttonSubType": "url", "buttonIndex": 1, "paramIndex": 0 }),
    );
    assert_match_object(
        &infos[1],
        &json!({ "buttonSubType": "copy_code", "buttonIndex": 2, "paramIndex": 1 }),
    );
}

#[test]
fn extracttemplatequickreplybuttons_returns_quick_replies_with_their_template_button_index() {
    assert_eq!(
        fc::extract_template_quick_reply_buttons(&static_url_then_quick_reply()),
        vec![json!({ "templateButtonIndex": 1, "text": "Parar mensagens" })]
    );
}

#[test]
fn extracttemplatequickreplybuttons_returns_an_empty_list_when_the_template_has_no_quick_replies() {
    assert!(
        fc::extract_template_quick_reply_buttons(&[buttons_component(json!([
            { "type": "URL", "text": "Open", "url": "https://example.com" },
        ]))])
        .is_empty()
    );
}

fn status_buttons() -> Vec<Value> {
    vec![
        fc::button_step_default_fn(&json!({ "label": "Delivered" })),
        fc::button_step_default_fn(&json!({ "label": "Failed" })),
    ]
}

#[test]
fn seedwatemplatestepbuttons_keeps_the_status_branches_in_front_and_appends_one_button_per_quick_reply()
 {
    let status = status_buttons();
    let seeded = fc::seed_wa_template_step_buttons(&status, &static_url_then_quick_reply());

    assert_eq!(seeded.len(), fc::WA_TEMPLATE_STATUS_BUTTON_COUNT + 1);
    assert_eq!(seeded[0], status[0]);
    assert_eq!(seeded[1], status[1]);
    assert_eq!(seeded[2]["label"], "Parar mensagens");
}

#[test]
fn seedwatemplatestepbuttons_truncates_quick_reply_labels_to_the_step_button_label_limit() {
    let seeded = fc::seed_wa_template_step_buttons(
        &status_buttons(),
        &[buttons_component(
            json!([{ "type": "QUICK_REPLY", "text": rep("A", 25) }]),
        )],
    );

    assert_eq!(seeded[2]["label"], rep("A", 20));
}

#[test]
fn seedwatemplatestepbuttons_reselecting_a_template_with_the_same_quick_replies_keeps_button_ids_edges_survive()
 {
    let first_seed =
        fc::seed_wa_template_step_buttons(&status_buttons(), &static_url_then_quick_reply());
    let second_seed =
        fc::seed_wa_template_step_buttons(&first_seed, &static_url_then_quick_reply());

    assert_eq!(second_seed[2], first_seed[2]);
}

#[test]
fn seedwatemplatestepbuttons_a_template_text_edit_keeps_the_button_s_id_and_configured_action_only_the_label_follows()
 {
    let first_seed =
        fc::seed_wa_template_step_buttons(&status_buttons(), &static_url_then_quick_reply());
    let renamed_template = vec![buttons_component(json!([
        { "type": "URL", "text": "Clique aqui", "url": "https://example.com/a" },
        { "type": "QUICK_REPLY", "text": "Stop now" },
    ]))];

    let second_seed = fc::seed_wa_template_step_buttons(&first_seed, &renamed_template);

    assert_eq!(second_seed[2]["id"], first_seed[2]["id"]);
    assert_eq!(second_seed[2]["label"], "Stop now");
}

#[test]
fn seedwatemplatestepbuttons_switching_to_a_template_without_quick_replies_drops_the_tail_but_keeps_status_branches()
 {
    let status = status_buttons();
    let seeded = fc::seed_wa_template_step_buttons(
        &fc::seed_wa_template_step_buttons(&status, &static_url_then_quick_reply()),
        &[buttons_component(
            json!([{ "type": "URL", "text": "Open", "url": "https://x.co" }]),
        )],
    );

    assert_eq!(seeded, status);
}

fn base_step() -> Value {
    json!({
        "id": "1",
        "nodeId": "node-1",
        "stepType": "sendWaTemplateMessage",
        "template": { "id": "tmpl-1", "name": "tpl", "language": "en", "inboxId": null, "params": {} },
        "buttons": [],
    })
}

fn with_params(params: Value) -> Value {
    let step = base_step();
    spread(
        &step,
        json!({ "template": spread(&step["template"], json!({ "params": params })) }),
    )
}

#[test]
fn sendwatemplatemessagestepschema_legacy_data_and_configured_buttons_accepts_and_strips_null_holes_persisted_by_the_old_sparse_form_arrays()
 {
    let parsed = fc::send_wa_template_message_step_schema()
        .parse(&with_params(json!({
            "button": [null, { "sub_type": "quick_reply", "index": 1, "payload": "" }],
        })))
        .unwrap();

    assert_eq!(
        parsed["template"]["params"]["button"],
        json!([{ "sub_type": "quick_reply", "index": 1, "payload": "" }])
    );
}

#[test]
fn sendwatemplatemessagestepschema_legacy_data_and_configured_buttons_strips_null_holes_inside_carousel_card_buttons_too()
 {
    let parsed = fc::send_wa_template_message_step_schema()
        .parse(&with_params(json!({
            "carousel": [{ "card_index": 0, "button": [null, { "sub_type": "url", "index": 1, "text": "suffix" }] }],
        })))
        .unwrap();

    assert_eq!(
        parsed["template"]["params"]["carousel"][0]["button"],
        json!([{ "sub_type": "url", "index": 1, "text": "suffix" }])
    );
}

#[test]
fn sendwatemplatemessagestepschema_legacy_data_and_configured_buttons_preserves_a_quick_reply_button_s_configured_action_through_parse_publish_must_not_wipe_it()
 {
    let configured_button = json!({
        "id": "11612473309626372",
        "label": "Parar mensagens",
        "buttonType": "startAnotherNode",
        "beforeStep": { "id": "11612473309626373", "stepType": "startAnotherNode", "nodeId": "11612473309626374" },
        "steps": [],
    });

    let parsed = fc::send_wa_template_message_step_schema()
        .parse(&spread(
            &base_step(),
            json!({
                "buttons": [
                    fc::button_step_default_fn(&json!({ "label": "Delivered" })),
                    fc::button_step_default_fn(&json!({ "label": "Failed" })),
                    configured_button,
                ],
            }),
        ))
        .unwrap();

    assert_match_object(
        &parsed["buttons"][2],
        &json!({
            "label": "Parar mensagens",
            "buttonType": "startAnotherNode",
            "beforeStep": { "nodeId": "11612473309626374" },
        }),
    );
}

#[test]
fn sendwatemplatemessagestepschema_legacy_data_and_configured_buttons_an_empty_buttons_list_still_falls_back_to_the_delivered_failed_status_defaults()
 {
    let parsed = fc::send_wa_template_message_step_schema()
        .parse(&base_step())
        .unwrap();

    let labels: Vec<&str> = parsed["buttons"]
        .as_array()
        .unwrap()
        .iter()
        .map(|b| s(b, "label"))
        .collect();
    assert_eq!(labels, vec!["Delivered", "Failed"]);
}

#[test]
fn bindwatemplatequickreplybuttons_pairs_quick_replies_with_the_seeded_tail_by_position() {
    let tail_button = fc::button_step_default_fn(&json!({ "label": "Parar mensagens" }));
    let mut buttons = status_buttons();
    buttons.push(tail_button.clone());
    let bindings =
        fc::bind_wa_template_quick_reply_buttons(&static_url_then_quick_reply(), &buttons);

    assert_eq!(
        bindings,
        vec![json!({ "templateButtonIndex": 1, "stepButton": tail_button })]
    );
}

#[test]
fn bindwatemplatequickreplybuttons_legacy_steps_with_only_status_buttons_yield_no_bindings() {
    let bindings =
        fc::bind_wa_template_quick_reply_buttons(&static_url_then_quick_reply(), &status_buttons());

    assert!(bindings.is_empty());
}
