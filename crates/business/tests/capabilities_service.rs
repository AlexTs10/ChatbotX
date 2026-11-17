//! Ported from `packages/business/__tests__/capabilities-service.test.ts`
//! (`getCapabilities`). The TS tests spy on the list services; here the
//! loaders run on SQLite, so "loader called" becomes "key present" and the
//! limits are checked on real rows. The `getFlowAuthoringContext` tests are
//! mapped to `flow::authoring_context` (P3.5f) in the test map.

mod common;

use chatbotx_business::capabilities::{
    CAPABILITIES_INCLUDES, OPT_IN_INCLUDES, default_includes, get_capabilities,
};
use chatbotx_db::rusqlite::params;
use common::{WS, setup, two_workspaces};
use serde_json::{Value, json};

fn keys(value: &Value) -> Vec<String> {
    let mut keys: Vec<String> = value.as_object().unwrap().keys().cloned().collect();
    keys.sort();
    keys
}

fn includes(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| (*s).to_string()).collect()
}

#[tokio::test]
async fn omitting_include_fetches_exactly_the_default_set_not_ai_agents() {
    let db = two_workspaces().await;
    let result = get_capabilities(&db, WS, None, None).await;
    let mut expected = includes(&default_includes());
    expected.sort();
    assert_eq!(keys(&result), expected);
    assert!(result.get("aiAgents").is_none());
    assert!(result.get("flowSpec").is_some());
}

#[tokio::test]
async fn projects_the_channel_policies() {
    let db = two_workspaces().await;
    for (channel, unsupported, no_buttons, button_count, native) in [
        ("tiktok", vec!["sendCard"], vec!["sendImage"], 3, json!([])),
        (
            "messenger",
            vec![],
            vec!["sendAudio", "sendFile"],
            3,
            json!(["callPhoneNumber"]),
        ),
        ("omnichannel", vec![], vec![], 3, json!([])),
    ] {
        let result = get_capabilities(&db, WS, Some(&includes(&["flowSpec"])), Some(channel)).await;
        let policy = &result["flowSpec"]["selectedChannelPolicy"];
        assert_eq!(policy["policyVersion"], 3, "{channel}");
        assert_eq!(policy["nativeButtonTypes"], native, "{channel}");
        assert_eq!(policy["limits"]["buttonCount"], button_count, "{channel}");
        for step in unsupported {
            assert!(
                policy["unsupportedSteps"]
                    .as_array()
                    .unwrap()
                    .contains(&json!(step)),
                "{channel} {step}"
            );
        }
        for step in no_buttons {
            assert!(
                policy["noButtonSteps"]
                    .as_array()
                    .unwrap()
                    .contains(&json!(step)),
                "{channel} {step}"
            );
        }
    }
}

#[tokio::test]
async fn exposes_the_product_message_rules_only_for_a_channel_that_declares_them() {
    let db = two_workspaces().await;
    let flow_spec = includes(&["flowSpec"]);
    let whatsapp = get_capabilities(&db, WS, Some(&flow_spec), Some("whatsapp")).await;
    let policy = &whatsapp["flowSpec"]["selectedChannelPolicy"];
    assert!(
        !policy["unsupportedSteps"]
            .as_array()
            .unwrap()
            .contains(&json!("sendProducts"))
    );
    assert_eq!(
        keys(&policy["productMessages"]["displays"]),
        vec!["carousel".to_string(), "products".to_string()]
    );
    let carousel = &policy["productMessages"]["displays"]["carousel"];
    assert_eq!(
        (
            carousel["minProducts"].clone(),
            carousel["maxProducts"].clone()
        ),
        (json!(2), json!(10))
    );
    for channel in ["omnichannel", "messenger", "telegram"] {
        let other = get_capabilities(&db, WS, Some(&flow_spec), Some(channel)).await;
        let policy = &other["flowSpec"]["selectedChannelPolicy"];
        assert!(
            policy["unsupportedSteps"]
                .as_array()
                .unwrap()
                .contains(&json!("sendProducts")),
            "{channel}"
        );
        assert!(policy.get("productMessages").is_none(), "{channel}");
    }
}

