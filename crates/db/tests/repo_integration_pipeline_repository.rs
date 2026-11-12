//! Port of `packages/database/__tests__/integration/pipeline-repository.test.ts` (P3.3).
//! The TS runs against Postgres; each port runs against a migrated SQLite file. Money is
//! x 10,000 (`money(10)`), timestamps are unix ms.

mod common;
#[macro_use]
#[path = "common/repo.rs"]
mod repo_common;
#[path = "common/repo7.rs"]
mod repo7;

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;

use chatbotx_db::repo::models::PipelineItem;
use chatbotx_db::repo::pipeline::{self, PipelineInsert, StripFieldKeyInput};
use chatbotx_db::repo::pipeline_activity::{
    self, ListActivityInput, PipelineActivityInsert, PipelineAnalyticsRange,
};
use chatbotx_db::repo::pipeline_field_condition::{
    CustomFieldType, PipelineFieldCondition, PipelineFieldOperator,
};
use chatbotx_db::repo::pipeline_item::{
    self, BoardFirstPageInput, ListStagePageInput, MoveStageItemsInput,
};
use chatbotx_db::repo::pipeline_item_content;
use chatbotx_db::repo::pipeline_item_cursor::{
    PipelineItemSort, PipelineItemSortKey, is_invalid_cursor,
};
use chatbotx_db::repo::pipeline_item_filter::{
    ClosedAs, DateRange, PipelineItemFilter, PipelineValueFilter, PipelineValueOperator,
};
use chatbotx_db::repo::pipeline_stage;
use chatbotx_db::repo::pipeline_variable_targets;
use chatbotx_db::repo::utils::SortDir;
use chatbotx_db::repo::{self, Json, now_ms};
use repo_common::{ms, ok, setup};
use repo7::*;

fn sort(key: PipelineItemSortKey, direction: SortDir) -> PipelineItemSort {
    PipelineItemSort { key, direction }
}

const CREATED_DESC: PipelineItemSort = PipelineItemSort {
    key: PipelineItemSortKey::CreatedAt,
    direction: SortDir::Desc,
};

fn page_input(fx: PipeFx, sort: PipelineItemSort, limit: i64) -> ListStagePageInput {
    ListStagePageInput {
        workspace_id: fx.workspace_id,
        pipeline_id: fx.pipeline_id,
        stage_id: Some(fx.stage_ids[0]),
        filter: PipelineItemFilter::default(),
        sort,
        limit,
        cursor: None,
    }
}

async fn list_page(
    db: &chatbotx_db::Db,
    input: ListStagePageInput,
) -> pipeline_item::PipelineItemPage {
    ok(repo::read(db, move |c| pipeline_item::list_stage_page(c, &input)).await)
}

fn pipeline_values(fx: PipeFx, name: &str, currency: Option<&str>) -> PipelineInsert {
    PipelineInsert {
        workspace_id: fx.workspace_id,
        name: name.to_owned(),
        r#type: "opportunity".to_owned(),
        reference_name: name.to_owned(),
        currency: currency.map(str::to_owned),
        settings: Json(serde_json::json!({})),
        created_by_id: None,
    }
}

fn activity(
    fx: PipeFx,
    item: &PipelineItem,
    kind: &str,
    occurred_at: i64,
) -> PipelineActivityInsert {
    PipelineActivityInsert {
        workspace_id: fx.workspace_id,
        pipeline_id: fx.pipeline_id,
        item_id: item.id,
        item_number: item.number,
        item_title: item.title.clone(),
        contact_id: item.contact_id,
        kind: kind.to_owned(),
        actor_kind: "user".to_owned(),
        occurred_at,
        ..PipelineActivityInsert::default()
    }
}

async fn insert_activity(db: &chatbotx_db::Db, rows: Vec<PipelineActivityInsert>) {
    ok(repo::write(db, move |tx| pipeline_activity::insert_many(tx, &rows)).await);
}

/// Port of: packages/database/__tests__/integration/pipeline-repository.test.ts:164 "pipeline repositories > stores a pipeline with no currency override"
#[tokio::test]
async fn pipeline_repositories_stores_a_pipeline_with_no_currency_override() {
    let (db, own, _) = pipeline_db().await;
    let values = pipeline_values(own, "Follows workspace", None);
    let row = ok(repo::write(&db, move |tx| pipeline::insert(tx, &values)).await);
    assert_eq!(row.currency, None);
}

/// Port of: packages/database/__tests__/integration/pipeline-repository.test.ts:180 "pipeline repositories > pages every pipeline id of the workspace, whatever its currency, in id order"
#[tokio::test]
async fn pipeline_repositories_pages_every_pipeline_id_of_the_workspace_whatever_its_currency_in_id_order()
 {
    let (db, own, other) = pipeline_db().await;
    let rows = ok(repo::write(&db, move |tx| {
        let follows = pipeline::insert(tx, &pipeline_values(own, "follows", None))?;
        let overrides = pipeline::insert(tx, &pipeline_values(own, "overrides", Some("USD")))?;
        pipeline::insert(tx, &pipeline_values(other, "other_workspace", None))?;
        Ok((follows.id, overrides.id))
    })
    .await);
    let mut expected = vec![own.pipeline_id, rows.0, rows.1];
    expected.sort_unstable();

    let mut pages: Vec<Vec<i64>> = Vec::new();
    let mut after_id: Option<i64> = None;
    loop {
        let page = ok(repo::read(&db, move |c| {
            pipeline::list_ids_page(c, own.workspace_id, after_id, 2)
        })
        .await);
        let len = page.len();
        after_id = page.last().copied();
        pages.push(page);
        if len < 2 {
            break;
        }
    }

    assert_eq!(pages.concat(), expected);
    assert_eq!(pages.iter().map(Vec::len).collect::<Vec<_>>(), vec![2, 1]);
}

