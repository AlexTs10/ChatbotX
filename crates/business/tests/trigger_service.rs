//! Port of `trigger.service.test.ts`, `trigger-service-update-settings.test.ts`
//! and `trigger-service-update-with-conditions.test.ts` (Q.D5). Adapted: the
//! mocked drizzle / reference calls become stored rows; "audits X" becomes
//! the returned change flag / audit detail (the caller writes it). The
//! template guard / lock cases are n-a (P3.6-D1); the cache refresh is the
//! 5 s RAM cache (no call to assert).

mod common;

use chatbotx_business::automation_action::{
    find_invalid_action_references, indexes_of_new_actions,
};
use chatbotx_business::trigger::{self, ConditionInput, MAX_TRIGGERS_PER_WORKSPACE};
use chatbotx_db::rusqlite::params;
use common::{OTHER_WS, TestDb, WS, scalar_i64, scalar_text, setup, two_workspaces};
use serde_json::json;

/// Trigger 51 (`WS`, actions `[addTag 1]`, conditions 61 tagApplied old and
/// 62 tagApplied 2), trigger 59 (`OTHER_WS`); tag 1; trigger folder 71, tag
/// folder 72; flow 9 whose current version has node 8.
async fn db() -> TestDb {
    let db = two_workspaces().await;
    setup(&db, |tx| {
        tx.execute(
            r#"INSERT INTO "Trigger" ("id", "name", "active", "actions", "workspaceId") VALUES
               (51, 'T', 1, '[{"type":"addTag","tagIds":["1"]}]', ?1), (59, 'Far', 1, '[]', ?2)"#,
            params![WS, OTHER_WS],
        )?;
        tx.execute(
            r#"INSERT INTO "Condition" ("id", "triggerId", "type", "sourceId") VALUES
               (61, 51, 'tagApplied', 'tag-old'), (62, 51, 'tagApplied', 'tag-2')"#,
            [],
        )?;
        tx.execute(
            r#"INSERT INTO "Tag" ("id", "name", "workspaceId") VALUES (1, 'a', ?1)"#,
            params![WS],
        )?;
        for (id, kind) in [(71, "trigger"), (72, "tag")] {
            tx.execute(
                r#"INSERT INTO "Folder" ("id", "name", "workspaceId", "folderType", "paths") VALUES (?1, 'f', ?2, ?3, '[]')"#,
                params![id, WS, kind],
            )?;
        }
        tx.execute(
            r#"INSERT INTO "Flow" ("id", "workspaceId", "name") VALUES (9, ?1, 'f')"#,
            params![WS],
        )?;
        tx.execute(
            r#"INSERT INTO "FlowVersion" ("id", "workspaceId", "flowId", "nodes", "edges", "isDraft", "startNodeId")
               VALUES (90, ?1, 9, '[{"id":"8"}]', '[]', 0, 8)"#,
            params![WS],
        )?;
        tx.execute(r#"UPDATE "Flow" SET "currentVersionId" = 90 WHERE "id" = 9"#, [])?;
        Ok(())
    })
    .await;
    db
}

fn condition(id: Option<&str>, kind: &str, source: Option<&str>) -> ConditionInput {
    ConditionInput {
        id: id.map(str::to_string),
        r#type: kind.to_string(),
        source_id: source.map(str::to_string),
        ..ConditionInput::default()
    }
}

// describe("triggerService.create")

#[tokio::test]
async fn rejects_when_the_workspace_is_already_at_the_trigger_quota() {
    let db = db().await;
    setup(&db, |tx| {
        for n in 0..MAX_TRIGGERS_PER_WORKSPACE - 1 {
            tx.execute(
                r#"INSERT INTO "Trigger" ("id", "name", "actions", "workspaceId") VALUES (?1, ?2, '[]', ?3)"#,
                params![1000 + n, format!("q{n}"), WS],
            )?;
        }
        Ok(())
    })
    .await;
    let error = trigger::create(&db, WS, "x".into(), None)
        .await
        .unwrap_err();
    assert_eq!(error.code(), Some("validation"));
    assert_eq!(error.message(), "validation.maxItemsReached");
    let chatbotx_business::BusinessError::Exception { field, data, .. } = error else {
        panic!("exception");
    };
    assert_eq!(field.as_deref(), Some("_"));
    assert_eq!(data, Some(json!({ "max": 50, "feature": "triggers" })));
}

#[tokio::test]
async fn checks_the_folder_exists_when_a_folder_id_is_given() {
    let db = db().await;
    let error = trigger::create(&db, WS, "x".into(), Some(72))
        .await
        .unwrap_err();
    assert_eq!(error.message(), "Folder not found");
    let created = trigger::create(&db, WS, "x".into(), Some(71))
        .await
        .expect("create");
    assert_eq!(created.folder_id, Some(71));
}

#[tokio::test]
async fn inserts_with_empty_actions_on_success() {
    let db = db().await;
    let created = trigger::create(&db, WS, "New".into(), None)
        .await
        .expect("create");
    assert_eq!(created.name, "New");
    assert_eq!(created.actions.0, json!([]));
    assert!(created.active);
}

// describe("triggerService.deleteMany")

#[tokio::test]
async fn deletes_only_triggers_of_the_workspace_and_names_them_in_the_audit() {
    let db = db().await;
    let deleted = trigger::delete_many(&db, WS, vec![51, 59])
        .await
        .expect("delete");
    assert_eq!(deleted, vec![51]);
    assert_eq!(
        trigger::delete_audit_detail(&deleted).as_deref(),
        Some("deleted trigger (#51)")
    );
    assert_eq!(
        trigger::delete_audit_detail(&[1, 2]).as_deref(),
        Some("deleted triggers (#1, #2)")
    );
    assert_eq!(trigger::delete_audit_detail(&[]), None);
    assert_eq!(
        scalar_i64(&db, r#"SELECT count(*) FROM "Trigger" WHERE "id" = ?1"#, 59).await,
        Some(1)
    );
}

// describe("triggerService.updateSettings")

#[tokio::test]
async fn throws_not_found_exception_when_the_trigger_does_not_exist() {
    let db = db().await;
    let error = trigger::update_settings(&db, WS, 59, Some("x".into()), None)
        .await
        .unwrap_err();
    assert_eq!(error.message(), "Trigger not found");
}

#[tokio::test]
async fn no_ops_without_writing_or_auditing_when_nothing_changed() {
    let db = db().await;
    let (_, detail) = trigger::update_settings(&db, WS, 51, Some("T".into()), Some(true))
        .await
        .expect("update");
    assert_eq!(detail, None);
}

#[tokio::test]
async fn audits_as_enabled_or_disabled_when_only_active_flips() {
    let db = db().await;
    let (row, detail) = trigger::update_settings(&db, WS, 51, None, Some(false))
        .await
        .expect("disable");
    assert_eq!(detail.as_deref(), Some("disabled a trigger (#51)"));
    assert!(!row.trigger.active);
    let (_, detail) = trigger::update_settings(&db, WS, 51, None, Some(true))
        .await
        .expect("enable");
    assert_eq!(detail.as_deref(), Some("enabled a trigger (#51)"));
}

#[tokio::test]
async fn audits_a_generic_updated_detail_when_a_non_active_field_changes() {
    let db = db().await;
    let (row, detail) = trigger::update_settings(&db, WS, 51, Some("Renamed".into()), Some(true))
        .await
        .expect("rename");
    assert_eq!(detail.as_deref(), Some("updated a trigger (#51)"));
    assert_eq!(row.trigger.name, "Renamed");
    assert_eq!(row.conditions.len(), 2);
}

// describe("triggerService.updateWithConditions")

#[tokio::test]
async fn returns_undefined_and_makes_no_writes_when_the_trigger_does_not_exist() {
    let db = db().await;
    let out = trigger::update_with_conditions(&db, WS, 59, vec![], vec![])
        .await
        .expect("update");
    assert!(out.is_none());
    assert_eq!(
        scalar_text(&db, r#"SELECT "name" FROM "Trigger" WHERE "id" = ?1"#, 59).await,
        Some("Far".into())
    );
}

#[tokio::test]
async fn does_not_audit_when_nothing_actually_changed() {
    let db = db().await;
    let out = trigger::update_with_conditions(
        &db,
        WS,
        51,
        vec![json!({ "tagIds": ["1"], "type": "addTag" })],
        vec![
            condition(Some("61"), "tagApplied", Some("tag-old")),
            condition(Some("62"), "tagApplied", Some("tag-2")),
        ],
    )
    .await
    .expect("update")
    .expect("found");
    assert!(!out.1);
    assert_eq!(out.0.conditions.len(), 2);
}

#[tokio::test]
async fn audits_when_actions_changed() {
    let db = db().await;
    let (_, changed) = trigger::update_with_conditions(
        &db,
        WS,
        51,
        vec![],
        vec![
            condition(Some("61"), "tagApplied", Some("tag-old")),
            condition(Some("62"), "tagApplied", Some("tag-2")),
        ],
    )
    .await
    .expect("update")
    .expect("found");
    assert!(changed);
    assert_eq!(
        scalar_text(
            &db,
            r#"SELECT "actions" FROM "Trigger" WHERE "id" = ?1"#,
            51
        )
        .await,
        Some("[]".into())
    );
}

#[tokio::test]
async fn partitions_conditions_into_delete_update_create_and_applies_each() {
    let db = db().await;
    let (found, changed) = trigger::update_with_conditions(
        &db,
        WS,
        51,
        vec![json!({ "type": "addTag", "tagIds": ["1"] })],
        vec![
            condition(Some("61"), "tagApplied", Some("tag-new")),
            condition(None, "newContact", None),
        ],
    )
    .await
    .expect("update")
    .expect("found");
    assert!(changed);
    assert_eq!(
        scalar_text(
            &db,
            r#"SELECT "sourceId" FROM "Condition" WHERE "id" = ?1"#,
            61
        )
        .await,
        Some("tag-new".into())
    );
    assert_eq!(
        scalar_i64(
            &db,
            r#"SELECT count(*) FROM "Condition" WHERE "id" = ?1"#,
            62
        )
        .await,
        Some(0)
    );
    let kinds: Vec<&str> = found.conditions.iter().map(|c| c.r#type.as_str()).collect();
    assert_eq!(kinds.len(), 2);
    assert!(kinds.contains(&"newContact"));
}

// describe("action references")

#[tokio::test]
async fn checks_only_the_actions_that_are_not_already_stored() {
    let next = vec![
        json!({ "type": "addTag", "tagIds": ["1"] }),
        json!({ "type": "startExternalStep", "flowId": "9", "nodeId": "8" }),
    ];
    let stored = vec![json!({ "type": "addTag", "tagIds": ["1"] })];
    let fresh = indexes_of_new_actions(&next, &stored);
    assert_eq!(fresh, [1].into_iter().collect());
    // A repeated stored action counts once.
    let twice = indexes_of_new_actions(&[stored[0].clone(), stored[0].clone()], &stored);
    assert_eq!(twice, [1].into_iter().collect());
    let db = db().await;
    let (_, changed) = trigger::update_with_conditions(&db, WS, 51, next, vec![])
        .await
        .expect("valid new reference")
        .expect("found");
    assert!(changed);
}

#[tokio::test]
async fn a_dangling_new_reference_fails_at_actions_index_field_and_writes_nothing() {
    let db = db().await;
    let error = trigger::update_with_conditions(
        &db,
        WS,
        51,
        vec![json!({ "type": "startExternalStep", "flowId": "9", "nodeId": "7" })],
        vec![],
    )
    .await
    .unwrap_err();
    assert_eq!(
        error.message(),
        "validation.trigger.actionReferenceNotFound"
    );
    let chatbotx_business::BusinessError::Exception { field, data, .. } = error else {
        panic!("exception");
    };
    assert_eq!(field.as_deref(), Some("actions.0.nodeId"));
    assert_eq!(data, Some(json!({ "field": "actions.0.nodeId" })));
    assert_eq!(
        scalar_i64(
            &db,
            r#"SELECT count(*) FROM "Condition" WHERE "triggerId" = ?1"#,
            51
        )
        .await,
        Some(2)
    );
}

#[tokio::test]
async fn nothing_new_means_no_reference_lookup_at_all() {
    let db = db().await;
    // The stored action names tag 1; deleting the tag does not block a save
    // that keeps the action as it is.
    setup(&db, |tx| {
        tx.execute(r#"DELETE FROM "Tag" WHERE "id" = 1"#, [])?;
        Ok(())
    })
    .await;
    let out = trigger::update_with_conditions(
        &db,
        WS,
        51,
        vec![json!({ "type": "addTag", "tagIds": ["1"] })],
        vec![],
    )
    .await
    .expect("update");
    assert!(out.is_some());
}

#[tokio::test]
async fn find_invalid_action_references_resolves_each_kind_in_the_workspace() {
    let db = db().await;
    let problems = chatbotx_business::tx::read(&db, |conn| {
            let actions = [
                json!({ "type": "addTag", "tagIds": ["1", "2", "abc"] }),
                json!({ "type": "startAnotherFlow", "flowId": "9" }),
                json!({ "type": "setCustomField", "customFieldId": "missing" }),
                json!({ "type": "clearCustomField", "customFieldId": "bot_field:5" }),
                json!({ "type": "pipeline", "pipelineId": "3", "action": "moveToStage", "stageId": "4" }),
            ];
            find_invalid_action_references(conn, WS, &actions, None, true)
        })
        .await
        .expect("lookup");
    assert_eq!(
        problems,
        vec![
            (0, "tagIds.1".to_string()),
            (0, "tagIds.2".to_string()),
            (2, "customFieldId".to_string()),
            (3, "customFieldId".to_string()),
            (4, "pipelineId".to_string()),
            (4, "stageId".to_string()),
        ]
    );
}
