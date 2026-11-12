//! Port of `packages/database/__tests__/ai-handover-settings-repository.test.ts` (P3.2).
//! The mocked Drizzle chains become real rows of a migrated SQLite file (adapted: the
//! asserted call shapes are checked as their effect on the stored row).

mod common;
#[macro_use]
#[path = "common/repo.rs"]
mod repo_common;

use chatbotx_db::repo::ai_handover_settings::{
    self as settings, SetApplyToAllInput, UpsertAiHandoverSettingsInput,
};
use chatbotx_db::repo::{self, Json};
use repo_common::handover::{INBOX, WS, exec, page};
use repo_common::{err_message, insert_flow, int, ok};
use serde_json::json;

fn upsert_input(goto: Option<i64>, message: Option<&str>) -> UpsertAiHandoverSettingsInput {
    UpsertAiHandoverSettingsInput {
        workspace_id: WS,
        inbox_id: INBOX,
        channel: "messenger".into(),
        enabled: true,
        schedule_enabled: true,
        time_ranges: Json(json!([{ "from": 8, "to": 17 }])),
        goto_flow_id: goto,
        return_message: message.map(str::to_owned),
        pause_bot_waiting_for_staff: true,
    }
}

async fn page_with_flow() -> common::TestDb {
    let db = page().await;
    repo_common::setup(&db, |tx| {
        insert_flow(tx, 600, WS)?;
        Ok(())
    })
    .await;
    db
}

