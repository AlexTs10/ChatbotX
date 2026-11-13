//! Port of `packages/business/__tests__/contact-custom-field-set-values.test.ts`
//! (P3.6-D2 slice contact custom-field values). Adapted: the mocked
//! `customFieldModel` / `contactCustomFieldModel` / contact / workspace
//! lookups become real rows; `insertValues` / `updateSet` become the stored
//! `ContactCustomField` rows; `emitCustomFieldChanged` becomes the returned
//! change list (no event bus); a "contact row absent" is a contact of another
//! workspace (the value row needs a real contact). The logger assertions are
//! not observable (the stored rows are asserted). The three
//! `emitCustomFieldChanges` cases are not-applicable (no event bus, no cache).

mod common;

use chatbotx_business::contact_custom_field::{
    FieldChange, SetValueByKeyInput, SetValuesInput, SourceTimezoneStrategy, TemporalInputParsing,
    set_value_by_key, set_values, set_values_in_transaction,
};
use chatbotx_db::rusqlite::params;
use common::{
    OTHER_WS, TestDb, WS, custom_field, custom_field_value, custom_field_values, scalar_i64, setup,
    two_workspaces,
};

/// Contact 10 of `WS` (`contact_zone`); contact 20 of `OTHER_WS`
/// (`Asia/Tokyo`); `WS` zone `workspace_zone`. Fields of `WS`: 31
/// `booking_at` datetime, 32 `birthday` date, 33 `last_location` shortText;
/// field 39 of `OTHER_WS`.
async fn fixture(contact_zone: Option<&'static str>, workspace_zone: &'static str) -> TestDb {
    let db = two_workspaces().await;
    setup(&db, move |tx| {
        tx.execute(
            r#"UPDATE "Workspace" SET "timezone" = ?1 WHERE "id" = ?2"#,
            params![workspace_zone, WS],
        )?;
        tx.execute(
            r#"INSERT INTO "Contact" ("id", "firstName", "workspaceId", "timezone") VALUES (10, 'Ada', ?1, ?2), (20, 'Bo', ?3, 'Asia/Tokyo')"#,
            params![WS, contact_zone, OTHER_WS],
        )?;
        custom_field(tx, 31, WS, "booking_at", "datetime")?;
        custom_field(tx, 32, WS, "birthday", "date")?;
        custom_field(tx, 33, WS, "last_location", "shortText")?;
        custom_field(tx, 39, OTHER_WS, "foreign", "datetime")?;
        Ok(())
    })
    .await;
    db
}

fn input(contact_id: i64, field: i64, value: &str) -> SetValuesInput {
    SetValuesInput {
        workspace_id: WS,
        contact_id,
        fields: vec![(field, value.to_string())],
        ..SetValuesInput::default()
    }
}

fn workspace_lenient(field: i64, value: &str) -> SetValuesInput {
    SetValuesInput {
        temporal_input_parsing: Some(TemporalInputParsing::Lenient),
        source_timezone_strategy: Some(SourceTimezoneStrategy::Workspace),
        ..input(10, field, value)
    }
}

fn change(field: i64, name: &str, old: Option<&str>, new: &str) -> FieldChange {
    FieldChange {
        contact_id: 10,
        custom_field_id: field,
        custom_field_name: name.to_string(),
        old_value: old.map(str::to_string),
        new_value: Some(new.to_string()),
    }
}

fn pairs(list: &[(i64, &str)]) -> Vec<(i64, String)> {
    list.iter().map(|(id, v)| (*id, (*v).to_string())).collect()
}

async fn updated_at(db: &TestDb, id: i64) -> i64 {
    scalar_i64(
        db,
        r#"SELECT "updatedAt" FROM "ContactCustomField" WHERE "id" = ?1"#,
        id,
    )
    .await
    .expect("updatedAt")
}

/// A stored value row 501 with an old `updatedAt`.
async fn stored(db: &TestDb, field: i64, value: &'static str) {
    setup(db, move |tx| {
        custom_field_value(tx, 501, 10, field, value)?;
        tx.execute(
            r#"UPDATE "ContactCustomField" SET "updatedAt" = 1000 WHERE "id" = 501"#,
            [],
        )?;
        Ok(())
    })
    .await;
}

