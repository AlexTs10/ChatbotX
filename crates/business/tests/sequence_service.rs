//! Port of `packages/business/__tests__/sequence-service.test.ts` and
//! `sequence-step-payload.test.ts` (Q.D5). Adapted: the mocked drizzle
//! calls, `folderService`, `handleStep*Impact` and the template guards
//! become stored rows; "audits X" becomes the returned audit detail (the
//! caller writes it). The template-lock cases are n-a (no template locks,
//! P3.6-D1); the 23505 / non-23505 rethrow cases are one stored-name case.

mod common;

use chatbotx_business::sequence::{self, StepPayload, UpdateSequence, is_stored_delay_consistent};
use chatbotx_db::rusqlite::params;
use common::{OTHER_WS, TestDb, WS, contact, scalar_i64, scalar_text, setup, two_workspaces};

/// Sequence 31 "Welcome" (`WS`, step 41 order 0), 32 "Other" (`WS`), 39
/// (`OTHER_WS`, step 49); flows 61 (`WS`) and 69 (`OTHER_WS`); sequence
/// folder 71 (`WS`), tag folder 72 (`WS`).
async fn db() -> TestDb {
    let db = two_workspaces().await;
    setup(&db, |tx| {
        for (id, name, ws) in [(31, "Welcome", WS), (32, "Other", WS), (39, "Far", OTHER_WS)] {
            tx.execute(
                r#"INSERT INTO "Sequence" ("id", "name", "workspaceId") VALUES (?1, ?2, ?3)"#,
                params![id, name, ws],
            )?;
        }
        for (id, sequence) in [(41, 31), (49, 39)] {
            tx.execute(
                r#"INSERT INTO "SequenceStep" ("id", "sequenceId", "order", "delayDays", "delayMinutes", "delayUnit")
                   VALUES (?1, ?2, 0, 1, 0, 'days')"#,
                params![id, sequence],
            )?;
        }
        for (id, ws) in [(61, WS), (69, OTHER_WS)] {
            tx.execute(
                r#"INSERT INTO "Flow" ("id", "workspaceId", "name") VALUES (?1, ?2, 'f')"#,
                params![id, ws],
            )?;
        }
        for (id, kind) in [(71, "sequence"), (72, "tag")] {
            tx.execute(
                r#"INSERT INTO "Folder" ("id", "name", "workspaceId", "folderType", "paths") VALUES (?1, 'f', ?2, ?3, '[]')"#,
                params![id, WS, kind],
            )?;
        }
        Ok(())
    })
    .await;
    db
}

fn step(order: i64) -> StepPayload {
    StepPayload {
        order,
        ..StepPayload::default()
    }
}

// describe("sequenceService.create")