#[tokio::test]
async fn whatsapp_unsupported_steps_keep_the_policy_declaration_order() {
    // Adapted: pins the order the parity snapshot (`p1-04/07-capabilities`)
    // records; the Rust policy values are sorted maps.
    let db = two_workspaces().await;
    let result = get_capabilities(&db, WS, Some(&includes(&["flowSpec"])), Some("whatsapp")).await;
    assert_eq!(
        result["flowSpec"]["selectedChannelPolicy"]["unsupportedSteps"],
        json!([
            "sendCard",
            "sendVideo",
            "sendGif",
            "sendAudio",
            "sendFile",
            "sendQuickReply",
            "sendCollection",
            "sendMessengerTemplateMessage"
        ])
    );
}

#[tokio::test]
async fn include_dispatches_only_the_requested_loaders() {
    let db = two_workspaces().await;
    let result = get_capabilities(&db, WS, Some(&includes(&["aiAgents"])), None).await;
    assert_eq!(result, json!({ "aiAgents": [] }));
}

#[tokio::test]
async fn truncates_a_workspaces_tags_and_templates_at_the_capabilities_list_limit() {
    let db = two_workspaces().await;
    setup(&db, |tx| {
        tx.execute(
            r#"INSERT INTO "Inbox" ("id", "name", "channel", "sourceId", "workspaceId") VALUES (3101, 'wa', 'whatsapp', 'phone-1', ?1)"#,
            [WS],
        )?;
        tx.execute(
            r#"INSERT INTO "IntegrationWhatsapp" ("id", "auth", "phoneNumberId", "wabaId", "businessId", "name", "workspaceId", "inboxId") VALUES (101, '{}', 'phone-1', 'waba-1', 'business', 'WhatsApp', ?1, 3101)"#,
            [WS],
        )?;
        for i in 0..250 {
            tx.execute(
                r#"INSERT INTO "Tag" ("id", "name", "workspaceId") VALUES (?1, ?2, ?3)"#,
                params![10_000 + i, format!("Tag {i}"), WS],
            )?;
            tx.execute(
                r#"INSERT INTO "WhatsappMessageTemplate" ("id", "name", "integrationWhatsappId", "sourceId", "language", "category", "status") VALUES (?1, ?2, 101, ?3, 'en', 'MARKETING', 'APPROVED')"#,
                params![20_000 + i, format!("Template {i}"), format!("src-{i}")],
            )?;
        }
        Ok(())
    })
    .await;
    let result = get_capabilities(&db, WS, Some(&includes(&["tags", "templates"])), None).await;
    assert_eq!(result["tags"].as_array().unwrap().len(), 200);
    assert_eq!(result["templates"].as_array().unwrap().len(), 200);
}

#[tokio::test]
async fn paginated_loaders_are_clamped_to_max_limit_50() {
    // Adapted: the TS test checks the requested `perPage` (200); the clamp
    // to `maxLimit` happens in `parsePagination`, so the rows show it.
    let db = two_workspaces().await;
    setup(&db, |tx| {
        for i in 0..60 {
            tx.execute(
                r#"INSERT INTO "Sequence" ("id", "name", "workspaceId") VALUES (?1, ?2, ?3)"#,
                params![30_000 + i, format!("Seq {i}"), WS],
            )?;
        }
        Ok(())
    })
    .await;
    let result = get_capabilities(&db, WS, Some(&includes(&["sequences"])), None).await;
    let sequences = result["sequences"].as_array().unwrap();
    assert_eq!(sequences.len(), 50);
    // The stable sort: ascending ids.
    assert_eq!(sequences[0], json!({ "id": "30000", "name": "Seq 0" }));
}

#[tokio::test]
async fn lists_only_connected_inboxes_of_the_workspace() {
    let db = two_workspaces().await;
    setup(&db, |tx| {
        tx.execute(
            r#"INSERT INTO "Inbox" ("id", "name", "channel", "sourceId", "workspaceId", "status") VALUES (1, 'Support', 'messenger', 'p1', ?1, 'connected'), (2, 'Old', 'messenger', 'p2', ?1, 'disconnected')"#,
            [WS],
        )?;
        Ok(())
    })
    .await;
    let result = get_capabilities(&db, WS, Some(&includes(&["inboxes"])), None).await;
    assert_eq!(
        result["inboxes"],
        json!([{ "id": "1", "name": "Support", "channel": "messenger" }])
    );
}

#[tokio::test]
async fn isolates_a_failing_loader_instead_of_rejecting_the_whole_call() {
    // Adapted: an unknown include is the failing loader here (the TS test
    // makes `tagService.listActive` reject).
    let db = two_workspaces().await;
    let result = get_capabilities(&db, WS, Some(&includes(&["broken", "inboxes"])), None).await;
    assert_eq!(result, json!({ "inboxes": [] }));
}