/// Port of: packages/database/__tests__/integration/pipeline-repository.test.ts:224 "pipeline repositories > hands out consecutive item numbers per workspace under concurrency"
#[tokio::test]
async fn pipeline_repositories_hands_out_consecutive_item_numbers_per_workspace_under_concurrency()
{
    let (db, own, other) = pipeline_db().await;
    let db = Arc::new(db);
    let mut tasks = Vec::new();
    for _ in 0..8 {
        let db = Arc::clone(&db);
        tasks.push(tokio::spawn(async move {
            repo::write(&db, move |tx| {
                pipeline::next_item_number(tx, own.workspace_id)
            })
            .await
        }));
    }
    let mut numbers = Vec::new();
    for task in tasks {
        numbers.push(ok(task.await.expect("join")));
    }
    let other_first = ok(repo::write(&db, move |tx| {
        pipeline::next_item_number(tx, other.workspace_id)
    })
    .await);

    numbers.sort_unstable();
    assert_eq!(numbers, vec![1, 2, 3, 4, 5, 6, 7, 8]);
    assert_eq!(other_first, 1);
}

/// Port of: packages/database/__tests__/integration/pipeline-repository.test.ts:242 "pipeline repositories > returns the existing row when an idempotency key is replayed"
#[tokio::test]
async fn pipeline_repositories_returns_the_existing_row_when_an_idempotency_key_is_replayed() {
    let (db, own, _) = pipeline_db().await;
    let first = insert_item(&db, own, |v| v.idempotency_key = Some("k1".to_owned())).await;
    let replay = insert_item(&db, own, |v| v.idempotency_key = Some("k1".to_owned())).await;

    assert_eq!(replay.id, first.id);
    let count = ok(repo::read(&db, move |c| {
        pipeline_item::count_by_stage(c, own.workspace_id, own.pipeline_id, own.stage_ids[0])
    })
    .await);
    assert_eq!(count, 1);
}

/// Port of: packages/database/__tests__/integration/pipeline-repository.test.ts:256 "pipeline repositories > scopes an idempotency key to its pipeline"
#[tokio::test]
async fn pipeline_repositories_scopes_an_idempotency_key_to_its_pipeline() {
    let (db, own, other) = pipeline_db().await;
    insert_item(&db, own, |v| v.idempotency_key = Some("shared".to_owned())).await;
    let elsewhere = insert_item(&db, other, |v| {
        v.idempotency_key = Some("shared".to_owned())
    })
    .await;

    assert_eq!(elsewhere.workspace_id, other.workspace_id);
}

/// The keyset-paging seed: values, due dates and priorities with ties and NULLs.
async fn seed_keyset(db: &chatbotx_db::Db, own: PipeFx) {
    let base = ms("2026-01-01T00:00:00.000Z");
    let rows: [(Option<i64>, Option<i64>, &str); 6] = [
        (Some(money(10)), Some(base + 1000), "low"),
        (Some(money(30)), Some(base + 3000), "high"),
        (None, None, "medium"),
        (Some(money(20)), Some(base + 2000), "high"),
        (None, None, "low"),
        (Some(money(30)), Some(base + 3000), "medium"),
    ];
    for (value, due_date, priority) in rows {
        insert_item(db, own, move |v| {
            v.value = value;
            v.due_date = due_date;
            v.priority = priority.to_owned();
        })
        .await;
    }
}

async fn collect(
    db: &chatbotx_db::Db,
    own: PipeFx,
    key: PipelineItemSortKey,
    direction: SortDir,
    page_size: i64,
) -> Vec<i64> {
    let mut ids = Vec::new();
    let mut cursor: Option<String> = None;
    for _ in 0..20 {
        let mut input = page_input(own, sort(key, direction), page_size);
        input.cursor = cursor.clone();
        let page = list_page(db, input).await;
        ids.extend(page.items.iter().map(|i| i.id));
        match page.next_cursor {
            None => return ids,
            Some(next) => cursor = Some(next),
        }
    }
    panic!("paging did not terminate");
}

async fn pages_every_item_exactly_once(key: PipelineItemSortKey, direction: SortDir) {
    let (db, own, _) = pipeline_db().await;
    seed_keyset(&db, own).await;

    let paged = collect(&db, own, key, direction, 2).await;
    let single = collect(&db, own, key, direction, 100).await;

    assert_eq!(paged.len(), 6);
    assert_eq!(paged.iter().collect::<HashSet<_>>().len(), 6);
    assert_eq!(paged, single);
}

/// Port of: packages/database/__tests__/integration/pipeline-repository.test.ts:304 "pipeline repositories > keyset paging > pages every item exactly once sorted by %s %s (createdAt asc)"
#[tokio::test]
async fn pipeline_repositories_keyset_paging_pages_every_item_exactly_once_sorted_by_created_at_asc()
 {
    pages_every_item_exactly_once(PipelineItemSortKey::CreatedAt, SortDir::Asc).await;
}

/// Port of: packages/database/__tests__/integration/pipeline-repository.test.ts:304 "pipeline repositories > keyset paging > pages every item exactly once sorted by %s %s (createdAt desc)"
#[tokio::test]
async fn pipeline_repositories_keyset_paging_pages_every_item_exactly_once_sorted_by_created_at_desc()
 {
    pages_every_item_exactly_once(PipelineItemSortKey::CreatedAt, SortDir::Desc).await;
}

/// Port of: packages/database/__tests__/integration/pipeline-repository.test.ts:304 "pipeline repositories > keyset paging > pages every item exactly once sorted by %s %s (updatedAt desc)"
#[tokio::test]
async fn pipeline_repositories_keyset_paging_pages_every_item_exactly_once_sorted_by_updated_at_desc()
 {
    pages_every_item_exactly_once(PipelineItemSortKey::UpdatedAt, SortDir::Desc).await;
}

