//! The flow-runtime half of `questionnaire/submission-service.ts`
//! (`startOrResume`, `markQuestionSent`, `answerCurrent`, `cancel`,
//! `deleteApplicant`). SQLite has one writer, so the `FOR UPDATE` of
//! `answerCurrent` is the write transaction itself.

use chatbotx_core::js;
use chatbotx_db::Db;
use chatbotx_db::repo::models::{Questionnaire, QuestionnaireQuestion, QuestionnaireSubmission};
use chatbotx_db::repo::{Model, is_unique_violation_error, new_id, now_ms};
use chatbotx_db::rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Value, json};

use crate::errors::{BusinessError, Result};
use crate::get_user_data::{EMAIL_PATTERN, PHONE_PATTERN};
use crate::tx;

/// `MAX_INVALID_ATTEMPTS`.
pub const MAX_INVALID_ATTEMPTS: i64 = 3;
/// `MAX_ANSWER_TEXT_LENGTH`.
const MAX_ANSWER_TEXT_LENGTH: usize = 1000;

fn db_error(error: chatbotx_db::rusqlite::Error) -> BusinessError {
    BusinessError::Repo(error.into())
}

/// `startOrResume` outcome.
#[derive(Debug, Clone, PartialEq)]
pub enum Started {
    Skip(&'static str),
    Wait {
        submission_id: i64,
        question: Box<QuestionnaireQuestion>,
    },
}

/// `answerCurrent` outcome.
#[derive(Debug, Clone, PartialEq)]
pub enum Answered {
    Skip(&'static str),
    Wait {
        submission_id: i64,
        question: QuestionnaireQuestion,
        sent_at: i64,
    },
    Retry {
        submission_id: i64,
        question: QuestionnaireQuestion,
        attempts: i64,
        retry_message: Option<String>,
        reason: &'static str,
    },
    Completed {
        submission_id: i64,
        trigger_flow_id: Option<i64>,
        completed_at: i64,
    },
}

/// `answerCurrent` input.
#[derive(Debug, Clone, Default)]
pub struct AnswerInput {
    pub workspace_id: i64,
    pub contact_id: i64,
    pub conversation_id: i64,
    pub questionnaire_id: i64,
    pub raw_text: String,
    pub attempts: i64,
    pub trigger_message_id: Option<i64>,
}

fn questionnaire_in(
    conn: &Connection,
    workspace_id: i64,
    id: i64,
) -> Result<Option<Questionnaire>> {
    super::find_in(conn, workspace_id, id)
}

/// The active, live questions in `orderNo` order.
fn active_questions(
    conn: &Connection,
    questionnaire_id: i64,
) -> Result<Vec<QuestionnaireQuestion>> {
    let mut stmt = conn
        .prepare(&format!(
            r#"SELECT {} FROM "QuestionnaireQuestion"
               WHERE "questionnaireId" = ?1 AND "active" = 1 AND "deletedAt" IS NULL
               ORDER BY "orderNo", "id""#,
            QuestionnaireQuestion::columns_sql(None)
        ))
        .map_err(db_error)?;
    stmt.query_map(params![questionnaire_id], QuestionnaireQuestion::from_row)
        .map_err(db_error)?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(db_error)
}

fn question_in(
    conn: &Connection,
    questionnaire_id: i64,
    id: i64,
) -> Result<Option<QuestionnaireQuestion>> {
    conn.query_row(
        &format!(
            r#"SELECT {} FROM "QuestionnaireQuestion"
               WHERE "id" = ?1 AND "questionnaireId" = ?2 AND "deletedAt" IS NULL"#,
            QuestionnaireQuestion::columns_sql(None)
        ),
        params![id, questionnaire_id],
        QuestionnaireQuestion::from_row,
    )
    .optional()
    .map_err(db_error)
}

fn submission_where(
    conn: &Connection,
    condition: &str,
    args: &[&dyn chatbotx_db::rusqlite::ToSql],
) -> Result<Option<QuestionnaireSubmission>> {
    conn.query_row(
        &format!(
            r#"SELECT {} FROM "QuestionnaireSubmission" WHERE {condition} ORDER BY "createdAt", "id" LIMIT 1"#,
            QuestionnaireSubmission::columns_sql(None)
        ),
        args,
        QuestionnaireSubmission::from_row,
    )
    .optional()
    .map_err(db_error)
}

/// `startOrResume`: restarts a cancelled submission, resumes the in-progress
/// one (moving it to this conversation), or starts one at the first question.
pub async fn start_or_resume(
    db: &Db,
    workspace_id: i64,
    questionnaire_id: i64,
    contact_id: i64,
    conversation_id: Option<i64>,
) -> Result<Started> {
    tx::write(db, move |conn| {
        let Some(questionnaire) = questionnaire_in(conn, workspace_id, questionnaire_id)? else {
            return Ok(Started::Skip("questionnaire_not_available"));
        };
        let questions = active_questions(conn, questionnaire_id)?;
        let Some(first) = questions.first().cloned() else {
            return Ok(Started::Skip("questionnaire_empty"));
        };
        let score = questionnaire.enable_score.then_some(0_i64);
        let existing = submission_where(
            conn,
            r#""workspaceId" = ?1 AND "questionnaireId" = ?2 AND "contactId" = ?3"#,
            params![workspace_id, questionnaire_id, contact_id],
        )?;
        if let Some(existing) = existing {
            if existing.status == "cancelled" {
                let sent_at = now_ms();
                conn.execute(
                    r#"DELETE FROM "QuestionnaireAnswer" WHERE "submissionId" = ?1"#,
                    [existing.id],
                )?;
                conn.execute(
                    r#"UPDATE "QuestionnaireSubmission" SET "conversationId" = ?1, "status" = 'inProgress',
                         "totalPoints" = ?2, "currentQuestionId" = ?3, "currentQuestionSentAt" = ?4,
                         "lastAnsweredMessageId" = NULL, "startedAt" = ?4, "completedAt" = NULL,
                         "cancelledAt" = NULL, "updatedAt" = ?4
                       WHERE "id" = ?5 AND "workspaceId" = ?6 AND "status" = 'cancelled'"#,
                    params![conversation_id, score, first.id, sent_at, existing.id, workspace_id],
                )?;
                return Ok(Started::Wait {
                    submission_id: existing.id,
                    question: Box::new(first),
                });
            }
            if existing.status != "inProgress" {
                return Ok(Started::Skip("questionnaire_already_submitted"));
            }
            let Some(current) = questions
                .iter()
                .find(|q| Some(q.id) == existing.current_question_id)
            else {
                return Ok(Started::Skip("questionnaire_current_question_missing"));
            };
            if existing.conversation_id != conversation_id {
                conn.execute(
                    r#"UPDATE "QuestionnaireSubmission" SET "conversationId" = ?1, "updatedAt" = ?2
                       WHERE "id" = ?3 AND "workspaceId" = ?4 AND "status" = 'inProgress'"#,
                    params![conversation_id, now_ms(), existing.id, workspace_id],
                )?;
            }
            return Ok(Started::Wait {
                submission_id: existing.id,
                question: Box::new(current.clone()),
            });
        }
        let active = submission_where(
            conn,
            r#""workspaceId" = ?1 AND "contactId" = ?2 AND "status" = 'inProgress'"#,
            params![workspace_id, contact_id],
        )?;
        if let Some(active) = active {
            return Ok(Started::Skip(if active.questionnaire_id == questionnaire_id {
                "questionnaire_already_submitted"
            } else {
                "contact_has_other_active_questionnaire"
            }));
        }
        let id = new_id();
        let inserted = conn.execute(
            r#"INSERT INTO "QuestionnaireSubmission" ("id", "workspaceId", "questionnaireId", "contactId",
                 "conversationId", "status", "totalPoints", "currentQuestionId", "currentQuestionSentAt")
               VALUES (?1, ?2, ?3, ?4, ?5, 'inProgress', ?6, ?7, ?8)"#,
            params![id, workspace_id, questionnaire_id, contact_id, conversation_id, score, first.id, now_ms()],
        );
        match inserted {
            Ok(_) => Ok(Started::Wait {
                submission_id: id,
                question: Box::new(first),
            }),
            Err(e) => {
                let e = chatbotx_db::repo::Error::from(e);
                if is_unique_violation_error(&e, None) {
                    Ok(Started::Skip("already_has_active_submission"))
                } else {
                    Err(BusinessError::Repo(e))
                }
            }
        }
    })
    .await
}

/// `markQuestionSent`.
pub async fn mark_question_sent(
    db: &Db,
    workspace_id: i64,
    submission_id: i64,
    question_id: i64,
    sent_at: i64,
) -> Result<()> {
    tx::write(db, move |conn| {
        conn.execute(
            r#"UPDATE "QuestionnaireSubmission" SET "currentQuestionId" = ?1, "currentQuestionSentAt" = ?2, "updatedAt" = ?5
               WHERE "id" = ?3 AND "workspaceId" = ?4 AND "status" = 'inProgress'"#,
            params![question_id, sent_at, submission_id, workspace_id, now_ms()],
        )?;
        Ok(())
    })
    .await
}

/// `cancel`: the contact's in-progress submission of the questionnaire.
pub fn cancel_in(
    conn: &Connection,
    workspace_id: i64,
    questionnaire_id: i64,
    contact_id: i64,
) -> Result<()> {
    let now = now_ms();
    conn.execute(
        r#"UPDATE "QuestionnaireSubmission" SET "status" = 'cancelled', "cancelledAt" = ?1,
             "currentQuestionId" = NULL, "currentQuestionSentAt" = NULL, "updatedAt" = ?1
           WHERE "workspaceId" = ?2 AND "questionnaireId" = ?3 AND "contactId" = ?4 AND "status" = 'inProgress'"#,
        params![now, workspace_id, questionnaire_id, contact_id],
    )?;
    Ok(())
}

/// `cancel`.
pub async fn cancel(
    db: &Db,
    workspace_id: i64,
    questionnaire_id: i64,
    contact_id: i64,
) -> Result<()> {
    tx::write(db, move |conn| {
        cancel_in(conn, workspace_id, questionnaire_id, contact_id)
    })
    .await
}

/// `deleteApplicant`: every submission of the contact for the questionnaire.
pub async fn delete_applicant(
    db: &Db,
    workspace_id: i64,
    questionnaire_id: i64,
    contact_id: i64,
) -> Result<()> {
    tx::write(db, move |conn| {
        conn.execute(
            r#"DELETE FROM "QuestionnaireSubmission"
               WHERE "workspaceId" = ?1 AND "questionnaireId" = ?2 AND "contactId" = ?3"#,
            params![workspace_id, questionnaire_id, contact_id],
        )?;
        Ok(())
    })
    .await
}

/// A JSON number as `JSON.stringify` writes it (integral values without `.0`).
fn js_number(value: f64) -> Value {
    if value.fract() == 0.0 && value.abs() < 9_007_199_254_740_992.0 {
        json!(value as i64)
    } else {
        json!(value)
    }
}

fn options_of(question: &QuestionnaireQuestion) -> Vec<Value> {
    question
        .config
        .as_ref()
        .and_then(|c| c.0.get("options"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

/// `parseAnswer`: the stored value, or the rejection reason.
pub fn parse_answer(
    question: &QuestionnaireQuestion,
    raw_text: &str,
) -> std::result::Result<Value, &'static str> {
    let value = js::trim(raw_text);
    if value.is_empty() {
        return Err("empty");
    }
    match question.r#type.as_str() {
        "number" => {
            let n = js::to_number(value);
            if n.is_finite() {
                Ok(js_number(n))
            } else {
                Err("invalid_number")
            }
        }
        "email" => EMAIL_PATTERN
            .is_match(value)
            .then(|| json!(value))
            .ok_or("invalid_email"),
        "phone" => PHONE_PATTERN
            .is_match(value)
            .then(|| json!(value))
            .ok_or("invalid_phone"),
        "multipleChoice" => options_of(question)
            .iter()
            .find(|o| o["id"].as_str() == Some(value) || o["label"].as_str() == Some(value))
            .map(|o| json!({ "optionId": o["id"], "label": o["label"] }))
            .ok_or("invalid_option"),
        _ if js::utf16_len(value) > MAX_ANSWER_TEXT_LENGTH => Err("too_long"),
        _ => Ok(json!(value)),
    }
}

/// `calculatePoints`.
fn calculate_points(question: &QuestionnaireQuestion, value: &Value) -> i64 {
    if question.r#type != "multipleChoice" {
        return question.point;
    }
    let option_id = value.get("optionId");
    options_of(question)
        .iter()
        .find(|o| option_id.is_some_and(|id| &o["id"] == id))
        .and_then(|o| o["points"].as_f64())
        .map_or(0, |p| p as i64)
}

/// `answerValueToString`.
pub fn answer_value_to_string(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::Object(o) => o.get("label").map(js::string_of).unwrap_or_default(),
        other => js::string_of(other),
    }
}