// ---- contactCustomFieldService.setValues — timezone resolution ----

#[tokio::test]
async fn contact_custom_field_service_set_values_timezone_resolution_falls_back_to_the_workspace_timezone_when_the_contact_row_is_absent()
 {
    let db = fixture(Some("Asia/Tokyo"), "Asia/Ho_Chi_Minh").await;
    // Contact 20 is not a contact of `WS`: its zone is not read.
    let changes = set_values(&db, input(20, 31, "2026-07-22 15:30"))
        .await
        .expect("set");
    let expected = FieldChange {
        contact_id: 20,
        ..change(31, "booking_at", None, "2026-07-22T08:30:00.000Z")
    };
    assert_eq!(changes, vec![expected]);
    assert_eq!(
        custom_field_values(&db, 20).await,
        pairs(&[(31, "2026-07-22T08:30:00.000Z")])
    );
}

#[tokio::test]
async fn contact_custom_field_service_set_values_timezone_resolution_falls_back_to_the_workspace_timezone_when_the_contact_has_a_null_timezone()
 {
    let db = fixture(None, "Asia/Ho_Chi_Minh").await;
    set_values(&db, input(10, 31, "2026-07-22 15:30"))
        .await
        .expect("set");
    assert_eq!(
        custom_field_values(&db, 10).await,
        pairs(&[(31, "2026-07-22T08:30:00.000Z")])
    );
}

#[tokio::test]
async fn contact_custom_field_service_set_values_timezone_resolution_prefers_the_contact_timezone_over_the_workspace_timezone()
 {
    let db = fixture(Some("Asia/Tokyo"), "Asia/Ho_Chi_Minh").await;
    set_values(&db, input(10, 31, "2026-07-22 15:30"))
        .await
        .expect("set");
    assert_eq!(
        custom_field_values(&db, 10).await,
        pairs(&[(31, "2026-07-22T06:30:00.000Z")])
    );
}

#[tokio::test]
async fn contact_custom_field_service_set_values_timezone_resolution_stores_date_values_offset_preserved_in_the_source_timezone()
 {
    let db = fixture(Some("Asia/Ho_Chi_Minh"), "UTC").await;
    let changes = set_values(&db, input(10, 32, "2026-07-22"))
        .await
        .expect("set");
    assert_eq!(
        changes,
        vec![change(32, "birthday", None, "2026-07-22T00:00:00+07:00")]
    );
    assert_eq!(
        custom_field_values(&db, 10).await,
        pairs(&[(32, "2026-07-22T00:00:00+07:00")])
    );
}

// ---- contactCustomFieldService.setValues — write/emit contract ----

#[tokio::test]
async fn contact_custom_field_service_set_values_write_emit_contract_skips_an_un_normalizable_temporal_value_no_insert_no_emit()
 {
    let db = fixture(Some("Asia/Ho_Chi_Minh"), "UTC").await;
    let changes = set_values(&db, input(10, 32, "2026-02-30"))
        .await
        .expect("set");
    assert!(changes.is_empty());
    assert!(custom_field_values(&db, 10).await.is_empty());
}

#[tokio::test]
async fn contact_custom_field_service_set_values_write_emit_contract_is_a_no_op_when_the_normalized_value_already_matches_the_stored_value()
 {
    let db = fixture(Some("Asia/Ho_Chi_Minh"), "UTC").await;
    stored(&db, 31, "2026-07-22T08:30:00.000Z").await;
    let changes = set_values(&db, input(10, 31, "2026-07-22 15:30"))
        .await
        .expect("set");
    assert!(changes.is_empty());
    assert_eq!(updated_at(&db, 501).await, 1000);
}

#[tokio::test]
async fn contact_custom_field_service_set_values_write_emit_contract_touch_unchanged_refreshes_updated_at_on_an_unchanged_value_without_emitting()
 {
    let db = fixture(Some("Asia/Ho_Chi_Minh"), "UTC").await;
    stored(&db, 33, "10.77,106.70").await;
    let changes = set_values(
        &db,
        SetValuesInput {
            touch_unchanged: true,
            ..input(10, 33, "10.77,106.70")
        },
    )
    .await
    .expect("set");
    assert!(changes.is_empty());
    assert!(updated_at(&db, 501).await > 1000);
    assert_eq!(
        custom_field_values(&db, 10).await,
        pairs(&[(33, "10.77,106.70")])
    );
}

