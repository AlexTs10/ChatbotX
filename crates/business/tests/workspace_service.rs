//! Ported from `packages/business/__tests__/workspace.service.test.ts` and
//! `packages/business/src/workspace/__tests__/service.test.ts` (the tests
//! that check stored rows; the cache, lock, transaction-ownership, tracker,
//! MAC and template-staging tests check mocks only and are n-a, see
//! `conversion/specs/Q.D11-test-map.tsv`). The services run against a
//! migrated SQLite file.

mod common;

use chatbotx_business::audit::{AuditActor, with_audit_context};
use chatbotx_business::keys::Edition;
use chatbotx_business::workspace::{self, CreateWorkspace, ExtraMember, SettingsPatch};
use chatbotx_db::Db;
use chatbotx_db::rusqlite::params;
use common::{fresh_db, scalar_i64, setup};
use serde_json::json;

const OWNER: i64 = 11;
const WS: i64 = 21;
const REQUESTED_AT: i64 = 1_793_491_200_000; // 2026-11-01T00:00:00Z

type LogRow = (String, Option<i64>, i64, Option<String>, Option<String>);

async fn log_rows(db: &Db) -> Vec<LogRow> {
    db.read(|conn| {
        let mut stmt = conn.prepare(
            r#"SELECT "status", "requestedByUserId", "requestedAt", "resourceName", "ipAddress"
               FROM "ResourceDeletionLog" ORDER BY "id""#,
        )?;
        let rows = stmt
            .query_map([], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    })
    .await
    .expect("log rows")
}

async fn seeded() -> common::TestDb {
    let db = fresh_db().await;
    setup(&db, |tx| {
        common::workspace(tx, OWNER, WS)?;
        tx.execute(
            r#"UPDATE "Workspace" SET "name" = 'Old Name' WHERE "id" = ?1"#,
            [WS],
        )?;
        Ok(())
    })
    .await;
    db
}

/// A workspace scheduled by user 12 at `REQUESTED_AT`.
async fn scheduled(db: &Db, with_pending_log: bool) {
    setup(db, move |tx| {
        tx.execute(r#"INSERT INTO "User" ("id", "email") VALUES (12, 'u12@example.test')"#, [])?;
        tx.execute(
            r#"UPDATE "Workspace" SET "scheduledDeletionAt" = ?2, "deletionRequestedAt" = ?3, "deletionRequestedById" = 12 WHERE "id" = ?1"#,
            params![WS, REQUESTED_AT + 86_400_000, REQUESTED_AT],
        )?;
        if with_pending_log {
            tx.execute(
                r#"INSERT INTO "ResourceDeletionLog" ("id", "workspaceId", "resourceType", "resourceId", "resourceName", "status", "requestedByUserId", "requestedAt")
                   VALUES (1, ?1, 'workspace', ?2, 'Old Name', 'pending', 12, ?3)"#,
                params![WS, WS.to_string(), REQUESTED_AT],
            )?;
        }
        Ok(())
    })
    .await;
}

fn actor() -> AuditActor {
    AuditActor {
        user_id: Some("13".into()),
        workspace_id: Some(WS.to_string()),
        ip_address: Some("203.0.113.9".into()),
        user_agent: Some("ua".into()),
        source: None,
    }
}

async fn add_user(db: &Db, id: i64) {
    setup(db, move |tx| {
        tx.execute(
            r#"INSERT INTO "User" ("id", "email") VALUES (?1, ?2)"#,
            params![id, format!("u{id}@example.test")],
        )?;
        Ok(())
    })
    .await;
}

// --- scheduleDeletion — deletion log ---

#[tokio::test]
async fn schedule_records_a_pending_request_for_the_submitting_actor() {
    let db = seeded().await;
    add_user(&db, 13).await;
    let now = REQUESTED_AT;
    let updated = with_audit_context(actor(), workspace::schedule_deletion(&db, 13, WS, now))
        .await
        .expect("schedule");
    assert_eq!(updated.deletion_requested_by_id, Some(13));
    assert_eq!(updated.deletion_requested_at, Some(now));
    assert_eq!(
        updated.scheduled_deletion_at,
        Some(workspace::deletion_schedule::next_scheduled_deletion_at(
            now
        ))
    );
    let rows = log_rows(&db).await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].0, "pending");
    assert_eq!(rows[0].1, Some(13));
    assert_eq!(rows[0].3.as_deref(), Some("Old Name"));
    assert_eq!(rows[0].4.as_deref(), Some("203.0.113.9"));
}