#[tokio::test]
async fn skips_a_malformed_non_array_template_components_value_instead_of_throwing() {
    let db = two_workspaces().await;
    setup(&db, |tx| {
        tx.execute(
            r#"INSERT INTO "Inbox" ("id", "name", "channel", "sourceId", "workspaceId") VALUES (3101, 'wa', 'whatsapp', 'phone-1', ?1)"#,
            [WS],
        )?;
        tx.execute(
            r#"INSERT INTO "IntegrationWhatsapp" ("id", "auth", "phoneNumberId", "wabaId", "businessId", "name", "workspaceId", "inboxId") VALUES (101, '{}', 'phone-1', 'waba-1', 'business', 'WhatsApp', ?1, 3101)"#,
            [WS],
        )?;
        tx.execute(
            r#"INSERT INTO "WhatsappMessageTemplate" ("id", "name", "integrationWhatsappId", "sourceId", "language", "category", "status", "components") VALUES (1, 'broken_template', 101, 's1', 'en', 'MARKETING', 'APPROVED', '{"not":"an array"}')"#,
            [],
        )?;
        Ok(())
    })
    .await;
    let result = get_capabilities(&db, WS, Some(&includes(&["templates"])), None).await;
    assert_eq!(
        result["templates"],
        json!([{ "id": "1", "name": "broken_template", "language": "en", "status": "APPROVED", "params": {} }])
    );
}

#[test]
fn default_includes_is_every_include_except_the_opt_in_ones() {
    assert_eq!(OPT_IN_INCLUDES, ["aiAgents"]);
    let expected: Vec<&str> = CAPABILITIES_INCLUDES
        .iter()
        .copied()
        .filter(|i| *i != "aiAgents")
        .collect();
    assert_eq!(default_includes(), expected);
}

#[tokio::test]
async fn response_has_the_shared_schema_shape() {
    // Adapted: no shared zod schema in Rust; the shape of every include is
    // checked on real rows instead.
    let db = two_workspaces().await;
    setup(&db, |tx| {
        tx.execute(
            r#"INSERT INTO "Tag" ("id", "name", "workspaceId") VALUES (3, 'vip', ?1)"#,
            [WS],
        )?;
        Ok(())
    })
    .await;
    let all = includes(CAPABILITIES_INCLUDES);
    let result = get_capabilities(&db, WS, Some(&all), None).await;
    let mut expected = all.clone();
    expected.sort();
    assert_eq!(keys(&result), expected);
    assert_eq!(result["tags"], json!([{ "id": "3", "name": "vip" }]));
    let flow_spec = &result["flowSpec"];
    assert_eq!(flow_spec["policyVersion"], 3);
    assert_eq!(
        flow_spec["waitUnits"],
        json!(["seconds", "minutes", "hours", "days"])
    );
    assert_eq!(flow_spec["channels"][0], "omnichannel");
    assert_eq!(flow_spec["stepTypes"][0]["type"], "send");
}

// --- getFlowAuthoringContext (`flow::authoring_context`) ---

async fn context(db: &chatbotx_db::Db) -> chatbotx_core::flow_config::FlowAuthoringContext {
    chatbotx_business::tx::read(db, |conn| {
        chatbotx_business::flow::authoring_context(conn, WS)
    })
    .await
    .expect("authoring context")
}

/// One WhatsApp integration (`101`) of `WS` and its templates
/// `(id, name, language, status, createdAt)`.
async fn templates(
    db: &chatbotx_db::Db,
    rows: &'static [(i64, &'static str, &'static str, &'static str, i64)],
) {
    setup(db, move |tx| {
        tx.execute(
            r#"INSERT INTO "Inbox" ("id", "name", "channel", "sourceId", "workspaceId") VALUES (3101, 'wa', 'whatsapp', 'phone-1', ?1)"#,
            [WS],
        )?;
        tx.execute(
            r#"INSERT INTO "IntegrationWhatsapp" ("id", "auth", "phoneNumberId", "wabaId", "businessId", "name", "workspaceId", "inboxId") VALUES (101, '{}', 'phone-1', 'waba-1', 'business', 'WhatsApp', ?1, 3101)"#,
            [WS],
        )?;
        for (id, name, language, status, created) in rows {
            tx.execute(
                r#"INSERT INTO "WhatsappMessageTemplate" ("id", "name", "integrationWhatsappId", "sourceId", "language", "category", "status", "createdAt") VALUES (?1, ?2, 101, ?3, ?4, 'MARKETING', ?5, ?6)"#,
                params![id, name, format!("src-{id}"), language, status, created],
            )?;
        }
        Ok(())
    })
    .await;
}