#[tokio::test]
async fn contact_custom_field_service_set_values_write_emit_contract_updates_an_existing_value_in_place_and_emits_old_new()
 {
    let db = fixture(Some("Asia/Ho_Chi_Minh"), "UTC").await;
    stored(&db, 31, "2020-01-01T00:00:00.000Z").await;
    let changes = set_values(&db, input(10, 31, "2026-07-22 15:30"))
        .await
        .expect("set");
    assert_eq!(
        changes,
        vec![change(
            31,
            "booking_at",
            Some("2020-01-01T00:00:00.000Z"),
            "2026-07-22T08:30:00.000Z"
        )]
    );
    let row = scalar_i64(
        &db,
        r#"SELECT count(*) FROM "ContactCustomField" WHERE "id" = ?1 AND "value" = '2026-07-22T08:30:00.000Z'"#,
        501,
    )
    .await;
    assert_eq!(row, Some(1));
}

#[tokio::test]
async fn contact_custom_field_service_set_values_write_emit_contract_does_nothing_when_no_custom_field_definitions_match_the_workspace()
 {
    let db = fixture(Some("Asia/Ho_Chi_Minh"), "UTC").await;
    let changes = set_values(&db, input(10, 39, "whatever"))
        .await
        .expect("set");
    assert!(changes.is_empty());
    assert!(custom_field_values(&db, 10).await.is_empty());
}

// ---- contactCustomFieldService.setValues — lenient spreadsheet parsing ----

#[tokio::test]
async fn contact_custom_field_service_set_values_lenient_spreadsheet_parsing_parses_a_dmy_datetime_cell_and_stores_it_as_the_workspace_anchored_utc_instant()
 {
    let db = fixture(Some("Asia/Tokyo"), "Asia/Ho_Chi_Minh").await;
    set_values(&db, workspace_lenient(31, "23/07/2026 09:30"))
        .await
        .expect("set");
    assert_eq!(
        custom_field_values(&db, 10).await,
        pairs(&[(31, "2026-07-23T02:30:00.000Z")])
    );
}

#[tokio::test]
async fn contact_custom_field_service_set_values_lenient_spreadsheet_parsing_stores_canonical_sheet_datetimes_as_workspace_local_even_when_the_contact_has_another_timezone()
 {
    let db = fixture(Some("Asia/Tokyo"), "Asia/Ho_Chi_Minh").await;
    set_values(&db, workspace_lenient(31, "2026-07-23 09:30"))
        .await
        .expect("set");
    assert_eq!(
        custom_field_values(&db, 10).await,
        pairs(&[(31, "2026-07-23T02:30:00.000Z")])
    );
}

#[tokio::test]
async fn contact_custom_field_service_set_values_lenient_spreadsheet_parsing_parses_a_unix_timestamp_cell_into_a_datetime_utc_instant()
 {
    let db = fixture(Some("Asia/Tokyo"), "Asia/Ho_Chi_Minh").await;
    set_values(&db, workspace_lenient(31, "1721800800"))
        .await
        .expect("set");
    assert_eq!(
        custom_field_values(&db, 10).await,
        pairs(&[(31, "2024-07-24T06:00:00.000Z")])
    );
}

#[tokio::test]
async fn contact_custom_field_service_set_values_lenient_spreadsheet_parsing_warns_and_skips_an_unparseable_cell_under_lenient_parsing()
 {
    let db = fixture(Some("Asia/Tokyo"), "Asia/Ho_Chi_Minh").await;
    let changes = set_values(&db, workspace_lenient(31, "definitely not a date"))
        .await
        .expect("set");
    assert!(changes.is_empty());
    assert!(custom_field_values(&db, 10).await.is_empty());
}