/// Port of: packages/database/__tests__/ai-handover-settings-repository.test.ts:48 "aiHandoverSettingsRepository.upsert > writes every setting explicitly, with the apply-to-all state off, and replaces only the settings on conflict"
#[tokio::test]
async fn ai_handover_settings_repository_upsert_writes_every_setting_explicitly_with_the_apply_to_all_state_off_and_replaces_only_the_settings_on_conflict()
 {
    let db = page_with_flow().await;
    let row = ok(repo::write(&db, |tx| {
        settings::upsert(tx, &upsert_input(Some(600), Some("Back with you")))
    })
    .await);
    assert_eq!(
        (row.workspace_id, row.inbox_id, row.channel.as_str()),
        (WS, INBOX, "messenger")
    );
    assert!(row.enabled && row.schedule_enabled && row.pause_bot_waiting_for_staff);
    assert_eq!(row.time_ranges, Json(json!([{ "from": 8, "to": 17 }])));
    assert_eq!(row.goto_flow_id, Some(600));
    assert_eq!(row.return_message.as_deref(), Some("Back with you"));
    assert!(!row.apply_to_all_customers);
    assert_eq!(row.apply_to_all_revision, 0);
    assert_eq!(
        (
            row.apply_to_all_message.clone(),
            row.apply_to_all_requested_by_user_id
        ),
        (None, None)
    );
    // On conflict only the settings are replaced: the apply-to-all state survives.
    exec(&db, r#"UPDATE "AIHandoverSettings" SET "applyToAllRevision" = 4, "applyToAllCustomers" = 1 WHERE "id" = ?1"#, vec![int(row.id)]).await;
    let mut next = upsert_input(None, None);
    next.enabled = false;
    let again = ok(repo::write(&db, move |tx| settings::upsert(tx, &next)).await);
    assert_eq!(again.id, row.id);
    assert!(!again.enabled);
    assert_eq!(
        (again.apply_to_all_revision, again.apply_to_all_customers),
        (4, true)
    );
}

/// Port of: packages/database/__tests__/ai-handover-settings-repository.test.ts:76 "aiHandoverSettingsRepository.upsert > can clear the optional settings"
#[tokio::test]
async fn ai_handover_settings_repository_upsert_can_clear_the_optional_settings() {
    let db = page_with_flow().await;
    ok(repo::write(&db, |tx| {
        settings::upsert(tx, &upsert_input(Some(600), Some("x")))
    })
    .await);
    let row = ok(repo::write(&db, |tx| settings::upsert(tx, &upsert_input(None, None))).await);
    assert_eq!((row.goto_flow_id, row.return_message), (None, None));
}

// not-applicable: packages/database/__tests__/ai-handover-settings-repository.test.ts:96 "aiHandoverSettingsRepository.upsert > throws when the write returns no row" — an INSERT ... ON CONFLICT DO UPDATE ... RETURNING always returns the row on SQLite; only a mocked client returns none (the error branch is kept in the code).

/// Port of: packages/database/__tests__/ai-handover-settings-repository.test.ts:108 "aiHandoverSettingsRepository.findByInbox > returns the Page's row, scoped by workspace AND inbox"
#[tokio::test]
async fn ai_handover_settings_repository_find_by_inbox_returns_the_page_s_row_scoped_by_workspace_and_inbox()
 {
    let db = page_with_flow().await;
    let row = ok(repo::write(&db, |tx| settings::upsert(tx, &upsert_input(None, None))).await);
    let found = ok(repo::read(&db, |c| settings::find_by_inbox(c, WS, INBOX)).await);
    assert_eq!(found.map(|r| r.id), Some(row.id));
    let other = ok(repo::read(&db, |c| settings::find_by_inbox(c, 9999, INBOX)).await);
    assert!(other.is_none());
}

/// Port of: packages/database/__tests__/ai-handover-settings-repository.test.ts:125 "aiHandoverSettingsRepository.findByInbox > returns null when nothing was saved"
#[tokio::test]
async fn ai_handover_settings_repository_find_by_inbox_returns_null_when_nothing_was_saved() {
    let db = page().await;
    assert!(ok(repo::read(&db, |c| settings::find_by_inbox(c, WS, INBOX)).await).is_none());
}

/// Port of: packages/database/__tests__/ai-handover-settings-repository.test.ts:166 "aiHandoverSettingsRepository.lockExisting > lockExisting locks the settings row FOR UPDATE alone, then reads the Page's status in a statement of its own, and never creates a row"
#[tokio::test]
async fn ai_handover_settings_repository_lock_existing_lock_existing_locks_the_settings_row_for_update_alone_then_reads_the_page_s_status_in_a_statement_of_its_own_and_never_creates_a_row()
 {
    let db = page_with_flow().await;
    let row = ok(repo::write(&db, |tx| settings::upsert(tx, &upsert_input(None, None))).await);
    let locked =
        ok(repo::write(&db, |tx| settings::lock_existing(tx, WS, INBOX)).await).expect("locked");
    assert_eq!(locked.settings.id, row.id);
    assert_eq!(locked.inbox_status, "connected");
    let count =
        repo_common::scalar_i64(&db, r#"SELECT count(*) FROM "AIHandoverSettings""#, vec![]).await;
    assert_eq!(count, Some(1));
}

/// Port of: packages/database/__tests__/ai-handover-settings-repository.test.ts:188 "aiHandoverSettingsRepository.lockExisting > lockExisting returns null for a Page without settings, without reading the status"
#[tokio::test]
async fn ai_handover_settings_repository_lock_existing_lock_existing_returns_null_for_a_page_without_settings_without_reading_the_status()
 {
    let db = page().await;
    assert!(ok(repo::write(&db, |tx| settings::lock_existing(tx, WS, INBOX)).await).is_none());
    let count =
        repo_common::scalar_i64(&db, r#"SELECT count(*) FROM "AIHandoverSettings""#, vec![]).await;
    assert_eq!(count, Some(0));
}

// not-applicable: packages/database/__tests__/ai-handover-settings-repository.test.ts:197 "aiHandoverSettingsRepository.lockExisting > a Page whose inbox row is gone counts as disconnected" — with SQLite FKs on (always), deleting the Inbox cascades to its AIHandoverSettings row, so a settings row without its inbox cannot exist (PRAGMA foreign_keys is a no-op inside the write transaction); only a mocked client returns that state. The `disconnected` fallback is kept in the code.

/// Port of: packages/database/__tests__/ai-handover-settings-repository.test.ts:233 "aiHandoverSettingsRepository.lockForApplyToAll > creates a switched-off row when the Page has none, then locks it FOR UPDATE"
#[tokio::test]
async fn ai_handover_settings_repository_lock_for_apply_to_all_creates_a_switched_off_row_when_the_page_has_none_then_locks_it_for_update()
 {
    let db = page().await;
    let locked = ok(repo::write(&db, |tx| {
        settings::lock_for_apply_to_all(tx, WS, INBOX, "messenger")
    })
    .await);
    assert_eq!(locked.inbox_status, "connected");
    let s = locked.settings;
    assert_eq!((s.workspace_id, s.inbox_id), (WS, INBOX));
    assert!(!s.enabled && !s.apply_to_all_customers);
    assert_eq!(s.apply_to_all_revision, 0);
    // Second call: no new row (ON CONFLICT (inboxId) DO NOTHING).
    let again = ok(repo::write(&db, |tx| {
        settings::lock_for_apply_to_all(tx, WS, INBOX, "messenger")
    })
    .await);
    assert_eq!(again.settings.id, s.id);
}

/// Port of: packages/database/__tests__/ai-handover-settings-repository.test.ts:261 "aiHandoverSettingsRepository.lockForApplyToAll > throws when the row cannot be read back"
///
/// Real-row form: the inbox already has a row of another workspace, so the insert is a
/// no-op and the workspace-scoped read finds nothing.
#[tokio::test]
async fn ai_handover_settings_repository_lock_for_apply_to_all_throws_when_the_row_cannot_be_read_back()
 {
    let db = page().await;
    common::seed_workspace(&db, 1002, 2002).await;
    repo_common::setup(&db, |tx| {
        ins!(tx, "AIHandoverSettings", { "id" => 1_i64, "workspaceId" => 2002_i64, "inboxId" => INBOX, "channel" => "messenger", "timeRanges" => "[]" })?;
        Ok(())
    })
    .await;
    let message = err_message(
        repo::write(&db, |tx| {
            settings::lock_for_apply_to_all(tx, WS, INBOX, "messenger")
        })
        .await,
    );
    assert!(
        message.contains("Failed to lock business AI settings"),
        "{message}"
    );
}

fn set(customers: bool, message: Option<&str>, user: Option<i64>) -> SetApplyToAllInput {
    SetApplyToAllInput {
        workspace_id: WS,
        inbox_id: INBOX,
        apply_to_all_customers: customers,
        apply_to_all_message: message.map(str::to_owned),
        requested_by_user_id: user,
    }
}

/// Port of: packages/database/__tests__/ai-handover-settings-repository.test.ts:286 "aiHandoverSettingsRepository.setApplyToAll > records the new state with the next revision, who asked and the message"
#[tokio::test]
async fn ai_handover_settings_repository_set_apply_to_all_records_the_new_state_with_the_next_revision_who_asked_and_the_message()
 {
    let db = page().await;
    ok(repo::write(&db, |tx| {
        settings::lock_for_apply_to_all(tx, WS, INBOX, "messenger")
    })
    .await);
    exec(
        &db,
        r#"UPDATE "AIHandoverSettings" SET "applyToAllRevision" = 4"#,
        vec![],
    )
    .await;
    let row = ok(repo::write(&db, |tx| {
        settings::set_apply_to_all(tx, &set(false, Some("A person is here"), Some(1001)))
    })
    .await);
    assert!(!row.apply_to_all_customers);
    assert_eq!(row.apply_to_all_revision, 5);
    assert_eq!(
        row.apply_to_all_message.as_deref(),
        Some("A person is here")
    );
    assert_eq!(row.apply_to_all_requested_by_user_id, Some(1001));
}

/// Port of: packages/database/__tests__/ai-handover-settings-repository.test.ts:310 "aiHandoverSettingsRepository.setApplyToAll > the first change of a Page is revision 1"
#[tokio::test]
async fn ai_handover_settings_repository_set_apply_to_all_the_first_change_of_a_page_is_revision_1()
{
    let db = page().await;
    ok(repo::write(&db, |tx| {
        settings::lock_for_apply_to_all(tx, WS, INBOX, "messenger")
    })
    .await);
    let row = ok(repo::write(&db, |tx| {
        settings::set_apply_to_all(tx, &set(true, None, None))
    })
    .await);
    assert_eq!(row.apply_to_all_revision, 1);
}

/// Port of: packages/database/__tests__/ai-handover-settings-repository.test.ts:326 "aiHandoverSettingsRepository.setApplyToAll > throws when no row was updated"
#[tokio::test]
async fn ai_handover_settings_repository_set_apply_to_all_throws_when_no_row_was_updated() {
    let db = page().await;
    let message = err_message(
        repo::write(&db, |tx| {
            settings::set_apply_to_all(tx, &set(true, None, None))
        })
        .await,
    );
    assert!(
        message.contains("Failed to record the apply-to-all change"),
        "{message}"
    );
}