#[tokio::test]
async fn creates_the_sequence_and_audits() {
    let db = db().await;
    let id = sequence::create(&db, WS, "New", None)
        .await
        .expect("create");
    assert_eq!(
        scalar_text(&db, r#"SELECT "name" FROM "Sequence" WHERE "id" = ?1"#, id).await,
        Some("New".into())
    );
}

#[tokio::test]
async fn throws_validation_exception_on_the_name_field_for_a_23505_unique_violation() {
    let db = db().await;
    let error = sequence::create(&db, WS, "Welcome", None)
        .await
        .unwrap_err();
    assert_eq!(error.code(), Some("validation"));
    assert_eq!(error.message(), "Name is already taken.");
}

// describe("sequenceService folder validation")

#[tokio::test]
async fn create_checks_the_folder_is_a_sequence_folder_of_this_workspace() {
    let db = db().await;
    let id = sequence::create(&db, WS, "In folder", Some(71))
        .await
        .expect("create");
    assert_eq!(
        scalar_i64(
            &db,
            r#"SELECT "folderId" FROM "Sequence" WHERE "id" = ?1"#,
            id
        )
        .await,
        Some(71)
    );
}

#[tokio::test]
async fn create_refuses_a_foreign_folder_before_writing_anything() {
    let db = db().await;
    let error = sequence::create(&db, WS, "X", Some(72)).await.unwrap_err();
    assert_eq!(error.message(), "Folder not found");
    assert_eq!(
        scalar_i64(
            &db,
            r#"SELECT count(*) FROM "Sequence" WHERE "workspaceId" = ?1"#,
            WS
        )
        .await,
        Some(2)
    );
}

#[tokio::test]
async fn update_checks_a_new_folder_id_but_not_a_null_move_to_root() {
    let db = db().await;
    let error = sequence::update(
        &db,
        WS,
        31,
        UpdateSequence {
            folder_id: Some(Some(72)),
            ..UpdateSequence::default()
        },
    )
    .await
    .unwrap_err();
    assert_eq!(error.message(), "Folder not found");
    sequence::update(
        &db,
        WS,
        31,
        UpdateSequence {
            folder_id: Some(Some(71)),
            ..UpdateSequence::default()
        },
    )
    .await
    .expect("move");
    sequence::update(
        &db,
        WS,
        31,
        UpdateSequence {
            folder_id: Some(None),
            ..UpdateSequence::default()
        },
    )
    .await
    .expect("move to root");
    assert_eq!(
        scalar_i64(
            &db,
            r#"SELECT "folderId" FROM "Sequence" WHERE "id" = ?1"#,
            31
        )
        .await,
        None
    );
}

// describe("sequenceService.update")

#[tokio::test]
async fn no_ops_when_nothing_changed() {
    let db = db().await;
    let detail = sequence::update(
        &db,
        WS,
        31,
        UpdateSequence {
            name: Some("Welcome".into()),
            active: Some(true),
            ..UpdateSequence::default()
        },
    )
    .await
    .expect("update");
    assert_eq!(detail, None);
}

#[tokio::test]
async fn updates_and_audits_with_a_generic_detail_for_multi_field_changes() {
    let db = db().await;
    let detail = sequence::update(
        &db,
        WS,
        31,
        UpdateSequence {
            name: Some("Renamed".into()),
            active: Some(false),
            ..UpdateSequence::default()
        },
    )
    .await
    .expect("update");
    assert_eq!(detail.as_deref(), Some("updated a sequence (#31)"));
    assert_eq!(
        scalar_text(&db, r#"SELECT "name" FROM "Sequence" WHERE "id" = ?1"#, 31).await,
        Some("Renamed".into())
    );
}

#[tokio::test]
async fn audits_an_enabled_detail_when_only_active_flips_true() {
    let db = db().await;
    let off = sequence::update(
        &db,
        WS,
        31,
        UpdateSequence {
            active: Some(false),
            ..UpdateSequence::default()
        },
    )
    .await
    .expect("disable");
    assert_eq!(off.as_deref(), Some("disabled a sequence (#31)"));
    let on = sequence::update(
        &db,
        WS,
        31,
        UpdateSequence {
            active: Some(true),
            ..UpdateSequence::default()
        },
    )
    .await
    .expect("enable");
    assert_eq!(on.as_deref(), Some("enabled a sequence (#31)"));
}

#[tokio::test]
async fn update_throws_validation_exception_on_the_name_field_for_a_23505_unique_violation() {
    let db = db().await;
    let error = sequence::update(
        &db,
        WS,
        31,
        UpdateSequence {
            name: Some("Other".into()),
            ..UpdateSequence::default()
        },
    )
    .await
    .unwrap_err();
    assert_eq!(error.code(), Some("validation"));
}

#[tokio::test]
async fn update_propagates_the_not_found_error_and_never_updates() {
    let db = db().await;
    let error = sequence::update(
        &db,
        WS,
        39,
        UpdateSequence {
            name: Some("Mine".into()),
            ..UpdateSequence::default()
        },
    )
    .await
    .unwrap_err();
    assert_eq!(error.message(), "Sequence not found");
    assert_eq!(
        scalar_text(&db, r#"SELECT "name" FROM "Sequence" WHERE "id" = ?1"#, 39).await,
        Some("Far".into())
    );
}

// describe("sequenceService.delete")

#[tokio::test]
async fn verifies_ownership_deletes_and_audits_with_the_sequence_id() {
    let db = db().await;
    sequence::delete(&db, WS, 31).await.expect("delete");
    assert_eq!(
        scalar_i64(
            &db,
            r#"SELECT count(*) FROM "SequenceStep" WHERE "sequenceId" = ?1"#,
            31
        )
        .await,
        Some(0)
    );
    let error = sequence::delete(&db, WS, 39).await.unwrap_err();
    assert_eq!(error.message(), "Sequence not found");
}

// describe("sequenceService.updateStep / deleteStep cross-workspace rejection")

#[tokio::test]
async fn update_step_throws_not_found_for_a_missing_or_foreign_step() {
    let db = db().await;
    for (sequence_id, step_id) in [(31, 999), (31, 49)] {
        let error = sequence::upsert_step(&db, WS, sequence_id, Some(step_id), step(0))
            .await
            .unwrap_err();
        assert_eq!(error.message(), "Step not found");
    }
}

#[tokio::test]
async fn delete_step_throws_not_found_for_a_missing_or_foreign_step() {
    let db = db().await;
    for step_id in [999, 49] {
        let error = sequence::delete_step(&db, WS, step_id, None)
            .await
            .unwrap_err();
        assert_eq!(error.message(), "Step not found");
    }
    assert_eq!(
        scalar_i64(
            &db,
            r#"SELECT count(*) FROM "SequenceStep" WHERE "id" = ?1"#,
            49
        )
        .await,
        Some(1)
    );
}

#[tokio::test]
async fn rejects_a_step_that_belongs_to_a_different_sequence_than_the_one_named() {
    let db = db().await;
    let error = sequence::delete_step(&db, WS, 41, Some(32))
        .await
        .unwrap_err();
    assert_eq!(error.message(), "Step not found");
    sequence::delete_step(&db, WS, 41, Some(31))
        .await
        .expect("named parent");
    assert_eq!(
        scalar_i64(
            &db,
            r#"SELECT count(*) FROM "SequenceStep" WHERE "id" = ?1"#,
            41
        )
        .await,
        Some(0)
    );
}

#[tokio::test]
async fn delete_step_recalculates_contacts_completing_those_without_a_next_step() {
    let db = db().await;
    setup(&db, |tx| {
        contact(tx, 101, WS)?;
        tx.execute(
            r#"INSERT INTO "ContactOnSequence" ("id", "enrolledAt", "currentStep", "status", "nextStepId", "nextRunAt", "contactId", "sequenceId", "workspaceId")
               VALUES (81, 0, 0, 'active', 41, 5, 101, 31, ?1)"#,
            params![WS],
        )?;
        Ok(())
    })
    .await;
    // The contact at currentStep 0 has no step after order 0 once 41 is gone.
    sequence::delete_step(&db, WS, 41, Some(31))
        .await
        .expect("delete");
    assert_eq!(
        scalar_text(
            &db,
            r#"SELECT "status" FROM "ContactOnSequence" WHERE "id" = ?1"#,
            81
        )
        .await,
        Some("completed".into())
    );
}

// describe("sequenceService.upsertStep")

#[tokio::test]
async fn create_path_creates_the_step_and_recalculates() {
    let db = db().await;
    setup(&db, |tx| {
        contact(tx, 101, WS)?;
        tx.execute(
            r#"INSERT INTO "ContactOnSequence" ("id", "enrolledAt", "currentStep", "status", "contactId", "sequenceId", "workspaceId")
               VALUES (81, 0, 0, 'active', 101, 31, ?1)"#,
            params![WS],
        )?;
        Ok(())
    })
    .await;
    let mut data = step(1);
    data.delay_days = Some(0);
    data.delay_minutes = Some(30);
    data.delay_unit = Some("minutes".into());
    let id = sequence::upsert_step(&db, WS, 31, None, data)
        .await
        .expect("create");
    // handleStepCreationImpact: the contact at step 0 now waits for the new step.
    assert_eq!(
        scalar_i64(
            &db,
            r#"SELECT "nextStepId" FROM "ContactOnSequence" WHERE "id" = ?1"#,
            81
        )
        .await,
        Some(id)
    );
    assert!(
        scalar_i64(
            &db,
            r#"SELECT "nextRunAt" FROM "ContactOnSequence" WHERE "id" = ?1"#,
            81
        )
        .await
        .is_some()
    );
}

#[tokio::test]
async fn create_path_reactivates_completed_contacts_and_schedules_a_dispatch() {
    let db = db().await;
    setup(&db, |tx| {
        contact(tx, 101, WS)?;
        common::inbox(tx, 91, WS, "webchat")?;
        common::contact_inbox(tx, 92, 101, 91, "webchat", "s")?;
        tx.execute(
            r#"INSERT INTO "ContactOnSequence" ("id", "enrolledAt", "completedAt", "currentStep", "status", "contactId", "sequenceId", "workspaceId")
               VALUES (81, 1000, 2000, 1, 'completed', 101, 31, ?1)"#,
            params![WS],
        )?;
        Ok(())
    })
    .await;
    let id = sequence::upsert_step(&db, WS, 31, None, step(1))
        .await
        .expect("create");
    assert_eq!(
        scalar_text(
            &db,
            r#"SELECT "status" FROM "ContactOnSequence" WHERE "id" = ?1"#,
            81
        )
        .await,
        Some("active".into())
    );
    // enrolledAt + 1 day (step 41) + 1 day (the new step's default delay).
    assert_eq!(
        scalar_i64(
            &db,
            r#"SELECT "nextRunAt" FROM "ContactOnSequence" WHERE "id" = ?1"#,
            81
        )
        .await,
        Some(1000 + 2 * 86_400_000)
    );
    assert_eq!(
        scalar_i64(
            &db,
            r#"SELECT count(*) FROM "SequenceDispatch" WHERE "stepId" = ?1"#,
            id
        )
        .await,
        Some(1)
    );
    assert_eq!(
        scalar_i64(
            &db,
            r#"SELECT count(*) FROM sequence_schedule s JOIN "SequenceDispatch" d ON d."id" = s.dispatch_id WHERE d."stepId" = ?1"#,
            id
        )
        .await,
        Some(1)
    );
}

#[tokio::test]
async fn update_path_recalculates_when_delay_days_changes() {
    let db = db().await;
    setup(&db, |tx| {
        contact(tx, 101, WS)?;
        tx.execute(
            r#"INSERT INTO "SequenceStep" ("id", "sequenceId", "order", "delayDays", "delayMinutes", "delayUnit") VALUES (42, 31, 1, 1, 0, 'days')"#,
            [],
        )?;
        tx.execute(
            r#"INSERT INTO "ContactOnSequence" ("id", "enrolledAt", "currentStep", "status", "nextStepId", "nextRunAt", "contactId", "sequenceId", "workspaceId")
               VALUES (81, 0, 0, 'active', 42, 7, 101, 31, ?1)"#,
            params![WS],
        )?;
        Ok(())
    })
    .await;
    let mut data = step(1);
    data.delay_days = Some(3);
    let id = sequence::upsert_step(&db, WS, 31, Some(42), data)
        .await
        .expect("update");
    assert_eq!(id, 42);
    assert_eq!(
        scalar_i64(
            &db,
            r#"SELECT "delayDays" FROM "SequenceStep" WHERE "id" = ?1"#,
            42
        )
        .await,
        Some(3)
    );
    assert_ne!(
        scalar_i64(
            &db,
            r#"SELECT "nextRunAt" FROM "ContactOnSequence" WHERE "id" = ?1"#,
            81
        )
        .await,
        Some(7)
    );
}

#[tokio::test]
async fn update_path_does_not_recalculate_when_only_flow_id_changes_and_order_is_unchanged() {
    let db = db().await;
    setup(&db, |tx| {
        contact(tx, 101, WS)?;
        tx.execute(
            r#"INSERT INTO "SequenceStep" ("id", "sequenceId", "order", "delayDays", "delayMinutes", "delayUnit") VALUES (42, 31, 1, 1, 0, 'days')"#,
            [],
        )?;
        tx.execute(
            r#"INSERT INTO "ContactOnSequence" ("id", "enrolledAt", "currentStep", "status", "nextStepId", "nextRunAt", "contactId", "sequenceId", "workspaceId")
               VALUES (81, 0, 0, 'active', 42, 7, 101, 31, ?1)"#,
            params![WS],
        )?;
        Ok(())
    })
    .await;
    let mut data = step(1);
    data.flow_id = Some(61);
    sequence::upsert_step(&db, WS, 31, Some(42), data)
        .await
        .expect("update");
    assert_eq!(
        scalar_i64(
            &db,
            r#"SELECT "nextRunAt" FROM "ContactOnSequence" WHERE "id" = ?1"#,
            81
        )
        .await,
        Some(7)
    );
    assert_eq!(
        scalar_i64(
            &db,
            r#"SELECT "flowId" FROM "SequenceStep" WHERE "id" = ?1"#,
            42
        )
        .await,
        Some(61)
    );
}

// describe("sequenceService step flow validation")

#[tokio::test]
async fn create_step_rejects_a_flow_that_is_not_in_the_workspace_and_inserts_nothing() {
    let db = db().await;
    let mut data = step(1);
    data.flow_id = Some(69);
    let error = sequence::upsert_step(&db, WS, 31, None, data)
        .await
        .unwrap_err();
    assert_eq!(error.message(), "Flow not found");
    assert_eq!(
        scalar_i64(
            &db,
            r#"SELECT count(*) FROM "SequenceStep" WHERE "sequenceId" = ?1"#,
            31
        )
        .await,
        Some(1)
    );
    // updateStep rejects a foreign flow before writing.
    let mut data = step(0);
    data.flow_id = Some(69);
    let error = sequence::upsert_step(&db, WS, 31, Some(41), data)
        .await
        .unwrap_err();
    assert_eq!(error.message(), "Flow not found");
}

#[tokio::test]
async fn create_step_accepts_a_flow_of_the_workspace() {
    let db = db().await;
    let mut data = step(1);
    data.flow_id = Some(61);
    let id = sequence::upsert_step(&db, WS, 31, None, data)
        .await
        .expect("create");
    assert_eq!(
        scalar_i64(
            &db,
            r#"SELECT "flowId" FROM "SequenceStep" WHERE "id" = ?1"#,
            id
        )
        .await,
        Some(61)
    );
}

// `sequence-step-payload.test.ts`

#[tokio::test]
async fn build_create_data_applies_defaults_for_every_optional_field_when_omitted() {
    let db = db().await;
    let id = sequence::upsert_step(&db, WS, 32, None, step(0))
        .await
        .expect("create");
    let row = db
        .read(move |conn| {
            Ok(conn.query_row(
                r#"SELECT json_array("delayDays", "delayMinutes", "delayUnit", "flowId", "specificDateTime", "isActive", "anytime", "sendTimeStart", "sendTimeEnd", "sendDays") FROM "SequenceStep" WHERE "id" = ?1"#,
                params![id],
                |r| r.get::<_, String>(0),
            )?)
        })
        .await
        .expect("row");
    assert_eq!(row, r#"[1,0,"days",null,null,1,1,null,null,null]"#);
}

#[tokio::test]
async fn build_create_data_keeps_explicit_values_and_blank_send_times_become_null() {
    let db = db().await;
    let data = StepPayload {
        order: 0,
        delay_days: Some(0),
        delay_minutes: Some(120),
        delay_unit: Some("hours".into()),
        flow_id: Some(61),
        specific_date_time: Some(Some(1_700_000_000_000)),
        is_active: Some(false),
        anytime: Some(false),
        send_time_start: Some(Some(String::new())),
        send_time_end: Some(Some("18:00".into())),
        send_days: Some(vec!["mon".into(), "tue".into()]),
    };
    let id = sequence::upsert_step(&db, WS, 32, None, data)
        .await
        .expect("create");
    let row = db
        .read(move |conn| {
            Ok(conn.query_row(
                r#"SELECT json_array("delayDays", "delayMinutes", "delayUnit", "flowId", "specificDateTime", "isActive", "anytime", "sendTimeStart", "sendTimeEnd", "sendDays") FROM "SequenceStep" WHERE "id" = ?1"#,
                params![id],
                |r| r.get::<_, String>(0),
            )?)
        })
        .await
        .expect("row");
    assert_eq!(
        row,
        r#"[0,120,"hours",61,1700000000000,0,0,null,"18:00","[\"mon\",\"tue\"]"]"#
    );
}

#[tokio::test]
async fn build_update_data_only_includes_fields_explicitly_present_in_the_input() {
    let db = db().await;
    let mut data = step(0);
    data.send_time_end = Some(Some("09:00".into()));
    sequence::upsert_step(&db, WS, 31, Some(41), data)
        .await
        .expect("set");
    let mut data = step(0);
    data.delay_minutes = Some(0);
    data.specific_date_time = Some(None);
    sequence::upsert_step(&db, WS, 31, Some(41), data)
        .await
        .expect("update");
    assert_eq!(
        scalar_text(
            &db,
            r#"SELECT "sendTimeEnd" FROM "SequenceStep" WHERE "id" = ?1"#,
            41
        )
        .await,
        Some("09:00".into())
    );
    assert_eq!(
        scalar_i64(
            &db,
            r#"SELECT "delayDays" FROM "SequenceStep" WHERE "id" = ?1"#,
            41
        )
        .await,
        Some(1)
    );
}

#[test]
fn stored_delay_consistency() {
    assert!(is_stored_delay_consistent("immediate", 0, 0));
    assert!(is_stored_delay_consistent("minutes", 0, 5));
    assert!(is_stored_delay_consistent("hours", 0, 120));
    assert!(!is_stored_delay_consistent("hours", 0, 90));
    assert!(is_stored_delay_consistent("days", 2, 0));
    assert!(!is_stored_delay_consistent("days", 0, 0));
    assert!(is_stored_delay_consistent("specificTime", 0, 0));
    let _ = OTHER_WS;
}