#[tokio::test]
async fn contact_custom_field_service_set_values_lenient_spreadsheet_parsing_strict_default_does_not_warn_when_skipping_an_un_normalizable_value()
 {
    let db = fixture(Some("Asia/Tokyo"), "Asia/Ho_Chi_Minh").await;
    let changes = set_values(&db, input(10, 32, "2026-02-30"))
        .await
        .expect("set");
    assert!(changes.is_empty());
    assert!(custom_field_values(&db, 10).await.is_empty());
}

// ---- contactCustomFieldService.setValueByKey — temporal forwarding ----

#[tokio::test]
async fn contact_custom_field_service_set_value_by_key_temporal_forwarding_forwards_lenient_parsing_and_source_timezone_override_into_the_set_values_funnel()
 {
    // Contact and workspace zones differ from the override: the override wins.
    let db = fixture(Some("Asia/Tokyo"), "UTC").await;
    set_value_by_key(
        &db,
        SetValueByKeyInput {
            workspace_id: WS,
            contact_id: 10,
            keyword: "31".to_string(),
            value: "23/07/2026 09:30".to_string(),
            source_timezone_override: Some("Asia/Ho_Chi_Minh".to_string()),
            temporal_input_parsing: Some(TemporalInputParsing::Lenient),
            ..SetValueByKeyInput::default()
        },
    )
    .await
    .expect("set");
    assert_eq!(
        custom_field_values(&db, 10).await,
        pairs(&[(31, "2026-07-23T02:30:00.000Z")])
    );
}

// ---- contactCustomFieldService.setValueByKey — touchUnchanged forwarding ----

fn location_by_key(touch_unchanged: bool) -> SetValueByKeyInput {
    SetValueByKeyInput {
        workspace_id: WS,
        contact_id: 10,
        keyword: "33".to_string(),
        value: "10.77,106.70".to_string(),
        touch_unchanged,
        ..SetValueByKeyInput::default()
    }
}

#[tokio::test]
async fn contact_custom_field_service_set_value_by_key_touch_unchanged_forwarding_touches_the_unchanged_row_when_touch_unchanged_is_set()
 {
    let db = fixture(None, "UTC").await;
    stored(&db, 33, "10.77,106.70").await;
    let changes = set_value_by_key(&db, location_by_key(true))
        .await
        .expect("set");
    assert!(changes.is_empty());
    assert!(updated_at(&db, 501).await > 1000);
}

#[tokio::test]
async fn contact_custom_field_service_set_value_by_key_touch_unchanged_forwarding_leaves_the_unchanged_row_alone_by_default()
 {
    let db = fixture(None, "UTC").await;
    stored(&db, 33, "10.77,106.70").await;
    set_value_by_key(&db, location_by_key(false))
        .await
        .expect("set");
    assert_eq!(updated_at(&db, 501).await, 1000);
}

// ---- contactCustomFieldService.setValuesInTransaction — write-only, defers side effects ----

#[tokio::test]
async fn contact_custom_field_service_set_values_in_transaction_write_only_defers_side_effects_persists_the_value_but_emits_no_event_and_invalidates_no_cache()
 {
    let db = fixture(Some("Asia/Ho_Chi_Minh"), "UTC").await;
    let changes = db
        .write(|tx| {
            Ok(set_values_in_transaction(
                tx,
                &input(10, 31, "2026-07-22 15:30"),
                5_000,
            ))
        })
        .await
        .expect("write")
        .expect("set");
    assert_eq!(
        changes,
        vec![change(31, "booking_at", None, "2026-07-22T08:30:00.000Z")]
    );
    assert_eq!(
        custom_field_values(&db, 10).await,
        pairs(&[(31, "2026-07-22T08:30:00.000Z")])
    );
}

#[tokio::test]
async fn contact_custom_field_service_set_values_in_transaction_write_only_defers_side_effects_returns_an_empty_change_list_when_nothing_changed()
 {
    let db = fixture(Some("Asia/Ho_Chi_Minh"), "UTC").await;
    stored(&db, 31, "2026-07-22T08:30:00.000Z").await;
    let changes = db
        .write(|tx| {
            Ok(set_values_in_transaction(
                tx,
                &input(10, 31, "2026-07-22 15:30"),
                5_000,
            ))
        })
        .await
        .expect("write")
        .expect("set");
    assert!(changes.is_empty());
    assert_eq!(updated_at(&db, 501).await, 1000);
}
