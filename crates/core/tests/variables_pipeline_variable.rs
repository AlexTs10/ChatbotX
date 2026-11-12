//! Port of `packages/variables/__tests__/pipeline-variable.test.ts`.

mod support;

use chatbotx_core::variables::contact_variable::ContactInboxArg;
use chatbotx_core::variables::deep_replace::{DeepOptions, DeepSource};
use chatbotx_core::variables::pipeline::{
    PipelineVariable, PipelineVariableAttribute, parse_pipeline_variable,
};
use chatbotx_core::variables::*;
use serde_json::json;
use support::iso_ms;
use support::variables::*;

fn contact() -> ContactModel {
    ContactModel {
        id: "contact-1".into(),
        workspace_id: "workspace-1".into(),
        first_name: s("Ada"),
        locale: None,
        timezone: s("UTC"),
        ..Default::default()
    }
}

fn contact_inbox() -> ContactInboxModel {
    ContactInboxModel {
        id: "inbox-1".into(),
        ..Default::default()
    }
}

fn workspace() -> WorkspaceModel {
    WorkspaceModel {
        id: "workspace-1".into(),
        timezone: s("UTC"),
        ..Default::default()
    }
}

fn sales() -> PipelineVariableTarget {
    PipelineVariableTarget {
        id: "900".into(),
        reference_name: "sales".into(),
        name: "Big deal".into(),
        stage_name: "Qualified".into(),
        status: "won".into(),
        status_label: "Closed won".into(),
        priority: "high".into(),
        value: Some(1234.5),
        currency: "USD".into(),
        assigned_admins: vec!["Alice".into(), "Bob".into()],
        created_at: iso_ms("2026-10-01T12:00:00Z"),
        due_date: Some(iso_ms("2026-10-20T12:00:00Z")),
        updated_at: iso_ms("2026-10-05T12:00:00Z"),
    }
}

/// Like `getAll`: every context owns one lazy loader.
fn make_context() -> VariableContext {
    VariableContext {
        contact: contact(),
        contact_inbox: Some(contact_inbox()),
        workspace: Some(workspace()),
        conversation: None,
        pipeline_items: Some(PipelineItemsLoader::for_contact(&contact())),
        ..Default::default()
    }
}

fn harness() -> Harness {
    Harness::new(FakeState {
        pipeline_targets: Ok(vec![sales()]),
        contact: Some(contact()),
        ..Default::default()
    })
}

async fn render(h: &Harness, text: &str) -> Result<String, VariableError> {
    h.v().replace_all(text, &make_context(), None).await
}

fn source() -> DeepSource {
    DeepSource {
        contact_inbox: ContactInboxArg::Model(contact_inbox()),
        workspace: Some(workspace()),
        ..Default::default()
    }
}

/// Port of: packages/variables/__tests__/pipeline-variable.test.ts:129 "parsePipelineVariable > accepts exactly pipelines.<reference_name>.<attribute>"
#[test]
fn parsepipelinevariable_accepts_exactly_pipelines_reference_name_attribute() {
    assert_eq!(
        parse_pipeline_variable("pipelines.sales.stage_name"),
        Some(PipelineVariable {
            reference_name: "sales".into(),
            attribute: PipelineVariableAttribute::StageName
        })
    );
    for bad in [
        "pipelines.sales",
        "pipelines.sales.unknown",
        "pipelines.Sales.name",
        "pipelines.sales.name.extra",
        "pipelines..name",
        "pipeline.sales.name",
    ] {
        assert_eq!(parse_pipeline_variable(bad), None, "{bad}");
    }
}

/// Port of: packages/variables/__tests__/pipeline-variable.test.ts:148 "pipeline variables in message text > resolves all ten attributes from the contact's item"
#[tokio::test]
async fn pipeline_variables_in_message_text_resolves_all_ten_attributes_from_the_contact_s_item() {
    let h = harness();
    for attribute in PipelineVariableAttribute::ALL {
        let rendered = render(&h, &format!("{{{{pipelines.sales.{attribute}}}}}"))
            .await
            .unwrap();
        let expected = match attribute {
            PipelineVariableAttribute::Name => Some("Big deal"),
            PipelineVariableAttribute::StageName => Some("Qualified"),
            PipelineVariableAttribute::Status => Some("Closed won"),
            PipelineVariableAttribute::Priority => Some("high"),
            PipelineVariableAttribute::Value => Some("USD 1,234.50"),
            PipelineVariableAttribute::AssignedAdmins => Some("Alice, Bob"),
            PipelineVariableAttribute::Id => Some("900"),
            _ => None,
        };
        if let Some(expected) = expected {
            assert_eq!(rendered, expected);
        }
        assert_ne!(rendered, "");
    }
    assert!(
        render(&h, "{{pipelines.sales.created_date}}")
            .await
            .unwrap()
            .contains("2026")
    );
    assert!(
        render(&h, "{{pipelines.sales.due_date}}")
            .await
            .unwrap()
            .contains("2026")
    );
}