/// Port of: packages/database/__tests__/integration/pipeline-repository.test.ts:304 "pipeline repositories > keyset paging > pages every item exactly once sorted by %s %s (value asc)"
#[tokio::test]
async fn pipeline_repositories_keyset_paging_pages_every_item_exactly_once_sorted_by_value_asc() {
    pages_every_item_exactly_once(PipelineItemSortKey::Value, SortDir::Asc).await;
}

/// Port of: packages/database/__tests__/integration/pipeline-repository.test.ts:304 "pipeline repositories > keyset paging > pages every item exactly once sorted by %s %s (value desc)"
#[tokio::test]
async fn pipeline_repositories_keyset_paging_pages_every_item_exactly_once_sorted_by_value_desc() {
    pages_every_item_exactly_once(PipelineItemSortKey::Value, SortDir::Desc).await;
}

/// Port of: packages/database/__tests__/integration/pipeline-repository.test.ts:304 "pipeline repositories > keyset paging > pages every item exactly once sorted by %s %s (dueDate asc)"
#[tokio::test]
async fn pipeline_repositories_keyset_paging_pages_every_item_exactly_once_sorted_by_due_date_asc()
{
    pages_every_item_exactly_once(PipelineItemSortKey::DueDate, SortDir::Asc).await;
}

/// Port of: packages/database/__tests__/integration/pipeline-repository.test.ts:304 "pipeline repositories > keyset paging > pages every item exactly once sorted by %s %s (dueDate desc)"
#[tokio::test]
async fn pipeline_repositories_keyset_paging_pages_every_item_exactly_once_sorted_by_due_date_desc()
{
    pages_every_item_exactly_once(PipelineItemSortKey::DueDate, SortDir::Desc).await;
}

/// Port of: packages/database/__tests__/integration/pipeline-repository.test.ts:304 "pipeline repositories > keyset paging > pages every item exactly once sorted by %s %s (priority asc)"
#[tokio::test]
async fn pipeline_repositories_keyset_paging_pages_every_item_exactly_once_sorted_by_priority_asc()
{
    pages_every_item_exactly_once(PipelineItemSortKey::Priority, SortDir::Asc).await;
}

/// Port of: packages/database/__tests__/integration/pipeline-repository.test.ts:304 "pipeline repositories > keyset paging > pages every item exactly once sorted by %s %s (priority desc)"
#[tokio::test]
async fn pipeline_repositories_keyset_paging_pages_every_item_exactly_once_sorted_by_priority_desc()
{
    pages_every_item_exactly_once(PipelineItemSortKey::Priority, SortDir::Desc).await;
}

/// Port of: packages/database/__tests__/integration/pipeline-repository.test.ts:325 "pipeline repositories > keyset paging > puts items without a value last in both directions"
#[tokio::test]
async fn pipeline_repositories_keyset_paging_puts_items_without_a_value_last_in_both_directions() {
    let (db, own, _) = pipeline_db().await;
    seed_keyset(&db, own).await;

    for direction in [SortDir::Asc, SortDir::Desc] {
        let ids = collect(&db, own, PipelineItemSortKey::Value, direction, 100).await;
        let mut values = Vec::new();
        for id in ids {
            let row = ok(repo::read(&db, move |c| {
                pipeline_item::find_by_id(c, own.workspace_id, id, None)
            })
            .await);
            values.push(row.and_then(|r| r.value));
        }

        assert_eq!(values[4..].to_vec(), vec![None, None]);
        let expected = if direction == SortDir::Asc {
            [10, 20, 30, 30]
        } else {
            [30, 30, 20, 10]
        };
        assert_eq!(
            values[..4].to_vec(),
            expected.iter().map(|v| Some(money(*v))).collect::<Vec<_>>()
        );
    }
}

/// Port of: packages/database/__tests__/integration/pipeline-repository.test.ts:347 "pipeline repositories > keyset paging > rejects a tampered cursor value without reaching SQL"
#[tokio::test]
async fn pipeline_repositories_keyset_paging_rejects_a_tampered_cursor_value_without_reaching_sql()
{
    let (db, own, _) = pipeline_db().await;
    seed_keyset(&db, own).await;
    let first = list_page(&db, page_input(own, CREATED_DESC, 2)).await;

    assert!(first.next_cursor.is_some());
    let mut input = page_input(own, sort(PipelineItemSortKey::Value, SortDir::Desc), 2);
    input.cursor = first.next_cursor;
    let err = repo::read(&db, move |c| pipeline_item::list_stage_page(c, &input))
        .await
        .expect_err("tampered cursor");
    assert!(is_invalid_cursor(&err));
    assert!(err.message().contains("Invalid cursor"));
}

/// Port of: packages/database/__tests__/integration/pipeline-repository.test.ts:371 "pipeline repositories > board first page returns at most N items per stage plus stage totals"
#[tokio::test]
async fn pipeline_repositories_board_first_page_returns_at_most_n_items_per_stage_plus_stage_totals()
 {
    let (db, own, _) = pipeline_db().await;
    for _ in 0..3 {
        insert_item(&db, own, |v| v.value = Some(money(10))).await;
    }
    insert_item(&db, own, move |v| {
        v.stage_id = own.stage_ids[1];
        v.value = Some(money(5));
    })
    .await;

    let board = BoardFirstPageInput {
        workspace_id: own.workspace_id,
        pipeline_id: own.pipeline_id,
        filter: PipelineItemFilter::default(),
        sort: CREATED_DESC,
        per_stage_limit: 2,
    };
    let pages = ok(repo::read(&db, move |c| {
        pipeline_item::list_board_first_page(c, &board)
    })
    .await);
    let totals = ok(repo::read(&db, move |c| {
        pipeline_item::list_stage_totals(
            c,
            own.workspace_id,
            own.pipeline_id,
            &PipelineItemFilter::default(),
        )
    })
    .await);

    let page_of = |stage_id: i64| pages.iter().find(|p| p.stage_id == stage_id);
    assert_eq!(page_of(own.stage_ids[0]).map(|p| p.items.len()), Some(2));
    assert_eq!(page_of(own.stage_ids[1]).map(|p| p.items.len()), Some(1));
    // 3 items, limit 2: the first stage has more, the second does not.
    assert!(page_of(own.stage_ids[0]).is_some_and(|p| p.next_cursor.is_some()));
    assert!(page_of(own.stage_ids[1]).is_some_and(|p| p.next_cursor.is_none()));
    // The rows are plain `PipelineItem`s (no `sortValue` field exists on the type).
    let by_stage: HashMap<i64, (i64, i64)> = totals
        .iter()
        .map(|t| (t.stage_id, (t.count, t.total_value)))
        .collect();
    assert_eq!(
        by_stage,
        HashMap::from([
            (own.stage_ids[0], (3, money(30))),
            (own.stage_ids[1], (1, money(5))),
        ])
    );
}

