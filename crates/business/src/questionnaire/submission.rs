//! The read / stats / delete half of `questionnaire/submission-service.ts`
//! (`questionnaireSubmissionService`).

use chatbotx_db::Db;
use chatbotx_db::repo::{PaginationInput, SortItem, get_pagination_with_defaults, like_contains};
use chatbotx_db::rusqlite::{OptionalExtension, Row, params};
use serde_json::Value;

use crate::errors::{BusinessError, Result};
use crate::tx;

fn db_error(error: chatbotx_db::rusqlite::Error) -> BusinessError {
    BusinessError::Repo(error.into())
}

/// `getQuestionnaireSubmissionListOrder` with the Postgres semantics:
/// `ASC` = `NULLS LAST`, `DESC` = `NULLS FIRST`; `status` in the
/// `questionnaireSubmissionStatus` enum order.
pub fn list_order_sql(sort: Option<&[SortItem]>) -> String {
    let Some(item) = sort.and_then(|s| s.first()) else {
        return r#"s."createdAt" DESC"#.to_string();
    };
    let pg = |column: &str| {
        if item.desc {
            format!("{column} DESC NULLS FIRST")
        } else {
            format!("{column} ASC NULLS LAST")
        }
    };
    match item.id.as_str() {
        "name" => pg(r#"c."fullName""#),
        "totalPoints" => pg(r#"s."totalPoints""#),
        "status" => pg(
            r#"CASE s."status" WHEN 'inProgress' THEN 0 WHEN 'completed' THEN 1 WHEN 'cancelled' THEN 2 WHEN 'failed' THEN 3 ELSE 4 END"#,
        ),
        "completedAt" => {
            if item.desc {
                r#"s."completedAt" DESC NULLS LAST"#.to_string()
            } else {
                r#"s."completedAt" ASC NULLS LAST"#.to_string()
            }
        }
        _ => r#"s."createdAt" DESC"#.to_string(),
    }
}

/// The contact columns at `start` (`id`, `fullName`, `firstName`, `lastName`,
/// `email`, `phoneNumber`, `avatar`).
fn contact_at(r: &Row<'_>, start: usize) -> chatbotx_db::rusqlite::Result<SubmissionContact> {
    Ok(SubmissionContact {
        id: r.get(start)?,
        full_name: r.get(start + 1)?,
        first_name: r.get(start + 2)?,
        last_name: r.get(start + 3)?,
        email: r.get(start + 4)?,
        phone_number: r.get(start + 5)?,
        avatar: r.get(start + 6)?,
    })
}

const CONTACT_COLUMNS: &str =
    r#"c."id", c."fullName", c."firstName", c."lastName", c."email", c."phoneNumber", c."avatar""#;

/// `list` input.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ListSubmissions {
    pub workspace_id: i64,
    pub questionnaire_id: i64,
    pub page: Option<i64>,
    /// Default 10.
    pub per_page: Option<i64>,
    pub name: Option<String>,
    pub sort: Option<Vec<SortItem>>,
}

/// The contact columns of a listed submission.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SubmissionContact {
    pub id: i64,
    pub full_name: Option<String>,
    pub first_name: Option<String>,
    pub last_name: Option<String>,
    pub email: Option<String>,
    pub phone_number: Option<String>,
    pub avatar: Option<String>,
}

/// One listed submission.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SubmissionListRow {
    pub id: i64,
    pub status: String,
    pub total_points: Option<i64>,
    /// Unix ms.
    pub completed_at: Option<i64>,
    pub conversation_id: Option<i64>,
    pub contact: SubmissionContact,
}

/// `list` result.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SubmissionPage {
    pub enable_score: bool,
    pub data: Vec<SubmissionListRow>,
    pub page_count: i64,
}

/// `dashboard` result.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SubmissionStats {
    pub total_applicants: i64,
    pub completed: i64,
    /// Rounded percent.
    pub completion_rate: i64,
}

/// One answer of `detail`.
#[derive(Debug, Clone, PartialEq)]
pub struct SubmissionAnswer {
    /// `questionIdSnapshot`.
    pub question_id: String,
    /// `labelSnapshot`.
    pub label: String,
    pub value: Option<Value>,
    pub points_earned: Option<i64>,
}

