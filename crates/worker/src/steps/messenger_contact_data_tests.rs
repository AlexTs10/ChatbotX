//! `updateMessengerContactData` through `runFlowNode` (Q.P7-V4): the part of
//! `apps/worker/__tests__/messenger-contact-data.test.ts` that needs the
//! Messenger context (no Messenger inbox → silent no-op). The profile merge
//! cases are in `crates/business/tests/messenger_contact_data.rs`.

use serde_json::json;

use super::test_support::*;

#[tokio::test]
async fn no_messenger_inbox_is_a_silent_no_op() {
    let fx = Fixture::new().await;
    fx.node(
        json!([{ "id": "s1", "stepType": "updateMessengerContactData" }]),
        json!([]),
    )
    .await;
    fx.run(json!({})).await.expect("the step never fails");
    assert_eq!(
        fx.text(r#"SELECT "firstName" FROM "Contact" WHERE "id" = 3"#)
            .await,
        Some("Ada".into())
    );
}