#[tokio::test]
async fn schedule_closes_the_previous_request_before_recording_a_resubmission() {
    let db = seeded().await;
    scheduled(&db, true).await;
    add_user(&db, 13).await;
    workspace::schedule_deletion(&db, 13, WS, REQUESTED_AT + 1000)
        .await
        .expect("schedule");
    let rows = log_rows(&db).await;
    assert_eq!(rows.len(), 2, "no historical row");
    assert_eq!(rows[0].0, "canceled");
    assert_eq!(rows[1].0, "pending");
    assert_eq!(rows[1].1, Some(13));
}

#[tokio::test]
async fn schedule_recovers_the_earlier_request_of_a_workspace_scheduled_before_the_log_existed() {
    let db = seeded().await;
    scheduled(&db, false).await;
    add_user(&db, 13).await;
    workspace::schedule_deletion(&db, 13, WS, REQUESTED_AT + 1000)
        .await
        .expect("schedule");
    let rows = log_rows(&db).await;
    assert_eq!(rows.len(), 2);
    assert_eq!(
        rows[0],
        (
            "canceled".into(),
            Some(12),
            REQUESTED_AT,
            Some("Old Name".into()),
            None
        )
    );
    assert_eq!((rows[1].0.as_str(), rows[1].1), ("pending", Some(13)));
}

#[tokio::test]
async fn schedule_writes_nothing_when_the_purge_has_already_started() {
    let db = seeded().await;
    setup(&db, |tx| {
        tx.execute(
            r#"UPDATE "Workspace" SET "purgeStartedAt" = 1 WHERE "id" = ?1"#,
            [WS],
        )?;
        Ok(())
    })
    .await;
    let error = workspace::schedule_deletion(&db, OWNER, WS, REQUESTED_AT)
        .await
        .expect_err("fenced");
    assert_eq!(error.code(), Some("workspaceDeletionStarted"));
    assert!(log_rows(&db).await.is_empty());
    assert_eq!(
        scalar_i64(
            &db,
            r#"SELECT "deletionRequestedAt" FROM "Workspace" WHERE "id" = ?1"#,
            WS
        )
        .await,
        None
    );
}

// --- cancelDeletion ---

#[tokio::test]
async fn cancel_closes_the_pending_request_as_canceled() {
    let db = seeded().await;
    scheduled(&db, true).await;
    let updated = workspace::cancel_deletion(&db, WS).await.expect("cancel");
    assert_eq!(updated.scheduled_deletion_at, None);
    assert_eq!(updated.deletion_requested_at, None);
    assert_eq!(updated.deletion_requested_by_id, None);
    let rows = log_rows(&db).await;
    assert_eq!(rows.len(), 1, "no historical row");
    assert_eq!(rows[0].0, "canceled");
}

#[tokio::test]
async fn cancel_recovers_a_request_made_before_the_log_existed() {
    let db = seeded().await;
    scheduled(&db, false).await;
    workspace::cancel_deletion(&db, WS).await.expect("cancel");
    let rows = log_rows(&db).await;
    assert_eq!(rows.len(), 1);
    assert_eq!(
        (rows[0].0.as_str(), rows[0].1, rows[0].2),
        ("canceled", Some(12), REQUESTED_AT)
    );
}

#[tokio::test]
async fn cancel_records_nothing_when_no_deletion_was_ever_requested() {
    let db = seeded().await;
    workspace::cancel_deletion(&db, WS).await.expect("cancel");
    assert!(log_rows(&db).await.is_empty());
}

#[tokio::test]
async fn cancel_is_rejected_after_the_durable_purge_fence_is_set() {
    let db = seeded().await;
    scheduled(&db, true).await;
    setup(&db, |tx| {
        tx.execute(
            r#"UPDATE "Workspace" SET "purgeStartedAt" = 1 WHERE "id" = ?1"#,
            [WS],
        )?;
        Ok(())
    })
    .await;
    let error = workspace::cancel_deletion(&db, WS)
        .await
        .expect_err("fenced");
    assert_eq!(error.code(), Some("workspaceDeletionStarted"));
    assert_eq!(log_rows(&db).await[0].0, "pending");
}

// --- updateSettings ---

