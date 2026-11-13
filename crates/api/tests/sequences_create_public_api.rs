//! Port of the `POST /v1/sequences` case of
//! `apps/builder/__tests__/sequences-public-api.test.ts` (P3.6-D2 slice
//! contact sequences). Adapted: the mocked `sequenceService.create` becomes
//! a migrated database; the asserted call becomes the stored row.

mod handlers_common;

use handlers_common::{WS, call, fixture, scalar};
use serde_json::json;

#[tokio::test]
async fn post_v1_sequences_sources_workspace_id_from_context() {
    let f = fixture().await;
    let (status, body) = call(
        &f,
        "sequences.create",
        WS,
        &[],
        None,
        Some(json!({"name": "Welcome series"})),
    )
    .await;
    assert_eq!(status, 201, "{body}");
    let id: i64 = body["sequenceId"]
        .as_str()
        .and_then(|s| s.parse().ok())
        .expect("sequenceId");
    let ws = scalar(
        &f,
        r#"SELECT "workspaceId" FROM "Sequence" WHERE "id" = ?1"#,
        id,
    )
    .await;
    assert_eq!(ws, Some(json!(WS)));
    let name = scalar(&f, r#"SELECT "name" FROM "Sequence" WHERE "id" = ?1"#, id).await;
    assert_eq!(name, Some(json!("Welcome series")));
}