/// `detail` result.
#[derive(Debug, Clone, PartialEq)]
pub struct SubmissionDetail {
    pub id: i64,
    pub contact: SubmissionContact,
    pub conversation_id: Option<i64>,
    pub status: String,
    pub total_points: Option<i64>,
    pub answers: Vec<SubmissionAnswer>,
}

/// `findLatestCompletedForContact` result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LatestCompleted {
    pub id: i64,
    pub total_points: i64,
}

/// `list` (questionnaire must exist).
pub async fn list(db: &Db, input: ListSubmissions) -> Result<SubmissionPage> {
    tx::read(db, move |conn| {
        let questionnaire = super::find_in(conn, input.workspace_id, input.questionnaire_id)?
            .ok_or_else(|| BusinessError::not_found("Questionnaire not found"))?;
        let per_page = input.per_page.unwrap_or(10);
        let pagination = get_pagination_with_defaults(PaginationInput {
            page: input.page,
            per_page: Some(per_page),
        });
        let name = input.name.as_deref().filter(|n| !n.is_empty()).map(like_contains);
        let from = r#"FROM "QuestionnaireSubmission" s JOIN "Contact" c ON c."id" = s."contactId"
            WHERE s."workspaceId" = ?1 AND s."questionnaireId" = ?2 AND (?3 IS NULL OR ilike(c."fullName", ?3))"#;
        let total: i64 = conn
            .query_row(
                &format!("SELECT COUNT(*) {from}"),
                params![input.workspace_id, input.questionnaire_id, name],
                |r| r.get(0),
            )
            .map_err(db_error)?;
        let mut stmt = conn
            .prepare(&format!(
                r#"SELECT s."id", s."status", s."totalPoints", s."completedAt", s."conversationId", {CONTACT_COLUMNS}
                   {from} ORDER BY {} LIMIT ?4 OFFSET ?5"#,
                list_order_sql(input.sort.as_deref())
            ))
            .map_err(db_error)?;
        let data = stmt
            .query_map(
                params![
                    input.workspace_id,
                    input.questionnaire_id,
                    name,
                    pagination.limit,
                    pagination.offset
                ],
                |r| {
                    Ok(SubmissionListRow {
                        id: r.get(0)?,
                        status: r.get(1)?,
                        total_points: r.get(2)?,
                        completed_at: r.get(3)?,
                        conversation_id: r.get(4)?,
                        contact: contact_at(r, 5)?,
                    })
                },
            )
            .map_err(db_error)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(db_error)?;
        let page_count = if per_page > 0 { (total + per_page - 1) / per_page } else { 0 };
        Ok(SubmissionPage {
            enable_score: questionnaire.enable_score,
            data,
            page_count,
        })
    })
    .await
}