#[tokio::test]
async fn authoring_context_returns_only_the_by_name_reference_maps() {
    let db = two_workspaces().await;
    templates(&db, &[(1001, "welcome_promo", "en", "APPROVED", 1)]).await;
    setup(&db, |tx| {
        tx.execute(
            r#"INSERT INTO "CustomField" ("id", "name", "type", "workspaceId") VALUES (1002, 'Plan', 'shortText', ?1)"#,
            [WS],
        )?;
        tx.execute(
            r#"INSERT INTO "Flow" ("id", "name", "workspaceId") VALUES (1003, 'Nurture', ?1)"#,
            [WS],
        )?;
        Ok(())
    })
    .await;
    let ctx = context(&db).await;
    let template = &ctx.templates_by_name["welcome_promo"];
    assert_eq!(
        (
            template.id.as_str(),
            template.language.as_str(),
            template.status.as_str()
        ),
        ("1001", "en", "APPROVED")
    );
    let field = &ctx.custom_fields_by_name["Plan"];
    // Adapted: `shortText` (the SQLite CHECK refuses the TS mock's `text`).
    assert_eq!(
        (field.id.as_str(), field.field_type.as_str()),
        ("1002", "shortText")
    );
    assert_eq!(ctx.flows_by_name["Nurture"].id, "1003");
}

#[tokio::test]
async fn authoring_context_fetches_templates_custom_fields_and_flows_unbounded() {
    // Adapted: the TS test checks that no `perPage` reaches the services; here
    // more rows than any page size all come back.
    let db = two_workspaces().await;
    setup(&db, |tx| {
        for i in 0..60 {
            tx.execute(
                r#"INSERT INTO "Flow" ("id", "name", "workspaceId") VALUES (?1, ?2, ?3)"#,
                params![5_000 + i, format!("Flow {i}"), WS],
            )?;
        }
        Ok(())
    })
    .await;
    assert_eq!(context(&db).await.flows_by_name.len(), 60);
}

#[tokio::test]
async fn authoring_context_maps_pipelines_by_reference_name_with_stages_and_fields() {
    let db = two_workspaces().await;
    setup(&db, |tx| {
        tx.execute(
            r#"INSERT INTO "Pipeline" ("id", "workspaceId", "name", "type", "referenceName", "settings") VALUES (5001, ?1, 'Sales', 'custom', 'sales', '{}')"#,
            [WS],
        )?;
        tx.execute(
            r#"INSERT INTO "PipelineStage" ("id", "workspaceId", "pipelineId", "name", "position") VALUES (5101, ?1, 5001, 'New', 0)"#,
            [WS],
        )?;
        tx.execute(
            r#"INSERT INTO "PipelineField" ("id", "workspaceId", "pipelineId", "name", "referenceName", "type", "position") VALUES (5201, ?1, 5001, 'Budget', 'budget', 'number', 0)"#,
            [WS],
        )?;
        Ok(())
    })
    .await;
    let ctx = context(&db).await;
    let sales = &ctx.pipelines_by_reference_name.as_ref().expect("pipelines")["sales"];
    assert_eq!(
        (
            sales.id.as_str(),
            sales.name.as_str(),
            sales.reference_name.as_str()
        ),
        ("5001", "Sales", "sales")
    );
    assert_eq!(
        sales
            .stages
            .iter()
            .map(|s| (s.id.as_str(), s.name.as_str()))
            .collect::<Vec<_>>(),
        vec![("5101", "New")]
    );
    assert_eq!(
        sales
            .fields
            .iter()
            .map(|f| (f.id.as_str(), f.name.as_str(), f.reference_name.as_str()))
            .collect::<Vec<_>>(),
        vec![("5201", "Budget", "budget")]
    );
}

#[tokio::test]
async fn authoring_context_prefers_the_approved_template_on_a_name_collision() {
    let db = two_workspaces().await;
    templates(
        &db,
        &[
            (1, "welcome_promo", "en", "REJECTED", 1),
            (2, "welcome_promo", "es", "APPROVED", 2),
        ],
    )
    .await;
    let ctx = context(&db).await;
    let template = &ctx.templates_by_name["welcome_promo"];
    assert_eq!(
        (
            template.id.as_str(),
            template.language.as_str(),
            template.status.as_str()
        ),
        ("2", "es", "APPROVED")
    );
}