#[tokio::test]
async fn update_settings_writes_only_the_settings_columns() {
    // Adapted: `SettingsPatch` has no non-settings field, so the check is that
    // the settings are stored and the name, owner and tenant stay.
    let db = seeded().await;
    setup(&db, |tx| {
        tx.execute(
            r#"INSERT INTO "Flow" ("id", "workspaceId", "name") VALUES (11, ?1, 'F')"#,
            [WS],
        )?;
        Ok(())
    })
    .await;
    let write = workspace::update_settings(
        &db,
        WS,
        SettingsPatch {
            default_reply: Some(Some(11)),
            default_reply_frequency: Some("oncePerDay".into()),
            smart_response_delay_seconds: Some(Some(10)),
            capi_limited_data_use: Some(true),
            logo: Some(Some("https://cdn.example.com/l.png".into())),
            ..SettingsPatch::default()
        },
    )
    .await
    .expect("update");
    let w = write.workspace;
    assert_eq!(w.default_reply.as_deref(), Some("11"));
    assert_eq!(w.default_reply_frequency, "oncePerDay");
    assert_eq!(w.smart_response_delay_seconds, Some(10));
    assert!(w.capi_limited_data_use);
    assert_eq!(w.logo.as_deref(), Some("https://cdn.example.com/l.png"));
    assert_eq!(
        (w.name.as_str(), w.owner_id, w.tenant_id),
        ("Old Name", OWNER, 1)
    );
}

#[tokio::test]
async fn a_default_reply_flow_of_another_workspace_is_refused_and_nothing_is_written() {
    let db = seeded().await;
    setup(&db, |tx| {
        common::workspace(tx, 12, 22)?;
        tx.execute(
            r#"INSERT INTO "Flow" ("id", "workspaceId", "name") VALUES (99, 22, 'F')"#,
            [],
        )?;
        Ok(())
    })
    .await;
    let error = workspace::update_settings(
        &db,
        WS,
        SettingsPatch {
            default_reply: Some(Some(99)),
            capi_limited_data_use: Some(true),
            ..SettingsPatch::default()
        },
    )
    .await
    .expect_err("refused");
    assert_eq!(error.message(), "Flow not found");
    assert_eq!(
        scalar_i64(
            &db,
            r#"SELECT "capiLimitedDataUse" FROM "Workspace" WHERE "id" = ?1"#,
            WS
        )
        .await,
        Some(0)
    );
}

#[tokio::test]
async fn clearing_the_default_reply_needs_no_flow() {
    let db = seeded().await;
    setup(&db, |tx| {
        tx.execute(
            r#"UPDATE "Workspace" SET "defaultReply" = '5' WHERE "id" = ?1"#,
            [WS],
        )?;
        Ok(())
    })
    .await;
    let write = workspace::update_settings(
        &db,
        WS,
        SettingsPatch {
            default_reply: Some(None),
            ..SettingsPatch::default()
        },
    )
    .await
    .expect("update");
    assert_eq!(write.workspace.default_reply, None);
}

#[tokio::test]
async fn an_empty_body_writes_nothing_and_returns_the_workspace() {
    let db = seeded().await;
    let before = scalar_i64(
        &db,
        r#"SELECT "updatedAt" FROM "Workspace" WHERE "id" = ?1"#,
        WS,
    )
    .await;
    let write = workspace::update_settings(&db, WS, SettingsPatch::default())
        .await
        .expect("update");
    assert_eq!(write.workspace.id, WS);
    assert_eq!(write.audit, None);
    assert_eq!(
        scalar_i64(
            &db,
            r#"SELECT "updatedAt" FROM "Workspace" WHERE "id" = ?1"#,
            WS
        )
        .await,
        before
    );
}

