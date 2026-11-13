//! Port of `apps/builder/__tests__/contacts-filter-fields-public-api.test.ts`
//! (P3.6-D2 slice filter fields). Adapted: the captured handler is
//! `handlers::dispatch` over a migrated database; the mocked
//! `listContactFilterFieldsForAPI` / `resolveContactFilterValueLabels` become
//! real rows, and the asserted delegation becomes the response.

mod handlers_common;

use chatbotx_db::rusqlite::params;
use handlers_common::{OTHER_WS, WS, call, fixture};
use serde_json::json;

// ---- GET /v1/contacts/filter-fields -------------------------------------------

#[tokio::test]
async fn get_v1_contacts_filter_fields_delegates_to_list_contact_filter_fields_for_api_scoped_to_the_workspace()
 {
    let f = fixture().await;
    f.state
        .db
        .write(|tx| {
            tx.execute(
                r#"INSERT INTO "Tag" ("id", "workspaceId", "name") VALUES (1, ?1, 'mine'), (2, ?2, 'theirs')"#,
                params![WS, OTHER_WS],
            )?;
            Ok(())
        })
        .await
        .expect("seed");

    let (status, body) = call(
        &f,
        "contacts.listFilterFields",
        WS,
        &[],
        Some("surface=flowCondition"),
        None,
    )
    .await;

    assert_eq!(status, 200, "{body}");
    assert_eq!(body["tags"], json!([{ "id": "1", "name": "mine" }]));
    assert_eq!(body["customFields"], json!([]));
    assert_eq!(body["botFields"], json!([]));
    // The surface is passed through: flow-only fields, no pipeline items.
    let fields: Vec<&str> = body["staticFields"]
        .as_array()
        .expect("staticFields")
        .iter()
        .filter_map(|f| f["field"].as_str())
        .collect();
    assert!(fields.contains(&"isWithinWorkingHours"));
    assert_eq!(
        body["pipelineItems"],
        json!({ "pipelines": [], "attributes": [] })
    );
}

// ---- POST /v1/contacts/filter-value-labels ------------------------------------

#[tokio::test]
async fn post_v1_contacts_filter_value_labels_resolves_the_ids_in_the_token_s_workspace_and_passes_only_tag_ids()
 {
    let f = fixture().await;
    f.state
        .db
        .write(|tx| {
            tx.execute(
                r#"INSERT INTO "Tag" ("id", "workspaceId", "name") VALUES (1, ?1, 'VIP'), (2, ?2, 'Other')"#,
                params![WS, OTHER_WS],
            )?;
            Ok(())
        })
        .await
        .expect("seed");

    let (status, body) = call(
        &f,
        "contacts.resolveFilterValueLabels",
        WS,
        &[],
        None,
        Some(json!({ "tags": ["1", "2"], "members": ["9"] })),
    )
    .await;

    assert_eq!(status, 200, "{body}");
    // Only the token's workspace, and only the `tags` key.
    assert_eq!(body, json!({ "tags": [{ "id": "1", "name": "VIP" }] }));
}