/// Port of: packages/variables/__tests__/pipeline-variable.test.ts:174 "pipeline variables in message text > an unknown pipeline renders empty, a malformed token stays literal"
#[tokio::test]
async fn pipeline_variables_in_message_text_an_unknown_pipeline_renders_empty_a_malformed_token_stays_literal()
 {
    let h = harness();
    assert_eq!(render(&h, "[{{pipelines.nope.name}}]").await.unwrap(), "[]");
    assert_eq!(
        render(&h, "[{{pipelines.sales.bogus}}]").await.unwrap(),
        "[{{pipelines.sales.bogus}}]"
    );
}

/// Port of: packages/variables/__tests__/pipeline-variable.test.ts:181 "pipeline variables in message text > empty value, due date and admins render empty"
#[tokio::test]
async fn pipeline_variables_in_message_text_empty_value_due_date_and_admins_render_empty() {
    let h = harness();
    h.fake.set(|st| {
        st.pipeline_targets = Ok(vec![PipelineVariableTarget {
            value: None,
            due_date: None,
            assigned_admins: vec![],
            ..sales()
        }])
    });
    assert_eq!(
        render(&h, "[{{pipelines.sales.value}}|{{pipelines.sales.due_date}}|{{pipelines.sales.assigned_admins}}]").await.unwrap(),
        "[||]"
    );
}

/// Port of: packages/variables/__tests__/pipeline-variable.test.ts:193 "pipeline variables in message text > loads once per replaceAll however many variables it has"
#[tokio::test]
async fn pipeline_variables_in_message_text_loads_once_per_replaceall_however_many_variables_it_has()
 {
    let h = harness();
    render(
        &h,
        "{{pipelines.sales.name}} {{pipelines.sales.status}} {{pipelines.support.id}}",
    )
    .await
    .unwrap();
    assert_eq!(h.fake.count("load_pipeline_variable_targets"), 1);
    assert!(
        h.fake
            .called("load_pipeline_variable_targets(workspace-1, contact-1)")
    );
}

/// Port of: packages/variables/__tests__/pipeline-variable.test.ts:205 "pipeline variables in message text > one context shares a single load across separate replaceAll calls"
#[tokio::test]
async fn pipeline_variables_in_message_text_one_context_shares_a_single_load_across_separate_replaceall_calls()
 {
    let h = harness();
    let variables = make_context();
    h.v()
        .replace_all("{{pipelines.sales.name}}", &variables, None)
        .await
        .unwrap();
    h.v()
        .replace_all("{{pipelines.sales.id}}", &variables, None)
        .await
        .unwrap();
    assert_eq!(h.fake.count("load_pipeline_variable_targets"), 1);
}

/// Port of: packages/variables/__tests__/pipeline-variable.test.ts:220 "pipeline variables in message text > does not load anything when no pipeline variable is used"
#[tokio::test]
async fn pipeline_variables_in_message_text_does_not_load_anything_when_no_pipeline_variable_is_used()
 {
    let h = harness();
    render(&h, "Hi {{first_name}}").await.unwrap();
    assert_eq!(h.fake.count("load_pipeline_variable_targets"), 0);
}

/// Port of: packages/variables/__tests__/pipeline-variable.test.ts:226 "pipeline variables in message text > reuses a context whose items are already loading"
#[tokio::test]
async fn pipeline_variables_in_message_text_reuses_a_context_whose_items_are_already_loading() {
    let h = harness();
    let variables = VariableContext {
        pipeline_items: Some(PipelineItemsLoader::preloaded(vec![sales()])),
        ..make_context()
    };
    let text = h
        .v()
        .replace_all("{{pipelines.sales.name}}", &variables, None)
        .await
        .unwrap();
    assert_eq!(text, "Big deal");
    assert_eq!(h.fake.count("load_pipeline_variable_targets"), 0);
}

/// Port of: packages/variables/__tests__/pipeline-variable.test.ts:240 "{{last_opportunity_id}} > renders the contact's last created or updated item id"
#[tokio::test]
async fn last_opportunity_id_renders_the_contact_s_last_created_or_updated_item_id() {
    let h = harness();
    h.fake.set(|st| st.last_item_id = s("9001"));
    assert_eq!(
        render(&h, "Ref {{last_opportunity_id}}").await.unwrap(),
        "Ref 9001"
    );
    assert!(
        h.fake
            .called("load_last_pipeline_item_id(workspace-1, contact-1)")
    );
    assert_eq!(h.fake.count("load_pipeline_variable_targets"), 0);
}

/// Port of: packages/variables/__tests__/pipeline-variable.test.ts:253 "{{last_opportunity_id}} > repeated tokens in one context query once"
#[tokio::test]
async fn last_opportunity_id_repeated_tokens_in_one_context_query_once() {
    let h = harness();
    h.fake.set(|st| st.last_item_id = s("9001"));
    assert_eq!(
        render(&h, "{{last_opportunity_id}}-{{last_opportunity_id}}")
            .await
            .unwrap(),
        "9001-9001"
    );
    assert_eq!(h.fake.count("load_last_pipeline_item_id"), 1);
}