/// `dashboard`: totals and completion rate.
pub async fn dashboard(
    db: &Db,
    workspace_id: i64,
    questionnaire_id: i64,
) -> Result<SubmissionStats> {
    tx::read(db, move |conn| {
        super::find_in(conn, workspace_id, questionnaire_id)?
            .ok_or_else(|| BusinessError::not_found("Questionnaire not found"))?;
        let (total, completed): (i64, i64) = conn
            .query_row(
                r#"SELECT COUNT(*), COUNT(CASE WHEN "status" = 'completed' THEN 1 END)
                   FROM "QuestionnaireSubmission" WHERE "workspaceId" = ?1 AND "questionnaireId" = ?2"#,
                params![workspace_id, questionnaire_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(db_error)?;
        let completion_rate = if total == 0 {
            0
        } else {
            // `Math.round` of a positive ratio.
            ((completed as f64 / total as f64) * 100.0 + 0.5).floor() as i64
        };
        Ok(SubmissionStats {
            total_applicants: total,
            completed,
            completion_rate,
        })
    })
    .await
}

/// `detail` (`Submission not found`).
pub async fn detail(
    db: &Db,
    workspace_id: i64,
    questionnaire_id: i64,
    submission_id: i64,
) -> Result<SubmissionDetail> {
    tx::read(db, move |conn| {
        let head = conn
            .query_row(
                &format!(
                    r#"SELECT s."id", s."conversationId", s."status", s."totalPoints", {CONTACT_COLUMNS}
                       FROM "QuestionnaireSubmission" s JOIN "Contact" c ON c."id" = s."contactId"
                       WHERE s."id" = ?1 AND s."workspaceId" = ?2 AND s."questionnaireId" = ?3"#
                ),
                params![submission_id, workspace_id, questionnaire_id],
                |r| {
                    Ok((
                        r.get::<_, i64>(0)?,
                        r.get::<_, Option<i64>>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, Option<i64>>(3)?,
                        contact_at(r, 4)?,
                    ))
                },
            )
            .optional()
            .map_err(db_error)?
            .ok_or_else(|| BusinessError::not_found("Submission not found"))?;
        let mut stmt = conn
            .prepare(
                r#"SELECT "questionIdSnapshot", "labelSnapshot", "value", "pointsEarned"
                   FROM "QuestionnaireAnswer" WHERE "submissionId" = ?1 ORDER BY "createdAt", "id""#,
            )
            .map_err(db_error)?;
        let answers = stmt
            .query_map(params![submission_id], |r| {
                let value: Option<String> = r.get(2)?;
                Ok(SubmissionAnswer {
                    question_id: r.get(0)?,
                    label: r.get(1)?,
                    value: value.and_then(|text| serde_json::from_str(&text).ok()),
                    points_earned: r.get(3)?,
                })
            })
            .map_err(db_error)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(db_error)?;
        let (id, conversation_id, status, total_points, contact) = head;
        Ok(SubmissionDetail {
            id,
            contact,
            conversation_id,
            status,
            total_points,
            answers,
        })
    })
    .await
}

/// `findLatestCompletedForContact`.
pub async fn find_latest_completed_for_contact(
    db: &Db,
    workspace_id: i64,
    contact_id: i64,
) -> Result<Option<LatestCompleted>> {
    tx::read(db, move |conn| {
        conn.query_row(
            r#"SELECT "id", "totalPoints" FROM "QuestionnaireSubmission"
               WHERE "workspaceId" = ?1 AND "contactId" = ?2 AND "status" = 'completed' AND "totalPoints" IS NOT NULL
               ORDER BY "completedAt" DESC NULLS FIRST LIMIT 1"#,
            params![workspace_id, contact_id],
            |r| {
                Ok(LatestCompleted {
                    id: r.get(0)?,
                    total_points: r.get(1)?,
                })
            },
        )
        .optional()
        .map_err(db_error)
    })
    .await
}

/// `deleteSubmission`.
pub async fn delete_submission(
    db: &Db,
    workspace_id: i64,
    questionnaire_id: i64,
    submission_id: i64,
) -> Result<()> {
    tx::write(db, move |conn| {
        conn.execute(
            r#"DELETE FROM "QuestionnaireSubmission" WHERE "workspaceId" = ?1 AND "questionnaireId" = ?2 AND "id" = ?3"#,
            params![workspace_id, questionnaire_id, submission_id],
        )
        .map_err(db_error)?;
        Ok(())
    })
    .await
}

/// `deleteSubmissions` (an empty list does nothing).
pub async fn delete_submissions(
    db: &Db,
    workspace_id: i64,
    questionnaire_id: i64,
    submission_ids: Vec<i64>,
) -> Result<()> {
    if submission_ids.is_empty() {
        return Ok(());
    }
    let ids = serde_json::to_string(&submission_ids).unwrap_or_default();
    tx::write(db, move |conn| {
        conn.execute(
            r#"DELETE FROM "QuestionnaireSubmission" WHERE "workspaceId" = ?1 AND "questionnaireId" = ?2 AND "id" IN (SELECT value FROM json_each(?3))"#,
            params![workspace_id, questionnaire_id, ids],
        )
        .map_err(db_error)?;
        Ok(())
    })
    .await
}