async fn run_titles(
    db: &chatbotx_db::Db,
    own: PipeFx,
    direction: SortDir,
    limit: i64,
    filter: PipelineItemFilter,
) -> Vec<String> {
    let mut input = page_input(own, sort(PipelineItemSortKey::CreatedAt, direction), limit);
    input.filter = filter;
    titles(&list_page(db, input).await.items)
}

fn sorted(mut v: Vec<String>) -> Vec<String> {
    v.sort();
    v
}

/// Port of: packages/database/__tests__/integration/pipeline-repository.test.ts:407 "pipeline repositories > filters by status, priority, search and field values"
#[tokio::test]
async fn pipeline_repositories_filters_by_status_priority_search_and_field_values() {
    let (db, own, _) = pipeline_db().await;
    insert_item(&db, own, |v| {
        v.title = "Alpha deal".to_owned();
        v.priority = "high".to_owned();
    })
    .await;
    insert_item(&db, own, |v| {
        v.title = "Beta".to_owned();
        v.status = "won".to_owned();
        v.field_values = Json(serde_json::json!({ "99": "gold" }));
    })
    .await;
    let run = |filter: PipelineItemFilter| run_titles(&db, own, SortDir::Desc, 10, filter);

    assert_eq!(
        run(PipelineItemFilter {
            statuses: vec!["won".to_owned()],
            ..Default::default()
        })
        .await,
        vec!["Beta"]
    );
    assert_eq!(
        run(PipelineItemFilter {
            priorities: vec!["high".to_owned()],
            ..Default::default()
        })
        .await,
        vec!["Alpha deal"]
    );
    assert_eq!(
        run(PipelineItemFilter {
            search: Some("alp".to_owned()),
            ..Default::default()
        })
        .await,
        vec!["Alpha deal"]
    );
    let closed_day = ms("2026-02-10T12:00:00.000Z");
    insert_item(&db, own, move |v| {
        v.title = "Closed lost".to_owned();
        v.status = "lost".to_owned();
        v.closed_at = Some(closed_day);
    })
    .await;
    insert_item(&db, own, move |v| {
        v.title = "Closed won".to_owned();
        v.status = "won".to_owned();
        v.closed_at = Some(closed_day);
    })
    .await;
    let day_range = DateRange {
        from: Some(ms("2026-02-10T00:00:00.000Z")),
        to: Some(ms("2026-02-10T23:59:59.999Z")),
    };
    let closed_as = |won: Option<DateRange>, lost: Option<DateRange>| PipelineItemFilter {
        closed_as: Some(ClosedAs { won, lost }),
        ..Default::default()
    };
    assert_eq!(
        run(closed_as(Some(day_range), None)).await,
        vec!["Closed won"]
    );
    assert_eq!(
        run(closed_as(None, Some(day_range))).await,
        vec!["Closed lost"]
    );
    assert_eq!(
        sorted(run(closed_as(Some(day_range), Some(day_range))).await),
        vec!["Closed lost", "Closed won"]
    );
    let later = DateRange {
        from: Some(ms("2030-01-01T00:00:00.000Z")),
        to: None,
    };
    assert_eq!(
        run(closed_as(Some(day_range), Some(later))).await,
        vec!["Closed won"]
    );
    assert_eq!(
        sorted(
            run(PipelineItemFilter {
                closed_at: Some(day_range),
                ..Default::default()
            })
            .await
        ),
        vec!["Closed lost", "Closed won"]
    );
    let stamp = ms("2026-03-05T12:00:00.000Z");
    let march = DateRange {
        from: Some(ms("2026-03-05T00:00:00.000Z")),
        to: Some(ms("2026-03-05T23:59:59.999Z")),
    };
    insert_item(&db, own, move |v| {
        v.title = "Own timestamps".to_owned();
        v.last_comment_at = Some(stamp);
        v.status_changed_at = Some(stamp);
        v.stage_entered_at = stamp;
    })
    .await;
    assert_eq!(
        run(PipelineItemFilter {
            last_comment_at: Some(march),
            ..Default::default()
        })
        .await,
        vec!["Own timestamps"]
    );
    assert_eq!(
        run(PipelineItemFilter {
            status_changed_at: Some(march),
            ..Default::default()
        })
        .await,
        vec!["Own timestamps"]
    );
    assert_eq!(
        run(PipelineItemFilter {
            stage_entered_at: Some(march),
            ..Default::default()
        })
        .await,
        vec!["Own timestamps"]
    );
    assert_eq!(
        run(PipelineItemFilter {
            field_conditions: BTreeMap::from([(
                "99".to_owned(),
                PipelineFieldCondition::with(PipelineFieldOperator::Is, "gold"),
            )]),
            field_types: BTreeMap::from([("99".to_owned(), CustomFieldType::ShortText)]),
            ..Default::default()
        })
        .await,
        vec!["Beta"]
    );
    assert_eq!(
        run(PipelineItemFilter {
            search: Some("'; drop table \"PipelineItem\"; --".to_owned()),
            ..Default::default()
        })
        .await,
        Vec::<String>::new()
    );
}