/// Port of: packages/variables/__tests__/pipeline-variable.test.ts:262 "{{last_opportunity_id}} > renders empty when the contact has no item"
#[tokio::test]
async fn last_opportunity_id_renders_empty_when_the_contact_has_no_item() {
    let h = harness();
    assert_eq!(
        render(&h, "Ref [{{last_opportunity_id}}]").await.unwrap(),
        "Ref []"
    );
}

/// Port of: packages/variables/__tests__/pipeline-variable.test.ts:268 "{{last_opportunity_id}} > is not queried when the text does not use it"
#[tokio::test]
async fn last_opportunity_id_is_not_queried_when_the_text_does_not_use_it() {
    let h = harness();
    render(&h, "Hello {{first_name}}").await.unwrap();
    assert_eq!(h.fake.count("load_last_pipeline_item_id"), 0);
}

/// Port of: packages/variables/__tests__/pipeline-variable.test.ts:276 "pipeline items are loaded lazily > text that only mentions pipelines. outside a token does not query"
#[tokio::test]
async fn pipeline_items_are_loaded_lazily_text_that_only_mentions_pipelines_outside_a_token_does_not_query()
 {
    let h = harness();
    render(&h, "see pipelines.sales.name and {{first_name}}")
        .await
        .unwrap();
    h.v()
        .resolve_contact_variables_deep(
            "contact-1",
            &json!({ "note": "pipelines.sales.name {{first_name}}" }),
            source(),
            DeepOptions::default(),
        )
        .await
        .unwrap();
    assert_eq!(h.fake.count("load_pipeline_variable_targets"), 0);
}

/// Port of: packages/variables/__tests__/pipeline-variable.test.ts:289 "pipeline items are loaded lazily > a rejecting loader that nothing awaits raises no unhandled rejection"
#[tokio::test]
async fn pipeline_items_are_loaded_lazily_a_rejecting_loader_that_nothing_awaits_raises_no_unhandled_rejection()
 {
    // Rust futures are lazy: an unread loader never starts, so there is no
    // rejection to leak. The renders succeed and the loader is not called.
    let h = harness();
    h.fake.set(|st| st.pipeline_targets = Err("db down".into()));
    render(&h, "{{first_name}}").await.unwrap();
    let variables = make_context();
    h.v()
        .replace_all("{{first_name}}", &variables, None)
        .await
        .unwrap();
    assert_eq!(h.fake.count("load_pipeline_variable_targets"), 0);
}

/// Port of: packages/variables/__tests__/pipeline-variable.test.ts:310 "pipeline items are loaded lazily > a rejecting loader still fails a render that reads a pipeline variable"
#[tokio::test]
async fn pipeline_items_are_loaded_lazily_a_rejecting_loader_still_fails_a_render_that_reads_a_pipeline_variable()
 {
    let h = harness();
    h.fake.set(|st| st.pipeline_targets = Err("db down".into()));
    assert!(render(&h, "{{pipelines.sales.name}}").await.is_err());
}

/// Port of: packages/variables/__tests__/pipeline-variable.test.ts:343 "pipeline variables in deep replace > resolve in every string leaf with one load, matching message text"
#[tokio::test]
async fn pipeline_variables_in_deep_replace_resolve_in_every_string_leaf_with_one_load_matching_message_text()
 {
    let h = harness();
    let result = h
        .v()
        .resolve_contact_variables_deep(
            "contact-1",
            &json!({
                "title": "{{pipelines.sales.name}}",
                "nested": [{ "line": "{{pipelines.sales.value}} in {{pipelines.sales.stage_name}}" }],
                "unknown": "[{{pipelines.nope.status}}]"
            }),
            source(),
            DeepOptions::default(),
        )
        .await
        .unwrap();
    let title = render(&h, "{{pipelines.sales.name}}").await.unwrap();
    assert_eq!(
        result,
        json!({ "title": title, "nested": [{ "line": "USD 1,234.50 in Qualified" }], "unknown": "[]" })
    );
    // One load for the deep structure, one for the `render` call above.
    assert_eq!(h.fake.count("load_pipeline_variable_targets"), 2);
}

/// Port of: packages/variables/__tests__/pipeline-variable.test.ts:369 "pipeline variables in deep replace > does not load pipeline items for a structure that never references them"
#[tokio::test]
async fn pipeline_variables_in_deep_replace_does_not_load_pipeline_items_for_a_structure_that_never_references_them()
 {
    let h = harness();
    h.v()
        .resolve_contact_variables_deep(
            "contact-1",
            &json!({ "greeting": "Hi {{first_name}}" }),
            source(),
            DeepOptions::default(),
        )
        .await
        .unwrap();
    assert_eq!(h.fake.count("load_pipeline_variable_targets"), 0);
}
