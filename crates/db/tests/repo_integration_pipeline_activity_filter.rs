//! Port of `packages/database/__tests__/integration/pipeline-activity-filter.test.ts`
//! (P3.3). The TS runs against Postgres; each port runs against a migrated SQLite file.

mod common;
#[macro_use]
#[path = "common/repo.rs"]
mod repo_common;
#[path = "common/repo7.rs"]
mod repo7;

use std::collections::BTreeMap;

use chatbotx_db::Db;
use chatbotx_db::repo;
use chatbotx_db::repo::models::PipelineItemActivity;
use chatbotx_db::repo::pipeline_activity;
use chatbotx_db::repo::pipeline_item::{self, ListStagePageInput};
use chatbotx_db::repo::pipeline_item_cursor::{PipelineItemSort, PipelineItemSortKey};
use chatbotx_db::repo::pipeline_item_filter::{
    DateRange, PipelineActivityMatch, PipelineItemFilter,
};
use chatbotx_db::repo::utils::SortDir;
use chatbotx_db::rusqlite::{Connection, params};
use repo_common::{ms, ok, setup};
use repo7::*;

const USER: i64 = 1001;
const ANOTHER_USER: i64 = 1002;
const WS: i64 = 2001;
const CONTACT: i64 = 4001;
const PIPELINE: i64 = 5001;
const OTHER_PIPELINE: i64 = 5002;
const STAGE: i64 = 6011;

/// The item fixture: every item goes to `PIPELINE` / `STAGE` / `CONTACT`.
const FX: PipeFx = PipeFx {
    user_id: USER,
    workspace_id: WS,
    pipeline_id: PIPELINE,
    stage_ids: [STAGE, STAGE],
    contact_ids: [CONTACT, CONTACT],
};

fn day(value: &str) -> i64 {
    ms(&format!("{value}T12:00:00.000Z"))
}

struct ActivitySeed {
    kind: &'static str,
    actor_kind: &'static str,
    actor_user_id: Option<i64>,
    at: &'static str,
    /// Record the row under the other pipeline of the workspace.
    elsewhere: bool,
}

const fn act(
    kind: &'static str,
    actor_kind: &'static str,
    actor_user_id: Option<i64>,
    at: &'static str,
) -> ActivitySeed {
    ActivitySeed {
        kind,
        actor_kind,
        actor_user_id,
        at,
        elsewhere: false,
    }
}

#[allow(clippy::too_many_arguments)]
fn put_activity(
    tx: &Connection,
    pipeline_id: i64,
    item_id: i64,
    item_number: i64,
    title: &str,
    contact_id: i64,
    kind: &str,
    actor_kind: &str,
    actor_user_id: Option<i64>,
    occurred_at: i64,
) -> Result<(), chatbotx_db::DbError> {
    tx.execute(
        r#"INSERT INTO "PipelineItemActivity" ("id", "workspaceId", "pipelineId", "itemId", "itemNumber", "itemTitle", "contactId", "kind", "actorKind", "actorUserId", "occurredAt")
           VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)"#,
        params![
            repo::new_id(),
            WS,
            pipeline_id,
            item_id,
            item_number,
            title,
            contact_id,
            kind,
            actor_kind,
            actor_user_id,
            occurred_at
        ],
    )?;
    Ok(())
}

async fn seed_item(db: &Db, title: &'static str, activities: Vec<ActivitySeed>) {
    let item = insert_item(db, FX, move |v| v.title = title.to_owned()).await;
    setup(db, move |tx| {
        for a in &activities {
            put_activity(
                tx,
                if a.elsewhere {
                    OTHER_PIPELINE
                } else {
                    PIPELINE
                },
                item.id,
                item.number,
                title,
                CONTACT,
                a.kind,
                a.actor_kind,
                a.actor_user_id,
                day(a.at),
            )?;
        }
        Ok(())
    })
    .await;
}