/// Port of: packages/database/__tests__/integration/pipeline-repository.test.ts:492 "pipeline repositories > filters by value operators and per-field conditions"
#[tokio::test]
async fn pipeline_repositories_filters_by_value_operators_and_per_field_conditions() {
    let (db, own, _) = pipeline_db().await;
    for (title, value) in [
        ("Low", Some(10)),
        ("Mid", Some(20)),
        ("High", Some(30)),
        ("Blank", None),
    ] {
        insert_item(&db, own, move |v| {
            v.title = title.to_owned();
            v.value = value.map(money);
        })
        .await;
    }
    let value_filter = |operator: Option<PipelineValueOperator>,
                        amount: Option<i64>,
                        min: Option<i64>,
                        max: Option<i64>| PipelineItemFilter {
        value: Some(PipelineValueFilter {
            operator,
            amount: amount.map(money),
            min: min.map(money),
            max: max.map(money),
        }),
        ..Default::default()
    };
    let run = |filter: PipelineItemFilter| run_titles(&db, own, SortDir::Asc, 20, filter);

    assert_eq!(
        run(value_filter(
            Some(PipelineValueOperator::Eq),
            Some(20),
            None,
            None
        ))
        .await,
        vec!["Mid"]
    );
    assert_eq!(
        run(value_filter(
            Some(PipelineValueOperator::Gt),
            Some(20),
            None,
            None
        ))
        .await,
        vec!["High"]
    );
    assert_eq!(
        run(value_filter(
            Some(PipelineValueOperator::Lt),
            Some(20),
            None,
            None
        ))
        .await,
        vec!["Low"]
    );
    // "Different" also matches the item without a value.
    assert_eq!(
        run(value_filter(
            Some(PipelineValueOperator::Ne),
            Some(20),
            None,
            None
        ))
        .await,
        vec!["Low", "High", "Blank"]
    );
    assert_eq!(
        run(value_filter(
            Some(PipelineValueOperator::Between),
            None,
            Some(15),
            Some(30)
        ))
        .await,
        vec!["Mid", "High"]
    );
    // The legacy `{min,max}` shape is still an interval.
    assert_eq!(
        run(value_filter(None, None, Some(15), Some(25))).await,
        vec!["Mid"]
    );
}

/// Port of: packages/database/__tests__/integration/pipeline-repository.test.ts:535 "pipeline repositories > never returns another workspace's items"
#[tokio::test]
async fn pipeline_repositories_never_returns_another_workspace_s_items() {
    let (db, own, other) = pipeline_db().await;
    insert_item(&db, other, |v| v.title = "Foreign".to_owned()).await;

    let mut input = page_input(other, CREATED_DESC, 10);
    input.workspace_id = own.workspace_id;
    let page = list_page(&db, input).await;

    assert!(page.items.is_empty());
}

/// Port of: packages/database/__tests__/integration/pipeline-repository.test.ts:549 "pipeline repositories > reorders stages in one statement"
#[tokio::test]
async fn pipeline_repositories_reorders_stages_in_one_statement() {
    let (db, own, _) = pipeline_db().await;
    let [first, second] = own.stage_ids;

    ok(repo::write(&db, move |tx| {
        pipeline_stage::reorder(tx, own.workspace_id, own.pipeline_id, &[second, first])
    })
    .await);
    let stages = ok(repo::read(&db, move |c| {
        pipeline_stage::list_by_pipeline(c, own.workspace_id, own.pipeline_id)
    })
    .await);

    assert_eq!(
        stages.iter().map(|s| s.id).collect::<Vec<_>>(),
        vec![second, first]
    );
}

/// Port of: packages/database/__tests__/integration/pipeline-repository.test.ts:565 "pipeline repositories > moves and deletes a stage's items in bounded batches"
#[tokio::test]
async fn pipeline_repositories_moves_and_deletes_a_stage_s_items_in_bounded_batches() {
    let (db, own, _) = pipeline_db().await;
    for _ in 0..5 {
        insert_item(&db, own, |_| {}).await;
    }

    let moved = ok(repo::write(&db, move |tx| {
        pipeline_item::move_stage_items_batch(
            tx,
            &MoveStageItemsInput {
                workspace_id: own.workspace_id,
                pipeline_id: own.pipeline_id,
                from_stage_id: own.stage_ids[0],
                to_stage_id: own.stage_ids[1],
                limit: 3,
                now: now_ms(),
            },
        )
    })
    .await);
    let deleted = ok(repo::write(&db, move |tx| {
        pipeline_item::delete_stage_items_batch(
            tx,
            own.workspace_id,
            own.pipeline_id,
            own.stage_ids[0],
            500,
        )
    })
    .await);

    assert_eq!(moved.len(), 3);
    assert_eq!(deleted.len(), 2);
}

async fn item_rows(db: &chatbotx_db::Db, workspace_id: i64) -> Vec<PipelineItem> {
    ok(repo::read(db, move |c| {
        repo::find_many::<PipelineItem>(
            c,
            "WHERE \"workspaceId\" = ?1 ORDER BY \"id\"",
            &[&workspace_id],
        )
    })
    .await)
}