fn next_after(questions: &[QuestionnaireQuestion], id: i64) -> Option<QuestionnaireQuestion> {
    let index = questions.iter().position(|q| q.id == id);
    // `findIndex` is -1 when gone, so `questions[0]` comes next.
    questions.get(index.map_or(0, |i| i + 1)).cloned()
}

/// Moves the submission on to `next` (or completes it); `total_points`
/// `Some` also writes the score.
fn advance(
    conn: &Connection,
    submission_id: i64,
    questionnaire: &Questionnaire,
    next: Option<QuestionnaireQuestion>,
    total_points: Option<Option<i64>>,
    trigger_message_id: Option<i64>,
) -> Result<Answered> {
    let now = now_ms();
    let (set_points, points) = (total_points.is_some(), total_points.flatten());
    conn.execute(
        r#"UPDATE "QuestionnaireSubmission" SET
             "totalPoints" = CASE WHEN ?1 THEN ?2 ELSE "totalPoints" END,
             "status" = CASE WHEN ?3 IS NULL THEN 'completed' ELSE "status" END,
             "completedAt" = CASE WHEN ?3 IS NULL THEN ?4 ELSE "completedAt" END,
             "currentQuestionId" = ?3,
             "currentQuestionSentAt" = CASE WHEN ?3 IS NULL THEN NULL ELSE ?4 END,
             "lastAnsweredMessageId" = ?5, "updatedAt" = ?4
           WHERE "id" = ?6"#,
        params![
            set_points,
            points,
            next.as_ref().map(|q| q.id),
            now,
            trigger_message_id,
            submission_id
        ],
    )?;
    Ok(match next {
        None => Answered::Completed {
            submission_id,
            trigger_flow_id: questionnaire.trigger_flow_id,
            completed_at: now,
        },
        Some(question) => Answered::Wait {
            submission_id,
            question,
            sent_at: now,
        },
    })
}