/// The TS `beforeAll`: two users, a workspace, a contact, the pipelines `sales` and
/// `support` (one `Lead` stage each) and six items with their activity rows.
async fn activity_db() -> common::TestDb {
    let db = common::fresh_db().await;
    setup(&db, |tx| {
        put_user(tx, USER)?;
        put_user(tx, ANOTHER_USER)?;
        tx.execute(
            r#"INSERT INTO "Workspace" ("id", "name", "ownerId") VALUES (?1, 'activity-filter', ?2)"#,
            params![WS, USER],
        )?;
        tx.execute(
            r#"INSERT INTO "Contact" ("id", "workspaceId") VALUES (?1, ?2)"#,
            params![CONTACT, WS],
        )?;
        put_pipeline(tx, PIPELINE, WS, "sales")?;
        put_stage(tx, STAGE, WS, PIPELINE, "Lead", 0)?;
        put_pipeline(tx, OTHER_PIPELINE, WS, "support")?;
        put_stage(tx, STAGE + 10, WS, OTHER_PIPELINE, "Lead", 0)
    })
    .await;
    seed_item(
        &db,
        "Stage by user",
        vec![act("stageChanged", "user", Some(USER), "2026-03-10")],
    )
    .await;
    seed_item(
        &db,
        "Status by flow",
        vec![act("statusChanged", "flow", None, "2026-03-20")],
    )
    .await;
    seed_item(
        &db,
        "Comment by other user",
        vec![act(
            "commentAdded",
            "user",
            Some(ANOTHER_USER),
            "2026-04-05",
        )],
    )
    .await;
    // Two rows, each satisfying only one part of "stage change by the user".
    seed_item(
        &db,
        "Split rows",
        vec![
            act("stageChanged", "flow", None, "2026-03-10"),
            act("commentAdded", "user", Some(USER), "2026-03-10"),
        ],
    )
    .await;
    // Matching row recorded under another pipeline (before a transfer).
    seed_item(
        &db,
        "Other pipeline only",
        vec![ActivitySeed {
            elsewhere: true,
            ..act("stageChanged", "user", Some(USER), "2026-03-10")
        }],
    )
    .await;
    seed_item(&db, "No activity", vec![]).await;
    db
}

fn input(filter: PipelineItemFilter) -> ListStagePageInput {
    ListStagePageInput {
        workspace_id: WS,
        pipeline_id: PIPELINE,
        stage_id: Some(STAGE),
        filter,
        sort: PipelineItemSort {
            key: PipelineItemSortKey::CreatedAt,
            direction: SortDir::Asc,
        },
        limit: 50,
        cursor: None,
    }
}

async fn list(db: &Db, filter: PipelineItemFilter) -> Vec<String> {
    let input = input(filter);
    titles(&ok(repo::read(db, move |c| pipeline_item::list_stage_page(c, &input)).await).items)
}

async fn match_titles(db: &Db, activity_match: Option<PipelineActivityMatch>) -> Vec<String> {
    list(
        db,
        PipelineItemFilter {
            activity_match,
            ..Default::default()
        },
    )
    .await
}

fn kinds(k: &[&str]) -> Vec<String> {
    k.iter().map(|s| (*s).to_owned()).collect()
}

fn range(from: Option<&str>, to: Option<&str>) -> Option<DateRange> {
    Some(DateRange {
        from: from.map(day),
        to: to.map(day),
    })
}

/// Port of: packages/database/__tests__/integration/pipeline-activity-filter.test.ts:208 "pipeline activity filter SQL > no condition (or an empty one) filters nothing"
#[tokio::test]
async fn pipeline_activity_filter_sql_no_condition_or_an_empty_one_filters_nothing() {
    let db = activity_db().await;
    let all = vec![
        "Stage by user",
        "Status by flow",
        "Comment by other user",
        "Split rows",
        "Other pipeline only",
        "No activity",
    ];

    assert_eq!(match_titles(&db, None).await, all);
    assert_eq!(
        match_titles(&db, Some(PipelineActivityMatch::default())).await,
        all
    );
    assert_eq!(
        match_titles(
            &db,
            Some(PipelineActivityMatch {
                kinds: vec![],
                actor_user_ids: vec![],
                ..Default::default()
            })
        )
        .await,
        all
    );
}

/// Port of: packages/database/__tests__/integration/pipeline-activity-filter.test.ts:222 "pipeline activity filter SQL > kinds match any of the listed kinds"
#[tokio::test]
async fn pipeline_activity_filter_sql_kinds_match_any_of_the_listed_kinds() {
    let db = activity_db().await;
    let by = |k: &[&str]| PipelineActivityMatch {
        kinds: kinds(k),
        ..Default::default()
    };

    assert_eq!(
        match_titles(&db, Some(by(&["stageChanged"]))).await,
        vec!["Stage by user", "Split rows"]
    );
    assert_eq!(
        match_titles(&db, Some(by(&["statusChanged", "commentAdded"]))).await,
        vec!["Status by flow", "Comment by other user", "Split rows"]
    );
}

