//! Port of `packages/database/__tests__/broadcast-repository.test.ts` (P3.2).
//! The mocked `findMany` / `$count` calls become real Broadcast rows of a migrated SQLite
//! file; "the where clause has X" is ported as "rows outside X are not returned" and "the
//! `with` loads Y" as "the relation Y is filled" (adapted).

mod common;
#[macro_use]
#[path = "common/repo.rs"]
mod repo_common;

use chatbotx_db::Db;
use chatbotx_db::repo::broadcast::{self as b, BroadcastListInput};
use chatbotx_db::repo::utils::SortItem;
use chatbotx_db::repo::{self};
use chatbotx_db::rusqlite;
use common::{fresh_db, seed};
use repo_common::bcast::{WS, insert, recipients};
use repo_common::{insert_flow, ok};

async fn db() -> common::TestDb {
    let db = fresh_db().await;
    seed(&db).await;
    db
}

fn input() -> BroadcastListInput {
    BroadcastListInput {
        workspace_id: WS,
        ..Default::default()
    }
}

async fn list_ids(db: &Db, i: BroadcastListInput) -> Vec<i64> {
    ok(repo::read(db, move |c| b::list_with_relations(c, &i)).await)
        .into_iter()
        .map(|r| r.broadcast.id)
        .collect()
}

async fn exec(db: &Db, sql: &'static str) {
    repo_common::handover::exec(db, sql, vec![]).await;
}