#[tokio::test]
async fn writing_the_target_country_refreshes_every_pipeline_and_leaving_it_out_does_not() {
    // Adapted: the notify call is observed through the returned
    // `currency_changes` of a workspace with one pipeline item.
    let db = seeded().await;
    let pipeline_rows = setup(&db, |tx| {
        Ok(tx.execute(r#"INSERT INTO "Pipeline" ("id", "workspaceId", "name", "type", "referenceName", "settings") VALUES (31, ?1, 'P', 'custom', 'p', '{}')"#, [WS])?)
    })
    .await;
    assert_eq!(pipeline_rows, 1);
    for country in ["JP", "VN"] {
        let write = workspace::update_settings(
            &db,
            WS,
            SettingsPatch {
                target_country: Some(country.into()),
                ..SettingsPatch::default()
            },
        )
        .await
        .expect("update");
        assert_eq!(write.workspace.target_country.as_deref(), Some(country));
        // Every write notifies (no change check against a stale read).
        assert_eq!(write.currency_changes.len(), 1);
        assert_eq!(write.currency_changes[0].pipeline_id, 31);
    }
    let write = workspace::update_settings(
        &db,
        WS,
        SettingsPatch {
            capi_limited_data_use: Some(true),
            ..SettingsPatch::default()
        },
    )
    .await
    .expect("update");
    assert!(write.currency_changes.is_empty());
}

#[tokio::test]
async fn a_failed_settings_write_notifies_nobody() {
    let db = seeded().await;
    // Adapted: a missing workspace is the failed write (no purge fence on settings).
    let result = workspace::update_settings(
        &db,
        999,
        SettingsPatch {
            target_country: Some("VN".into()),
            ..SettingsPatch::default()
        },
    )
    .await;
    assert!(result.is_err());
}

#[tokio::test]
async fn a_field_left_out_is_not_written() {
    let db = seeded().await;
    setup(&db, |tx| {
        tx.execute(
            r#"UPDATE "Workspace" SET "smartResponseDelaySeconds" = 30 WHERE "id" = ?1"#,
            [WS],
        )?;
        Ok(())
    })
    .await;
    let write = workspace::update_settings(
        &db,
        WS,
        SettingsPatch {
            capi_limited_data_use: Some(false),
            ..SettingsPatch::default()
        },
    )
    .await
    .expect("update");
    assert_eq!(write.workspace.smart_response_delay_seconds, Some(30));
    assert!(!write.workspace.capi_limited_data_use);
}

// --- update: audit detail ---

#[tokio::test]
async fn update_audits_a_name_change_and_a_configuration_change() {
    use chatbotx_db::rusqlite::types::Value as Sql;
    let db = seeded().await;
    let renamed = workspace::update(&db, WS, vec![("name", Sql::Text("New".into()))])
        .await
        .expect("rename");
    assert_eq!(renamed.audit.as_deref(), Some("changed the workspace name"));
    let config = workspace::update(&db, WS, vec![("brandColor", Sql::Text("#000000".into()))])
        .await
        .expect("config");
    assert_eq!(
        config.audit.as_deref(),
        Some("updated the workspace configuration")
    );
}

// --- setLogoIfEmpty ---

#[tokio::test]
async fn set_logo_if_empty_stores_the_logo_once() {
    let db = seeded().await;
    let logo = "public/space/21/logos/logo.jpg".to_string();
    assert!(
        workspace::set_logo_if_empty(&db, WS, logo.clone())
            .await
            .expect("first")
    );
    assert!(
        !workspace::set_logo_if_empty(&db, WS, "other".into())
            .await
            .expect("second")
    );
    let stored = db
        .read(|conn| {
            Ok(conn.query_row(
                r#"SELECT "logo" FROM "Workspace" WHERE "id" = ?1"#,
                [WS],
                |r| r.get::<_, String>(0),
            )?)
        })
        .await
        .expect("logo");
    assert_eq!(stored, logo);
}

// --- create ---

async fn owner_only() -> common::TestDb {
    let db = fresh_db().await;
    add_user(&db, OWNER).await;
    db
}

fn create_input() -> CreateWorkspace {
    CreateWorkspace {
        name: "WS".into(),
        owner_id: None,
        tenant_id: None,
        created_by: OWNER,
        members: Vec::new(),
    }
}

#[tokio::test]
async fn create_consumes_a_workspace_seat_and_creates_the_owner_member() {
    let db = owner_only().await;
    setup(&db, |tx| {
        tx.execute(
            r#"INSERT INTO "UserQuota" ("id", "userId", "workspacesLimit", "teamMembersUsed") VALUES (1, ?1, 5, 0)"#,
            [OWNER],
        )?;
        Ok(())
    })
    .await;
    let first = workspace::create(&db, Edition::Cloud, create_input())
        .await
        .expect("create");
    workspace::create(&db, Edition::Cloud, create_input())
        .await
        .expect("second");
    assert_eq!(first.owner_id, OWNER);
    assert_eq!(first.tenant_id, 1);
    assert_eq!(
        scalar_i64(
            &db,
            r#"SELECT "workspacesUsed" FROM "UserQuota" WHERE "userId" = ?1"#,
            OWNER
        )
        .await,
        Some(2)
    );
    // Only a workspace seat: the user-level team-member counter stays.
    assert_eq!(
        scalar_i64(
            &db,
            r#"SELECT "teamMembersUsed" FROM "UserQuota" WHERE "userId" = ?1"#,
            OWNER
        )
        .await,
        Some(0)
    );
    let role = db
        .read(move |conn| {
            Ok(conn.query_row(
                r#"SELECT "role" FROM "WorkspaceMember" WHERE "workspaceId" = ?1 AND "userId" = ?2"#,
                params![first.id, OWNER],
                |r| r.get::<_, String>(0),
            )?)
        })
        .await
        .expect("member");
    assert_eq!(role, "owner");
}

#[tokio::test]
async fn create_throws_workspace_limit_reached_when_the_quota_is_exhausted() {
    let db = owner_only().await;
    setup(&db, |tx| {
        tx.execute(
            r#"INSERT INTO "UserQuota" ("id", "userId", "workspacesLimit", "workspacesUsed") VALUES (1, ?1, 1, 1)"#,
            [OWNER],
        )?;
        Ok(())
    })
    .await;
    let error = workspace::create(&db, Edition::Cloud, create_input())
        .await
        .expect_err("limit");
    assert_eq!(error.code(), Some("workspaceLimitReached"));
    assert_eq!(
        scalar_i64(
            &db,
            r#"SELECT COUNT(*) FROM "Workspace" WHERE "ownerId" = ?1"#,
            OWNER
        )
        .await,
        Some(0)
    );
}

#[tokio::test]
async fn create_inserts_an_agent_member_for_each_extra_member() {
    let db = owner_only().await;
    add_user(&db, 12).await;
    add_user(&db, 13).await;
    let permissions = json!({ "superAdmin": false, "analytics": true });
    let mut input = create_input();
    input.members = vec![
        ExtraMember {
            user_id: 12,
            permissions: permissions.clone(),
        },
        ExtraMember {
            user_id: 13,
            permissions: permissions.clone(),
        },
    ];
    let created = workspace::create(&db, Edition::Cloud, input)
        .await
        .expect("create");
    let rows = db
        .read(move |conn| {
            let mut stmt = conn.prepare(
                r#"SELECT "userId", "role", "permissions" FROM "WorkspaceMember" WHERE "workspaceId" = ?1 ORDER BY "id""#,
            )?;
            let rows = stmt
                .query_map([created.id], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?)))?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        })
        .await
        .expect("members");
    assert_eq!(rows.len(), 3);
    assert_eq!((rows[0].0, rows[0].1.as_str()), (OWNER, "owner"));
    for (row, user) in rows[1..].iter().zip([12, 13]) {
        assert_eq!((row.0, row.1.as_str()), (user, "agent"));
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&row.2).unwrap(),
            permissions
        );
    }
    assert_eq!(
        scalar_i64(
            &db,
            r#"SELECT "teamMembersUsed" FROM "WorkspaceUsage" WHERE "workspaceId" = ?1"#,
            created.id
        )
        .await,
        Some(3)
    );
}