/// Port of: packages/database/__tests__/integration/pipeline-repository.test.ts:600 "pipeline repositories > removes a deleted field key in bounded batches without touching updatedAt"
#[tokio::test]
async fn pipeline_repositories_removes_a_deleted_field_key_in_bounded_batches_without_touching_updated_at()
 {
    let (db, own, _) = pipeline_db().await;
    for _ in 0..5 {
        insert_item(&db, own, |v| {
            v.field_values = Json(serde_json::json!({ "7": "a", "8": "b" }));
        })
        .await;
    }
    let before: HashMap<i64, i64> = item_rows(&db, own.workspace_id)
        .await
        .iter()
        .map(|i| (i.id, i.updated_at))
        .collect();
    let mut after_id: Option<i64> = None;
    let mut scanned = Vec::new();
    for _ in 0..3 {
        let input = StripFieldKeyInput {
            workspace_id: own.workspace_id,
            pipeline_id: own.pipeline_id,
            field_id: "7".to_owned(),
            limit: 2,
            after_id,
        };
        let batch =
            ok(repo::write(&db, move |tx| pipeline::strip_field_key_batch(tx, &input)).await);
        after_id = batch.last_id.or(after_id);
        scanned.push(batch.scanned);
    }

    assert_eq!(scanned, vec![2, 2, 1]);
    let items = item_rows(&db, own.workspace_id).await;
    assert_eq!(
        items
            .iter()
            .map(|i| i.field_values.0.clone())
            .collect::<Vec<_>>(),
        vec![serde_json::json!({ "8": "b" }); 5]
    );
    assert!(
        items
            .iter()
            .all(|i| before.get(&i.id) == Some(&i.updated_at))
    );
}

/// The TS `drainWhileMoving`, under the one SQLite writer: the move job (holding the
/// "row lock") commits before the drain job starts, so the drain sees the moved item.
async fn move_then_drain<R: Send + 'static>(
    db: &chatbotx_db::Db,
    own: PipeFx,
    moved_id: i64,
    drain: impl FnOnce(&mut chatbotx_db::WriteTx<'_>) -> repo::Result<R> + Send + 'static,
) -> R {
    ok(repo::write(db, move |tx| {
        tx.execute(
            "UPDATE \"PipelineItem\" SET \"stageId\" = ?1 WHERE \"id\" = ?2",
            (own.stage_ids[1], moved_id),
        )?;
        Ok(())
    })
    .await);
    ok(repo::write(db, drain).await)
}

/// Port of: packages/database/__tests__/integration/pipeline-repository.test.ts:692 "pipeline repositories > stage drain vs a concurrent move out of the stage > a delete batch skips an item that left the stage while it waited"
#[tokio::test]
async fn pipeline_repositories_stage_drain_vs_a_concurrent_move_out_of_the_stage_a_delete_batch_skips_an_item_that_left_the_stage_while_it_waited()
 {
    let (db, own, _) = pipeline_db().await;
    let moved = insert_item(&db, own, |_| {}).await;
    let stays = insert_item(&db, own, |_| {}).await;

    let removed = move_then_drain(&db, own, moved.id, move |tx| {
        pipeline_item::delete_stage_items_batch(
            tx,
            own.workspace_id,
            own.pipeline_id,
            own.stage_ids[0],
            10,
        )
    })
    .await;

    assert_eq!(
        removed.iter().map(|r| r.id).collect::<Vec<_>>(),
        vec![stays.id]
    );
    let survivors: Vec<(i64, i64)> = item_rows(&db, own.workspace_id)
        .await
        .iter()
        .map(|i| (i.id, i.stage_id))
        .collect();
    assert_eq!(survivors, vec![(moved.id, own.stage_ids[1])]);
}

/// Port of: packages/database/__tests__/integration/pipeline-repository.test.ts:721 "pipeline repositories > stage drain vs a concurrent move out of the stage > a move batch does not drag back an item that already left"
#[tokio::test]
async fn pipeline_repositories_stage_drain_vs_a_concurrent_move_out_of_the_stage_a_move_batch_does_not_drag_back_an_item_that_already_left()
 {
    let (db, own, _) = pipeline_db().await;
    let moved = insert_item(&db, own, |_| {}).await;
    insert_item(&db, own, |_| {}).await;
    let third = own.stage_ids[1];

    let rows = move_then_drain(&db, own, moved.id, move |tx| {
        pipeline_item::move_stage_items_batch(
            tx,
            &MoveStageItemsInput {
                workspace_id: own.workspace_id,
                pipeline_id: own.pipeline_id,
                from_stage_id: own.stage_ids[0],
                to_stage_id: third,
                limit: 10,
                now: now_ms() + 86_400_000,
            },
        )
    })
    .await;

    assert!(!rows.iter().any(|r| r.id == moved.id));
    assert_eq!(rows.len(), 1);
}

/// Port of: packages/database/__tests__/integration/pipeline-repository.test.ts:747 "pipeline repositories > deletes a pipeline's items and activity in bounded batches"
#[tokio::test]
async fn pipeline_repositories_deletes_a_pipeline_s_items_and_activity_in_bounded_batches() {
    let (db, own, _) = pipeline_db().await;
    for _ in 0..5 {
        insert_item(&db, own, |_| {}).await;
    }
    let rows = (1..=5)
        .map(|n| PipelineActivityInsert {
            workspace_id: own.workspace_id,
            pipeline_id: own.pipeline_id,
            item_id: n,
            item_number: n,
            item_title: "x".to_owned(),
            contact_id: own.contact_ids[0],
            kind: "created".to_owned(),
            actor_kind: "user".to_owned(),
            occurred_at: now_ms(),
            ..PipelineActivityInsert::default()
        })
        .collect();
    insert_activity(&db, rows).await;

    let mut items = Vec::new();
    for _ in 0..3 {
        items.push(ok(repo::write(&db, move |tx| {
            pipeline_item::delete_pipeline_items_batch(tx, own.workspace_id, own.pipeline_id, 3)
        })
        .await));
    }
    let mut activity = Vec::new();
    for _ in 0..3 {
        activity.push(ok(repo::write(&db, move |tx| {
            pipeline_activity::delete_by_pipeline_batch(tx, own.workspace_id, own.pipeline_id, 3)
        })
        .await));
    }

    assert_eq!(items, vec![3, 2, 0]);
    assert_eq!(activity, vec![3, 2, 0]);
}