/// Port of: packages/database/__tests__/broadcast-repository.test.ts:67 "broadcastRepository.listWithRelations > scopes to the workspace and excludes soft-deleted rows"
#[tokio::test]
async fn broadcast_repository_list_with_relations_scopes_to_the_workspace_and_excludes_soft_deleted_rows()
 {
    let db = db().await;
    common::seed_workspace(&db, 1002, 2002).await;
    repo_common::setup(&db, |tx| {
        insert_flow(tx, 600, WS)?;
        ins!(tx, "IntegrationWhatsapp", { "id" => 6001_i64, "auth" => "{}", "phoneNumberId" => "pn", "wabaId" => "w", "businessId" => "b", "name" => "WA", "workspaceId" => WS, "inboxId" => 3001_i64 })?;
        insert(tx, 1, "mine", "draft")?;
        insert(tx, 2, "deleted", "draft")?;
        insert(tx, 3, "other ws", "draft")?;
        tx.execute(r#"UPDATE "Broadcast" SET "deletedAt" = 5 WHERE "id" = 2"#, [])?;
        tx.execute(r#"UPDATE "Broadcast" SET "workspaceId" = 2002 WHERE "id" = 3"#, [])?;
        tx.execute(r#"UPDATE "Broadcast" SET "flowId" = 600, "integrationWhatsappId" = 6001 WHERE "id" = 1"#, [])?;
        Ok(())
    })
    .await;
    let rows = ok(repo::read(&db, |c| b::list_with_relations(c, &input())).await);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].broadcast.id, 1);
    assert_eq!(
        rows[0].flow.as_ref().map(|f| f.name.as_str()),
        Some("Flow 600")
    );
    assert_eq!(
        rows[0]
            .integration_whatsapp
            .as_ref()
            .map(|f| f.name.as_str()),
        Some("WA")
    );
}

/// Port of: packages/database/__tests__/broadcast-repository.test.ts:88 "broadcastRepository.listWithRelations > loads every target page with its page and flow names"
#[tokio::test]
async fn broadcast_repository_list_with_relations_loads_every_target_page_with_its_page_and_flow_names()
 {
    let db = db().await;
    repo_common::setup(&db, |tx| {
        insert_flow(tx, 600, WS)?;
        repo_common::insert_inbox(tx, 3002, WS, "messenger", "page-2")?;
        insert(tx, 1, "multi", "draft")?;
        ins!(tx, "BroadcastTarget", { "broadcastId" => 1_i64, "inboxId" => 3001_i64, "flowId" => 600_i64 })?;
        ins!(tx, "BroadcastTarget", { "broadcastId" => 1_i64, "inboxId" => 3002_i64 })?;
        Ok(())
    })
    .await;
    let rows = ok(repo::read(&db, |c| b::list_with_relations(c, &input())).await);
    let targets = &rows[0].targets;
    assert_eq!(targets.len(), 2);
    assert_eq!(
        targets[0].inbox.as_ref().map(|i| (i.id, i.name.as_str())),
        Some((3001, "Inbox"))
    );
    assert_eq!(
        targets[0].flow.as_ref().map(|f| f.name.as_str()),
        Some("Flow 600")
    );
    assert_eq!(
        targets[1].inbox.as_ref().map(|i| i.name.as_str()),
        Some("Inbox 3002")
    );
    assert!(targets[1].flow.is_none());
}

async fn statuses() -> common::TestDb {
    let db = db().await;
    repo_common::setup(&db, |tx| {
        insert(tx, 1, "a", "failed")?;
        insert(tx, 2, "b", "sent")?;
        Ok(())
    })
    .await;
    db
}

/// Port of: packages/database/__tests__/broadcast-repository.test.ts:104 "broadcastRepository.listWithRelations > passes the status filter through to the where clause"
#[tokio::test]
async fn broadcast_repository_list_with_relations_passes_the_status_filter_through_to_the_where_clause()
 {
    let db = statuses().await;
    let mut i = input();
    i.status = Some("failed".into());
    assert_eq!(list_ids(&db, i).await, vec![1]);
}

/// Port of: packages/database/__tests__/broadcast-repository.test.ts:118 "broadcastRepository.listWithRelations > omits status from the where clause when it is null"
#[tokio::test]
async fn broadcast_repository_list_with_relations_omits_status_from_the_where_clause_when_it_is_null()
 {
    let db = statuses().await;
    let mut i = input();
    i.status = None;
    assert_eq!(list_ids(&db, i).await, vec![2, 1]);
}

/// Template 1 (recurring, no occurrence), its run 2 (occurrenceAt), a plain broadcast 3.
async fn recurring() -> common::TestDb {
    let db = db().await;
    repo_common::setup(&db, |tx| {
        insert(tx, 1, "template", "recurring")?;
        insert(tx, 2, "run", "sent")?;
        insert(tx, 3, "plain", "sent")?;
        tx.execute(r#"UPDATE "Broadcast" SET "schedulesType" = 'recurring' WHERE "id" = 1"#, [])?;
        tx.execute(r#"UPDATE "Broadcast" SET "parentBroadcastId" = 1, "occurrenceAt" = 50, "schedulesType" = 'recurring' WHERE "id" = 2"#, [])?;
        Ok(())
    })
    .await;
    db
}

/// Port of: packages/database/__tests__/broadcast-repository.test.ts:148 "broadcastRepository recurrence filters > shows runs and templates by default, so existing callers see no change"
#[tokio::test]
async fn broadcast_repository_recurrence_filters_shows_runs_and_templates_by_default_so_existing_callers_see_no_change()
 {
    let db = recurring().await;
    assert_eq!(list_ids(&db, input()).await, vec![3, 2, 1]);
}

/// Port of: packages/database/__tests__/broadcast-repository.test.ts:156 "broadcastRepository recurrence filters > includeRuns=false hides runs through occurrenceAt, which outlives a purged template"
#[tokio::test]
async fn broadcast_repository_recurrence_filters_include_runs_false_hides_runs_through_occurrence_at_which_outlives_a_purged_template()
 {
    let db = recurring().await;
    // The template is purged: the run keeps its occurrenceAt (parent SET NULL).
    exec(&db, r#"DELETE FROM "Broadcast" WHERE "id" = 1"#).await;
    let mut i = input();
    i.include_runs = Some(false);
    assert_eq!(list_ids(&db, i).await, vec![3]);
}

/// Port of: packages/database/__tests__/broadcast-repository.test.ts:161 "broadcastRepository recurrence filters > excludeTemplates keeps everything except a recurring broadcast without an occurrence"
#[tokio::test]
async fn broadcast_repository_recurrence_filters_exclude_templates_keeps_everything_except_a_recurring_broadcast_without_an_occurrence()
 {
    let db = recurring().await;
    let mut i = input();
    i.exclude_templates = Some(true);
    assert_eq!(list_ids(&db, i).await, vec![3, 2]);
}

/// Port of: packages/database/__tests__/broadcast-repository.test.ts:169 "broadcastRepository recurrence filters > parentBroadcastId and schedulesType narrow the list"
#[tokio::test]
async fn broadcast_repository_recurrence_filters_parent_broadcast_id_and_schedules_type_narrow_the_list()
 {
    let db = recurring().await;
    let mut i = input();
    i.parent_broadcast_id = Some(1);
    i.schedules_type = Some("recurring".into());
    assert_eq!(list_ids(&db, i).await, vec![2]);
}

/// Port of: packages/database/__tests__/broadcast-repository.test.ts:179 "broadcastRepository recurrence filters > loads the schedule summary and the parent name without extra queries"
#[tokio::test]
async fn broadcast_repository_recurrence_filters_loads_the_schedule_summary_and_the_parent_name_without_extra_queries()
 {
    let db = recurring().await;
    repo_common::setup(&db, |tx| {
        ins!(tx, "BroadcastRecurrence", { "id" => 9_i64, "workspaceId" => WS, "broadcastId" => 1_i64, "rule" => r#"{"freq":"daily"}"#, "nextRunAt" => 99_i64, "occurrenceCount" => 4_i64 })?;
        Ok(())
    })
    .await;
    let rows = ok(repo::read(&db, |c| b::list_with_relations(c, &input())).await);
    let template = rows.iter().find(|r| r.broadcast.id == 1).expect("template");
    let rec = template.recurrence.as_ref().expect("recurrence");
    assert_eq!(
        (rec.id, rec.next_run_at, rec.occurrence_count),
        (9, Some(99), 4)
    );
    assert_eq!(rec.rule.0["freq"], "daily");
    let run = rows.iter().find(|r| r.broadcast.id == 2).expect("run");
    assert_eq!(
        run.parent.as_ref().map(|p| (p.id, p.name.as_str())),
        Some((1, "template"))
    );
}

/// Port of: packages/database/__tests__/broadcast-repository.test.ts:206 "broadcastRepository list filters > narrows by channel and schedule window"
#[tokio::test]
async fn broadcast_repository_list_filters_narrows_by_channel_and_schedule_window() {
    let db = db().await;
    let from = repo_common::ms("2026-10-01T00:00:00Z");
    let to = repo_common::ms("2026-10-31T00:00:00Z");
    repo_common::setup(&db, move |tx| {
        for (id, channel, at) in [
            (1_i64, "whatsapp", from + 1),
            (2, "whatsapp", to + 1),
            (3, "messenger", from + 1),
            (4, "whatsapp", to),
        ] {
            insert(tx, id, "b", "sent")?;
            tx.execute(
                r#"UPDATE "Broadcast" SET "channel" = ?1, "schedulesAt" = ?2 WHERE "id" = ?3"#,
                rusqlite::params![channel, at, id],
            )?;
        }
        Ok(())
    })
    .await;
    let mut i = input();
    i.channel = Some("whatsapp".into());
    i.scheduled_from = Some(from);
    i.scheduled_to = Some(to);
    assert_eq!(list_ids(&db, i).await, vec![4, 1]);
}

/// Port of: packages/database/__tests__/broadcast-repository.test.ts:225 "broadcastRepository list filters > adds no schedule or channel condition when none is given"
#[tokio::test]
async fn broadcast_repository_list_filters_adds_no_schedule_or_channel_condition_when_none_is_given()
 {
    let db = db().await;
    repo_common::setup(&db, |tx| {
        insert(tx, 1, "a", "sent")?;
        insert(tx, 2, "b", "sent")?;
        tx.execute(
            r#"UPDATE "Broadcast" SET "channel" = 'whatsapp', "schedulesAt" = 0 WHERE "id" = 2"#,
            [],
        )?;
        Ok(())
    })
    .await;
    assert_eq!(list_ids(&db, input()).await, vec![2, 1]);
}

/// Port of: packages/database/__tests__/broadcast-repository.test.ts:237 "broadcastRepository list filters > an empty or unknown sort still orders by newest first"
#[tokio::test]
async fn broadcast_repository_list_filters_an_empty_or_unknown_sort_still_orders_by_newest_first() {
    let db = statuses().await;
    let mut i = input();
    i.sort = Some(Vec::new());
    assert_eq!(list_ids(&db, i.clone()).await, vec![2, 1]);
    i.sort = Some(vec![SortItem {
        id: "unknownColumn".into(),
        desc: false,
    }]);
    assert_eq!(list_ids(&db, i).await, vec![2, 1]);
}

/// Port of: packages/database/__tests__/broadcast-repository.test.ts:249 "broadcastRepository list filters > get-by-id loads the target pages too"
#[tokio::test]
async fn broadcast_repository_list_filters_get_by_id_loads_the_target_pages_too() {
    let db = db().await;
    repo_common::setup(&db, |tx| {
        insert(tx, 7, "seven", "draft")?;
        ins!(tx, "BroadcastTarget", { "broadcastId" => 7_i64, "inboxId" => 3001_i64 })?;
        Ok(())
    })
    .await;
    let found = ok(repo::read(&db, |c| b::find_by_id_or_name(c, WS, "7")).await).expect("found");
    assert_eq!(found.targets.len(), 1);
}

async fn with_audience() -> common::TestDb {
    let db = db().await;
    repo_common::setup(&db, |tx| {
        repo_common::insert_inbox(tx, 3100, WS, "webchat", "wc")?;
        insert(tx, 1, "a", "sent")?;
        insert(tx, 2, "b", "sent")?;
        recipients(tx, 1, 3100, 100, 3)?;
        recipients(tx, 2, 3100, 500, 1)?;
        Ok(())
    })
    .await;
    db
}

/// Port of: packages/database/__tests__/broadcast-repository.test.ts:267 "broadcastRepository.listAudience / countAudience > lists audience rows scoped to the broadcast id"
#[tokio::test]
async fn broadcast_repository_list_audience_count_audience_lists_audience_rows_scoped_to_the_broadcast_id()
 {
    let db = with_audience().await;
    let rows = ok(repo::read(&db, |c| b::list_audience(c, 2, 20, 0)).await);
    assert_eq!(rows.len(), 1);
    assert_eq!((rows[0].row.contact_id, rows[0].contact.id), (500, 500));
}

/// Port of: packages/database/__tests__/broadcast-repository.test.ts:285 "broadcastRepository.listAudience / countAudience > counts audience rows scoped to the broadcast id"
#[tokio::test]
async fn broadcast_repository_list_audience_count_audience_counts_audience_rows_scoped_to_the_broadcast_id()
 {
    let db = with_audience().await;
    assert_eq!(ok(repo::read(&db, |c| b::count_audience(c, 1)).await), 3);
}

async fn active() -> common::TestDb {
    let db = db().await;
    common::seed_workspace(&db, 1002, 2002).await;
    repo_common::setup(&db, |tx| {
        for (id, status) in [
            (1_i64, "scheduled"),
            (2, "sending"),
            (3, "sent"),
            (4, "scheduled"),
            (5, "scheduled"),
            (6, "scheduled"),
        ] {
            insert(tx, id, "b", status)?;
            tx.execute(
                r#"UPDATE "Broadcast" SET "channel" = 'messenger' WHERE "id" = ?1"#,
                [id],
            )?;
        }
        tx.execute(
            r#"UPDATE "Broadcast" SET "deletedAt" = 1 WHERE "id" = 4"#,
            [],
        )?;
        tx.execute(
            r#"UPDATE "Broadcast" SET "channel" = 'whatsapp' WHERE "id" = 5"#,
            [],
        )?;
        tx.execute(
            r#"UPDATE "Broadcast" SET "workspaceId" = 2002 WHERE "id" = 6"#,
            [],
        )?;
        Ok(())
    })
    .await;
    db
}

/// Port of: packages/database/__tests__/broadcast-repository.test.ts:300 "broadcastRepository.countActive > counts active rows by workspace, channel, statuses, and non-deleted state"
#[tokio::test]
async fn broadcast_repository_count_active_counts_active_rows_by_workspace_channel_statuses_and_non_deleted_state()
 {
    let db = active().await;
    let n = ok(repo::read(&db, |c| {
        b::count_active(c, WS, "messenger", &["scheduled", "sending"], None)
    })
    .await);
    assert_eq!(n, 2);
}

/// Port of: packages/database/__tests__/broadcast-repository.test.ts:324 "broadcastRepository.countActive > excludes the supplied broadcast id and uses the passed transaction"
#[tokio::test]
async fn broadcast_repository_count_active_excludes_the_supplied_broadcast_id_and_uses_the_passed_transaction()
 {
    let db = active().await;
    // Inside a write transaction, seeing its own uncommitted row.
    let n = ok(repo::write(&db, |tx| {
        insert(tx, 7, "new", "sending")?;
        tx.execute(
            r#"UPDATE "Broadcast" SET "channel" = 'messenger' WHERE "id" = 7"#,
            [],
        )?;
        b::count_active(tx, WS, "messenger", &["scheduled", "sending"], Some(1))
    })
    .await);
    assert_eq!(n, 2);
}

async fn named() -> common::TestDb {
    let db = db().await;
    repo_common::setup(&db, |tx| {
        insert(tx, 123, "My Broadcast", "draft")?;
        insert(tx, 124, "123", "draft")?;
        Ok(())
    })
    .await;
    db
}

/// Port of: packages/database/__tests__/broadcast-repository.test.ts:352 "broadcastRepository.findByIdOrName > looks up by id when idOrName is numeric"
#[tokio::test]
async fn broadcast_repository_find_by_id_or_name_looks_up_by_id_when_id_or_name_is_numeric() {
    let db = named().await;
    let found = ok(repo::read(&db, |c| b::find_by_id_or_name(c, WS, "123")).await).expect("found");
    assert_eq!(found.broadcast.id, 123);
}

/// Port of: packages/database/__tests__/broadcast-repository.test.ts:367 "broadcastRepository.findByIdOrName > looks up by name when idOrName is not numeric"
#[tokio::test]
async fn broadcast_repository_find_by_id_or_name_looks_up_by_name_when_id_or_name_is_not_numeric() {
    let db = named().await;
    let found =
        ok(repo::read(&db, |c| b::find_by_id_or_name(c, WS, "My Broadcast")).await).expect("found");
    assert_eq!(found.broadcast.id, 123);
}