/// Port of: packages/database/__tests__/integration/pipeline-activity-filter.test.ts:234 "pipeline activity filter SQL > actor user matches only user rows of that user"
#[tokio::test]
async fn pipeline_activity_filter_sql_actor_user_matches_only_user_rows_of_that_user() {
    let db = activity_db().await;
    let by = |ids: Vec<i64>| PipelineActivityMatch {
        actor_user_ids: ids,
        ..Default::default()
    };

    assert_eq!(
        match_titles(&db, Some(by(vec![USER]))).await,
        vec!["Stage by user", "Split rows"]
    );
    assert_eq!(
        match_titles(&db, Some(by(vec![USER, ANOTHER_USER]))).await,
        vec!["Stage by user", "Comment by other user", "Split rows"]
    );
}

/// Port of: packages/database/__tests__/integration/pipeline-activity-filter.test.ts:246 "pipeline activity filter SQL > automation matches flow, trigger, token and system rows"
#[tokio::test]
async fn pipeline_activity_filter_sql_automation_matches_flow_trigger_token_and_system_rows() {
    let db = activity_db().await;

    assert_eq!(
        match_titles(
            &db,
            Some(PipelineActivityMatch {
                automation: true,
                ..Default::default()
            })
        )
        .await,
        vec!["Status by flow", "Split rows"]
    );
}

/// Port of: packages/database/__tests__/integration/pipeline-activity-filter.test.ts:253 "pipeline activity filter SQL > a user and automation are alternatives for who did it"
#[tokio::test]
async fn pipeline_activity_filter_sql_a_user_and_automation_are_alternatives_for_who_did_it() {
    let db = activity_db().await;

    assert_eq!(
        match_titles(
            &db,
            Some(PipelineActivityMatch {
                actor_user_ids: vec![ANOTHER_USER],
                automation: true,
                ..Default::default()
            })
        )
        .await,
        vec!["Status by flow", "Comment by other user", "Split rows"]
    );
}

/// Port of: packages/database/__tests__/integration/pipeline-activity-filter.test.ts:259 "pipeline activity filter SQL > the date range bounds the row time inclusively"
#[tokio::test]
async fn pipeline_activity_filter_sql_the_date_range_bounds_the_row_time_inclusively() {
    let db = activity_db().await;
    let by = |occurred_at: Option<DateRange>| PipelineActivityMatch {
        occurred_at,
        ..Default::default()
    };

    assert_eq!(
        match_titles(&db, Some(by(range(Some("2026-03-15"), Some("2026-04-30"))))).await,
        vec!["Status by flow", "Comment by other user"]
    );
    assert_eq!(
        match_titles(&db, Some(by(range(None, Some("2026-03-10"))))).await,
        vec!["Stage by user", "Split rows"]
    );
}

/// Port of: packages/database/__tests__/integration/pipeline-activity-filter.test.ts:271 "pipeline activity filter SQL > every part must hold on ONE row: rows each satisfying one part do not match"
#[tokio::test]
async fn pipeline_activity_filter_sql_every_part_must_hold_on_one_row_rows_each_satisfying_one_part_do_not_match()
 {
    let db = activity_db().await;
    let stage_by_user = |occurred_at: Option<DateRange>| PipelineActivityMatch {
        kinds: kinds(&["stageChanged"]),
        actor_user_ids: vec![USER],
        occurred_at,
        ..Default::default()
    };

    // "Split rows" has a stage change (by a flow) and a comment (by the user).
    assert_eq!(
        match_titles(&db, Some(stage_by_user(None))).await,
        vec!["Stage by user"]
    );
    assert_eq!(
        match_titles(
            &db,
            Some(PipelineActivityMatch {
                kinds: kinds(&["commentAdded"]),
                automation: true,
                ..Default::default()
            })
        )
        .await,
        Vec::<String>::new()
    );
    assert_eq!(
        match_titles(
            &db,
            Some(stage_by_user(range(Some("2026-03-01"), Some("2026-03-31"))))
        )
        .await,
        vec!["Stage by user"]
    );
    assert_eq!(
        match_titles(&db, Some(stage_by_user(range(Some("2026-04-01"), None)))).await,
        Vec::<String>::new()
    );
}