/// Port of: packages/database/__tests__/integration/pipeline-repository.test.ts:801 "pipeline repositories > finds the latest open item and counts open items, optionally leaving one out"
#[tokio::test]
async fn pipeline_repositories_finds_the_latest_open_item_and_counts_open_items_optionally_leaving_one_out()
 {
    let (db, own, other) = pipeline_db().await;
    let older = insert_item(&db, own, |_| {}).await;
    insert_item(&db, own, |v| v.status = "won".to_owned()).await;
    let newest = insert_item(&db, own, |_| {}).await;
    insert_item(&db, own, move |v| v.contact_id = own.contact_ids[1]).await;
    let (ws, pid, cid) = (own.workspace_id, own.pipeline_id, own.contact_ids[0]);

    let latest = ok(repo::read(&db, move |c| {
        pipeline_item::find_latest_open_for_contact(c, ws, pid, cid)
    })
    .await);
    let count = ok(repo::read(&db, move |c| {
        pipeline_item::count_open_for_contact(c, ws, pid, cid, None)
    })
    .await);
    let newest_id = newest.id;
    let count_except = ok(repo::read(&db, move |c| {
        pipeline_item::count_open_for_contact(c, ws, pid, cid, Some(newest_id))
    })
    .await);
    let other_ws = other.workspace_id;
    let none = ok(repo::read(&db, move |c| {
        pipeline_item::find_latest_open_for_contact(c, other_ws, pid, cid)
    })
    .await);

    assert_eq!(latest.as_ref().map(|r| r.id), Some(newest.id));
    assert_ne!(latest.as_ref().map(|r| r.id), Some(older.id));
    assert_eq!(count, 2);
    assert_eq!(count_except, 1);
    assert!(none.is_none());
}

/// Port of: packages/database/__tests__/integration/pipeline-repository.test.ts:834 "pipeline repositories > finds the contact's last created or updated item across pipelines"
#[tokio::test]
async fn pipeline_repositories_finds_the_contact_s_last_created_or_updated_item_across_pipelines() {
    let (db, own, other) = pipeline_db().await;
    let older = insert_item(&db, own, |_| {}).await;
    let newer = insert_item(&db, own, |_| {}).await;
    let last = |ws: i64, contact: i64| {
        let db = &db;
        async move {
            ok(repo::read(db, move |c| {
                pipeline_variable_targets::find_last_touched_item_id(c, ws, contact)
            })
            .await)
        }
    };

    let after_create = last(own.workspace_id, own.contact_ids[0]).await;
    let older_id = older.id;
    setup(&db, move |tx| {
        Ok(tx.execute(
            "UPDATE \"PipelineItem\" SET \"updatedAt\" = ?1 WHERE \"id\" = ?2",
            (now_ms() + 60_000, older_id),
        )?)
    })
    .await;
    let after_update = last(own.workspace_id, own.contact_ids[0]).await;
    let other_contact = last(own.workspace_id, own.contact_ids[1]).await;
    let other_workspace = last(other.workspace_id, own.contact_ids[0]).await;

    assert_eq!(after_create, Some(newer.id));
    assert_eq!(after_update, Some(older.id));
    assert_eq!(other_contact, None);
    assert_eq!(other_workspace, None);
}

/// Port of: packages/database/__tests__/integration/pipeline-repository.test.ts:868 "pipeline repositories > requires exactly one assignee target"
#[tokio::test]
async fn pipeline_repositories_requires_exactly_one_assignee_target() {
    let (db, own, _) = pipeline_db().await;
    let item = insert_item(&db, own, |_| {}).await;

    let item_id = item.id;
    let err = db
        .write(move |tx| {
            Ok(tx.execute(
                "INSERT INTO \"PipelineItemAssignee\" (\"id\", \"workspaceId\", \"itemId\") VALUES (1, ?1, ?2)",
                (own.workspace_id, item_id),
            )?)
        })
        .await
        .expect_err("one target check");
    assert!(
        format!("{err:?} {err}").contains("PipelineItemAssignee_one_target_check"),
        "{err:?}"
    );
}

/// Port of: packages/database/__tests__/integration/pipeline-repository.test.ts:879 "pipeline repositories > keeps activity after the item is deleted and drops it with the pipeline"
#[tokio::test]
async fn pipeline_repositories_keeps_activity_after_the_item_is_deleted_and_drops_it_with_the_pipeline()
 {
    let (db, own, _) = pipeline_db().await;
    let item = insert_item(&db, own, |_| {}).await;
    insert_activity(&db, vec![activity(own, &item, "created", now_ms())]).await;
    let list = |item_id: i64| {
        let db = &db;
        async move {
            let input = ListActivityInput {
                workspace_id: own.workspace_id,
                item_id,
                limit: 10,
                ..ListActivityInput::default()
            };
            ok(repo::read(db, move |c| pipeline_activity::list_for_item(c, &input)).await)
        }
    };

    let item_id = item.id;
    ok(repo::write(&db, move |tx| {
        pipeline_item::delete(tx, own.workspace_id, item_id)
    })
    .await);
    let after_item_delete = list(item.id).await;
    ok(repo::write(&db, move |tx| {
        pipeline::delete(tx, own.workspace_id, own.pipeline_id)
    })
    .await);
    let after_pipeline_delete = list(item.id).await;

    assert_eq!(after_item_delete.items.len(), 1);
    assert_eq!(after_pipeline_delete.items.len(), 0);
}

