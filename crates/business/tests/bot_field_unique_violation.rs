//! Port of `packages/business/__tests__/bot-field-unique-violation.test.ts`
//! (P3.6-D2 slice bot fields). Adapted: the mocked insert / update that
//! throws `23505` becomes a real `(workspaceId, type, name)` collision. The
//! "rethrows a non-unique-violation error" case is not-applicable (no fault
//! injection).

mod common;

use chatbotx_business::bot_field::{CreateBotField, UpdateBotField, create, update_by_key};
use common::{WS, bot_field, setup, two_workspaces};

async fn fixture() -> common::TestDb {
    let db = two_workspaces().await;
    setup(&db, |tx| {
        bot_field(tx, 1, WS, "plan", "shortText", None, 1_000)?;
        bot_field(tx, 2, WS, "tier", "shortText", None, 2_000)?;
        Ok(())
    })
    .await;
    db
}

fn name_taken(error: &chatbotx_business::BusinessError) {
    assert_eq!(error.code(), Some("validation"));
    assert_eq!(error.message(), "Name is already taken");
    match error {
        chatbotx_business::BusinessError::Exception { field, .. } => {
            assert_eq!(field.as_deref(), Some("name"));
        }
        chatbotx_business::BusinessError::Repo(_) => panic!("repo error"),
    }
}

// ---- botFieldService — unique-violation mapping ----

#[tokio::test]
async fn bot_field_service_unique_violation_mapping_create_maps_a_23505_unique_violation_to_validation_exception_name()
 {
    let db = fixture().await;
    let error = create(
        &db,
        WS,
        CreateBotField {
            name: "plan".to_string(),
            r#type: "shortText".to_string(),
            ..CreateBotField::default()
        },
    )
    .await
    .expect_err("taken");
    name_taken(&error);
}

#[tokio::test]
async fn bot_field_service_unique_violation_mapping_update_by_key_via_persist_update_maps_a_23505_unique_violation_to_validation_exception_name()
 {
    let db = fixture().await;
    let error = update_by_key(
        &db,
        WS,
        "tier",
        UpdateBotField {
            name: Some("plan".to_string()),
            ..UpdateBotField::default()
        },
    )
    .await
    .expect_err("taken");
    name_taken(&error);
}