/// Port of: packages/database/__tests__/integration/pipeline-activity-filter.test.ts:295 "pipeline activity filter SQL > a row recorded under another pipeline never matches"
#[tokio::test]
async fn pipeline_activity_filter_sql_a_row_recorded_under_another_pipeline_never_matches() {
    let db = activity_db().await;
    let other = "Other pipeline only".to_owned();

    let both = match_titles(
        &db,
        Some(PipelineActivityMatch {
            kinds: kinds(&["stageChanged"]),
            actor_user_ids: vec![USER],
            ..Default::default()
        }),
    )
    .await;
    assert!(!both.contains(&other));
    let by_user = match_titles(
        &db,
        Some(PipelineActivityMatch {
            actor_user_ids: vec![USER],
            ..Default::default()
        }),
    )
    .await;
    assert!(!by_user.contains(&other));
    let by_kind = match_titles(
        &db,
        Some(PipelineActivityMatch {
            kinds: kinds(&["stageChanged"]),
            ..Default::default()
        }),
    )
    .await;
    assert!(!by_kind.contains(&other));
}

/// Port of: packages/database/__tests__/integration/pipeline-activity-filter.test.ts:307 "pipeline activity filter SQL > the per-kind activity date filter also ignores rows of another pipeline"
#[tokio::test]
async fn pipeline_activity_filter_sql_the_per_kind_activity_date_filter_also_ignores_rows_of_another_pipeline()
 {
    let db = activity_db().await;
    let by_kind = list(
        &db,
        PipelineItemFilter {
            activity: BTreeMap::from([(
                "stageChanged".to_owned(),
                DateRange {
                    from: Some(day("2026-01-01")),
                    to: Some(day("2026-12-31")),
                },
            )]),
            ..Default::default()
        },
    )
    .await;

    assert!(!by_kind.contains(&"Other pipeline only".to_owned()));
}

/// Port of: packages/database/__tests__/integration/pipeline-activity-filter.test.ts:326 "pipeline activity filter SQL > a deleted contact's titles are cleared from history; other contacts and counts stay"
#[tokio::test]
async fn pipeline_activity_filter_sql_a_deleted_contact_s_titles_are_cleared_from_history_other_contacts_and_counts_stay()
 {
    let db = activity_db().await;
    let other_contact: i64 = 987_654_321;
    setup(&db, move |tx| {
        for (contact, title) in [(CONTACT, "Ada Lovelace"), (other_contact, "Kept")] {
            tx.execute(
                r#"INSERT INTO "PipelineItemActivity" ("id", "workspaceId", "pipelineId", "itemId", "itemNumber", "itemTitle", "contactId", "kind", "actorKind", "fromTitle", "toTitle", "occurredAt")
                   VALUES (?1, ?2, ?3, 1, 1, ?4, ?5, 'titleChanged', 'user', ?6, ?4, ?7)"#,
                params![
                    repo::new_id(),
                    WS,
                    PIPELINE,
                    title,
                    contact,
                    format!("{title} before"),
                    day("2026-05-01")
                ],
            )?;
        }
        Ok(())
    })
    .await;

    ok(repo::write(&db, move |tx| {
        pipeline_activity::scrub_contact_titles(tx, WS, &[CONTACT])
    })
    .await);

    let rows = ok(repo::read(&db, |c| {
        repo::find_many::<PipelineItemActivity>(
            c,
            "WHERE \"kind\" = 'titleChanged' ORDER BY \"id\"",
            &[],
        )
    })
    .await);
    let own: Vec<(i64, String, Option<String>, Option<String>)> = rows
        .iter()
        .filter(|r| r.contact_id == CONTACT)
        .map(|r| {
            (
                r.contact_id,
                r.item_title.clone(),
                r.from_title.clone(),
                r.to_title.clone(),
            )
        })
        .collect();
    assert_eq!(own, vec![(CONTACT, String::new(), None, None)]);
    let kept = rows.iter().find(|r| r.contact_id == other_contact);
    assert_eq!(kept.and_then(|r| r.to_title.as_deref()), Some("Kept"));
}
