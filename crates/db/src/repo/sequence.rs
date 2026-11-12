//! Port of `repositories/sequence/repository.ts` (P3.3).

use rusqlite::Connection;

use super::models::{Flow, Sequence, SequenceStep};
use super::sql::Sql;
use super::utils::{
    PaginationInput, SortItem, get_pagination_with_defaults, like_contains, order_by_sql,
    parse_order_by,
};
use super::{Model, Result, now_ms};

/// `rootFolderId` (`partials/folder.ts`).
pub const ROOT_FOLDER_ID: &str = "0";

/// `SequenceListInput` (`folder_id` is the API string: `"0"` = root).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SequenceListInput {
    pub workspace_id: i64,
    pub name: Option<String>,
    pub folder_id: Option<String>,
    pub active: Option<bool>,
    pub page: Option<i64>,
    pub per_page: Option<i64>,
    pub sort: Option<Vec<SortItem>>,
}

/// A sequence with `stepsCount` / `subscribersCount`.
#[derive(Debug, Clone, PartialEq)]
pub struct SequenceWithCounts {
    pub sequence: Sequence,
    pub steps_count: i64,
    pub subscribers_count: i64,
}

/// A step with its flow.
#[derive(Debug, Clone, PartialEq)]
pub struct StepWithFlow {
    pub step: SequenceStep,
    pub flow: Option<Flow>,
}

/// A sequence with its steps (by `order`).
#[derive(Debug, Clone, PartialEq)]
pub struct SequenceWithSteps {
    pub sequence: Sequence,
    pub steps: Vec<StepWithFlow>,
}

fn build_where(input: &SequenceListInput) -> Sql {
    let folder = input
        .folder_id
        .as_deref()
        .filter(|f| !f.is_empty())
        .map(|f| {
            if f == ROOT_FOLDER_ID {
                Sql::raw("\"folderId\" IS NULL")
            } else {
                // A non-numeric id matches nothing (Postgres would reject the bigint cast).
                match f.parse::<i64>() {
                    Ok(id) => Sql::bind("\"folderId\" = ", id),
                    Err(_) => Sql::raw("1 = 0"),
                }
            }
        });
    Sql::and([
        Some(Sql::bind("\"workspaceId\" = ", input.workspace_id)),
        folder,
        input
            .name
            .as_deref()
            .filter(|n| !n.is_empty())
            .map(|n| Sql::bind("ilike(\"name\", ", like_contains(n)).closed(")")),
        input
            .active
            .map(|a| Sql::bind("\"active\" = ", i64::from(a))),
    ])
}

/// `listWithCounts`.
pub fn list_with_counts(
    conn: &Connection,
    input: &SequenceListInput,
) -> Result<Vec<SequenceWithCounts>> {
    let p = get_pagination_with_defaults(PaginationInput {
        page: input.page,
        per_page: input.per_page,
    });
    let order = parse_order_by(Sequence::COLUMNS, input.sort.as_deref(), None);
    let mut q = Sql::raw(format!(
        "SELECT {}, (SELECT count(*) FROM \"SequenceStep\" st WHERE st.\"sequenceId\" = s.\"id\"), \
         (SELECT count(*) FROM \"ContactOnSequence\" e WHERE e.\"sequenceId\" = s.\"id\") FROM \"Sequence\" s WHERE ",
        Sequence::columns_sql(Some("s"))
    ));
    q.append(build_where(input));
    q.push(" ");
    q.push(&order_by_sql(&order, Some("s")));
    q.push(" LIMIT ");
    q.push_bind(p.limit);
    q.push(" OFFSET ");
    q.push_bind(p.offset);
    let n = Sequence::COLUMNS.len();
    q.query_map(conn, |r| {
        Ok(SequenceWithCounts {
            sequence: Sequence::from_row(r)?,
            steps_count: r.get(n)?,
            subscribers_count: r.get(n + 1)?,
        })
    })
}

/// `count`.
pub fn count(conn: &Connection, input: &SequenceListInput) -> Result<i64> {
    let mut q = Sql::raw("SELECT count(*) FROM \"Sequence\" WHERE ");
    q.append(build_where(input));
    Ok(q.query_map(conn, |r| r.get::<_, i64>(0))?
        .into_iter()
        .next()
        .unwrap_or(0))
}

/// `findWithSteps`.
pub fn find_with_steps(
    conn: &Connection,
    workspace_id: i64,
    id: i64,
) -> Result<Option<SequenceWithSteps>> {
    let Some(sequence) = super::find_one::<Sequence>(
        conn,
        "\"id\" = ?1 AND \"workspaceId\" = ?2",
        &[&id, &workspace_id],
    )?
    else {
        return Ok(None);
    };
    let steps: Vec<SequenceStep> = super::find_many(
        conn,
        "WHERE \"sequenceId\" = ?1 ORDER BY \"order\" ASC",
        &[&id],
    )?;
    let mut out = Vec::with_capacity(steps.len());
    for step in steps {
        let flow = match step.flow_id {
            Some(f) => super::find_one::<Flow>(conn, "\"id\" = ?1", &[&f])?,
            None => None,
        };
        out.push(StepWithFlow { step, flow });
    }
    Ok(Some(SequenceWithSteps {
        sequence,
        steps: out,
    }))
}

/// `completeActiveEnrollmentBatch`: completes up to `limit` active enrollments of the
/// workspace; the count.
pub fn complete_active_enrollment_batch(
    conn: &Connection,
    workspace_id: i64,
    limit: i64,
) -> Result<usize> {
    let now = now_ms();
    Ok(conn.execute(
        "UPDATE \"ContactOnSequence\" SET \"status\" = 'completed', \"completedAt\" = ?3, \"nextRunAt\" = NULL, \
         \"nextStepId\" = NULL, \"updatedAt\" = ?3 WHERE \"id\" IN (SELECT \"id\" FROM \"ContactOnSequence\" \
         WHERE \"workspaceId\" = ?1 AND \"status\" = 'active' LIMIT ?2)",
        rusqlite::params![workspace_id, limit, now],
    )?)
}