/// `answerCurrent`: judges `raw_text` against the current question of the
/// contact's in-progress submission in this conversation.
pub async fn answer_current(db: &Db, input: AnswerInput) -> Result<Answered> {
    tx::write(db, move |conn| {
        let ws = input.workspace_id;
        let Some(submission) = submission_where(
            conn,
            r#""workspaceId" = ?1 AND "contactId" = ?2 AND "conversationId" = ?3
               AND "questionnaireId" = ?4 AND "status" = 'inProgress'"#,
            params![ws, input.contact_id, input.conversation_id, input.questionnaire_id],
        )?
        else {
            return Ok(Answered::Skip("submission_not_found"));
        };
        let Some(current_id) = submission.current_question_id else {
            return Ok(Answered::Skip("submission_not_found"));
        };
        // A redelivered answer: ask the current question again.
        if input.trigger_message_id.is_some()
            && submission.last_answered_message_id == input.trigger_message_id
        {
            return Ok(match question_in(conn, input.questionnaire_id, current_id)? {
                Some(question) => Answered::Wait {
                    submission_id: submission.id,
                    question,
                    sent_at: submission.current_question_sent_at.unwrap_or_else(now_ms),
                },
                None => Answered::Skip("question_missing"),
            });
        }
        let questionnaire = questionnaire_in(conn, ws, input.questionnaire_id)?;
        let question = question_in(conn, input.questionnaire_id, current_id)?;
        let (Some(questionnaire), Some(question)) = (questionnaire, question) else {
            cancel_in(conn, ws, input.questionnaire_id, input.contact_id)?;
            return Ok(Answered::Skip("question_missing"));
        };

        let value = match parse_answer(&question, &input.raw_text) {
            Ok(value) => value,
            Err(reason) => {
                let attempts = input.attempts + 1;
                if attempts >= MAX_INVALID_ATTEMPTS {
                    let questions = active_questions(conn, input.questionnaire_id)?;
                    let next = next_after(&questions, question.id);
                    return advance(conn, submission.id, &questionnaire, next, None, input.trigger_message_id);
                }
                return Ok(Answered::Retry {
                    submission_id: submission.id,
                    retry_message: question.retry_message.clone(),
                    question,
                    attempts,
                    reason,
                });
            }
        };

        let points = questionnaire.enable_score.then(|| calculate_points(&question, &value));
        let label: String = match question.custom_field_id {
            Some(field) => conn
                .query_row(r#"SELECT "name" FROM "CustomField" WHERE "id" = ?1"#, [field], |r| r.get(0))
                .optional()?
                .unwrap_or_else(|| question.title.clone()),
            None => question.title.clone(),
        };
        let now = now_ms();
        conn.execute(
            r#"INSERT INTO "QuestionnaireAnswer" ("id", "submissionId", "questionId", "questionIdSnapshot",
                 "questionTitleSnapshot", "questionTypeSnapshot", "labelSnapshot", "value", "pointsEarned", "answeredAt")
               VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
               ON CONFLICT ("submissionId", "questionIdSnapshot") DO UPDATE SET
                 "value" = excluded."value", "pointsEarned" = excluded."pointsEarned",
                 "labelSnapshot" = excluded."labelSnapshot", "questionTitleSnapshot" = excluded."questionTitleSnapshot",
                 "questionTypeSnapshot" = excluded."questionTypeSnapshot", "answeredAt" = excluded."answeredAt",
                 "attemptCount" = "attemptCount" + 1, "updatedAt" = excluded."answeredAt""#,
            params![
                new_id(),
                submission.id,
                question.id,
                question.id.to_string(),
                question.title,
                question.r#type,
                label,
                value.to_string(),
                points,
                now
            ],
        )?;
        if questionnaire.enable_custom_field_mapping {
            let text = answer_value_to_string(&value);
            if let Some(field) = question.custom_field_id {
                crate::contact_custom_field::set_values_in_tx(
                    conn,
                    ws,
                    input.contact_id,
                    &[(field, text.clone())],
                    None,
                    now,
                )?;
            }
            if let Some(key) = question
                .system_field_key
                .as_deref()
                .filter(|k| super::is_rich_system_contact_field(k))
            {
                crate::contact::service::update_text_columns(
                    conn,
                    ws,
                    input.contact_id,
                    &crate::facebook_lead_ads::rich_system_columns(key, &text),
                )?;
            }
        }
        let questions = active_questions(conn, input.questionnaire_id)?;
        let next = next_after(&questions, question.id);
        let total = if questionnaire.enable_score {
            let sum: Option<i64> = conn.query_row(
                r#"SELECT SUM("pointsEarned") FROM "QuestionnaireAnswer" WHERE "submissionId" = ?1"#,
                [submission.id],
                |r| r.get(0),
            )?;
            Some(Some(sum.unwrap_or(0)))
        } else {
            Some(None)
        };
        advance(conn, submission.id, &questionnaire, next, total, input.trigger_message_id)
    })
    .await
}