/// Port of: packages/database/__tests__/integration/pipeline-repository.test.ts:918 "pipeline repositories > filters activity to the requested kinds"
#[tokio::test]
async fn pipeline_repositories_filters_activity_to_the_requested_kinds() {
    let (db, own, _) = pipeline_db().await;
    let item = insert_item(&db, own, |_| {}).await;
    insert_activity(
        &db,
        ["created", "stageChanged", "commentAdded"]
            .iter()
            .map(|k| activity(own, &item, k, now_ms()))
            .collect(),
    )
    .await;
    let read = |kinds: Option<Vec<String>>| {
        let db = &db;
        let item_id = item.id;
        async move {
            let input = ListActivityInput {
                workspace_id: own.workspace_id,
                item_id,
                limit: 10,
                cursor: None,
                kinds,
            };
            ok(repo::read(db, move |c| pipeline_activity::list_for_item(c, &input)).await)
        }
    };

    let filtered = read(Some(vec![
        "stageChanged".to_owned(),
        "commentAdded".to_owned(),
    ]))
    .await;
    let all = read(None).await;

    let mut kinds: Vec<String> = filtered.items.iter().map(|r| r.kind.clone()).collect();
    kinds.sort();
    assert_eq!(kinds, vec!["commentAdded", "stageChanged"]);
    assert_eq!(all.items.len(), 3);
}

/// Port of: packages/database/__tests__/integration/pipeline-repository.test.ts:956 "pipeline repositories > pages activity newest first without gaps or repeats on equal timestamps"
#[tokio::test]
async fn pipeline_repositories_pages_activity_newest_first_without_gaps_or_repeats_on_equal_timestamps()
 {
    let (db, own, _) = pipeline_db().await;
    let item = insert_item(&db, own, |_| {}).await;
    // `08:00:00.123456Z` in the TS: SQLite keeps whole milliseconds.
    let at = ms("2026-04-01T08:00:00.123Z");
    insert_activity(
        &db,
        (0..25)
            .map(|_| activity(own, &item, "commentAdded", at))
            .collect(),
    )
    .await;
    let item_id = item.id;
    let read = |cursor: Option<String>| {
        let db = &db;
        async move {
            let input = ListActivityInput {
                workspace_id: own.workspace_id,
                item_id,
                limit: 20,
                cursor,
                kinds: None,
            };
            repo::read(db, move |c| pipeline_activity::list_for_item(c, &input)).await
        }
    };

    let first = ok(read(None).await);
    let second = ok(read(first.next_cursor.clone()).await);

    assert_eq!(first.items.len(), 20);
    assert_eq!(second.items.len(), 5);
    assert!(second.next_cursor.is_none());
    let ids: HashSet<i64> = first
        .items
        .iter()
        .chain(&second.items)
        .map(|r| r.id)
        .collect();
    assert_eq!(ids.len(), 25);
    let bad = read(Some("not-a-cursor".to_owned()))
        .await
        .expect_err("bad cursor");
    assert!(bad.message().contains("Invalid cursor"));
    // A null-ranked cursor cannot come from a timestamp page; it must not silently
    // restart from the first page.
    let null_cursor = chatbotx_db::repo::pipeline_item_cursor::encode_pipeline_item_cursor(
        &chatbotx_db::repo::pipeline_item_cursor::PipelineItemCursor {
            v: 1,
            sort: PipelineItemSortKey::CreatedAt,
            dir: SortDir::Desc,
            null_rank: 1,
            sort_value: None,
            id: "1".to_owned(),
        },
    );
    let bad = read(Some(null_cursor.clone()))
        .await
        .expect_err("null cursor");
    assert!(bad.message().contains("Invalid cursor"));
    let comments = repo::read(&db, move |c| {
        pipeline_item_content::list_comments(c, own.workspace_id, item_id, 10, Some(&null_cursor))
    })
    .await
    .expect_err("null cursor on comments");
    assert!(comments.message().contains("Invalid cursor"));
}

/// Port of: packages/database/__tests__/integration/pipeline-repository.test.ts:1012 "pipeline repositories > counts created, won and lost transitions per day and per actor"
#[tokio::test]
async fn pipeline_repositories_counts_created_won_and_lost_transitions_per_day_and_per_actor() {
    let (db, own, _) = pipeline_db().await;
    let at = ms("2026-03-10T12:00:00.000Z");
    let base = PipelineActivityInsert {
        workspace_id: own.workspace_id,
        pipeline_id: own.pipeline_id,
        item_id: 1,
        item_number: 1,
        item_title: "x".to_owned(),
        contact_id: own.contact_ids[0],
        actor_kind: "user".to_owned(),
        actor_user_id: Some(own.user_id),
        occurred_at: at,
        ..PipelineActivityInsert::default()
    };
    let row = |kind: &str, to_status: Option<&str>, to_value: Option<i64>| PipelineActivityInsert {
        kind: kind.to_owned(),
        to_status: to_status.map(str::to_owned),
        to_value: to_value.map(money),
        ..base.clone()
    };
    insert_activity(
        &db,
        vec![
            row("created", None, Some(100)),
            row("statusChanged", Some("won"), Some(100)),
            row("statusChanged", Some("lost"), Some(50)),
            row("stageChanged", None, None),
        ],
    )
    .await;
    let range = PipelineAnalyticsRange {
        workspace_id: own.workspace_id,
        pipeline_id: own.pipeline_id,
        from: ms("2026-03-01T00:00:00.000Z"),
        to: ms("2026-03-31T23:59:59.000Z"),
    };

    let daily = ok(repo::read(&db, move |c| {
        pipeline_activity::daily_series(c, &range, "UTC")
    })
    .await);
    let by_actor =
        ok(repo::read(&db, move |c| pipeline_activity::totals_by_actor(c, &range)).await);

    assert!(daily.iter().all(|r| r.day == "2026-03-10"));
    let mut kinds: Vec<String> = daily.iter().map(|r| r.kind.clone()).collect();
    kinds.sort();
    assert_eq!(kinds, vec!["created", "statusChanged", "statusChanged"]);
    assert!(
        by_actor
            .iter()
            .all(|r| r.actor_user_id == Some(own.user_id))
    );
}