#[tokio::test]
async fn community_creates_the_first_workspace_and_refuses_the_second() {
    let db = owner_only().await;
    workspace::create(&db, Edition::Community, create_input())
        .await
        .expect("first");
    let error = workspace::create(&db, Edition::Community, create_input())
        .await
        .expect_err("second");
    assert_eq!(error.code(), Some("workspaceLimitReached"));
    assert_eq!(
        scalar_i64(
            &db,
            r#"SELECT COUNT(*) FROM "Workspace" WHERE "ownerId" = ?1"#,
            OWNER
        )
        .await,
        Some(1)
    );
}

#[tokio::test]
async fn community_limit_counts_the_owner_when_it_differs_from_the_creator() {
    let db = owner_only().await;
    add_user(&db, 19).await;
    let mut input = create_input();
    input.owner_id = Some(19);
    workspace::create(&db, Edition::Community, input)
        .await
        .expect("first for 19");
    // The creator (11) owns nothing yet, so it can still create its own.
    workspace::create(&db, Edition::Community, create_input())
        .await
        .expect("first for 11");
    let mut again = create_input();
    again.owner_id = Some(19);
    let error = workspace::create(&db, Edition::Community, again)
        .await
        .expect_err("second for 19");
    assert_eq!(error.code(), Some("workspaceLimitReached"));
}

#[tokio::test]
async fn the_limit_is_skipped_off_community() {
    let db = owner_only().await;
    workspace::create(&db, Edition::Cloud, create_input())
        .await
        .expect("first");
    workspace::create(&db, Edition::Cloud, create_input())
        .await
        .expect("second");
    assert_eq!(
        scalar_i64(
            &db,
            r#"SELECT COUNT(*) FROM "Workspace" WHERE "ownerId" = ?1"#,
            OWNER
        )
        .await,
        Some(2)
    );
}
