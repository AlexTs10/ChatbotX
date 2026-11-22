//! `questionnaire/service.ts` (`questionnaireService`) and the read / stats /
//! delete half of `submission-service.ts` ([`submission`]). No template locks
//! (`withTemplateLockedTransaction`, `assertResourceEditable`,
//! `assertContentCopyable`, `withDeleteGuard`): DECISIONS 2026-11-13.
//! `startOrResume`, `answerCurrent` and `conversationService` belong to the
//! flow runtime unit.

pub mod flow;
pub mod submission;

use std::collections::{HashMap, HashSet};

use chatbotx_core::utils::id::is_numeric_id;
use chatbotx_db::Db;
use chatbotx_db::repo::models::{Questionnaire, QuestionnaireQuestion};
use chatbotx_db::repo::small_repos::questionnaire_list_names_with_prefix;
use chatbotx_db::repo::{
    Model, PaginationInput, SortItem, get_pagination_with_defaults, is_unique_violation_error,
    like_contains, new_id, now_ms,
};
use chatbotx_db::rusqlite::types::Value as Sql;
use chatbotx_db::rusqlite::{Connection, OptionalExtension, params, params_from_iter};
use serde_json::{Value, json};

use crate::copy_name::{
    CopyNameOptions, CopyNameStyle, copy_name_prefix, insert_with_free_copy_name,
};
use crate::errors::{BusinessError, Result};
use crate::tx;

/// `QUESTIONNAIRE_COPY_NAME`.
pub const QUESTIONNAIRE_COPY_NAME: CopyNameOptions = CopyNameOptions {
    style: CopyNameStyle::Dash,
    max_length: 255,
};
const NAME_KEY: &str = "Questionnaire_workspaceId_name_key";
/// `supportedQuestionnaireQuestionTypes`.
pub const SUPPORTED_QUESTION_TYPES: &[&str] =
    &["text", "number", "email", "phone", "multipleChoice"];

fn db_error(error: chatbotx_db::rusqlite::Error) -> BusinessError {
    BusinessError::Repo(error.into())
}

fn not_found() -> BusinessError {
    BusinessError::not_found("Questionnaire not found")
}

fn name_exists() -> BusinessError {
    BusinessError::exception(
        "Questionnaire name already exists",
        "nameAlreadyExists",
        409,
    )
}

/// `throwMappedUniqueError`.
fn map_unique(error: chatbotx_db::rusqlite::Error) -> BusinessError {
    let error = chatbotx_db::repo::Error::from(error);
    if is_unique_violation_error(&error, None) {
        name_exists()
    } else {
        BusinessError::Repo(error)
    }
}

/// `SYSTEM_FIELDS_BY_QUESTION_TYPE`.
fn system_fields_for(kind: &str) -> &'static [&'static str] {
    match kind {
        "text" | "multipleChoice" => &["first_name", "last_name", "full_name"],
        "email" => &["email"],
        "phone" => &["phone"],
        _ => &[],
    }
}

/// `isRichSystemContactField`: the system contact fields a question can write.
pub const RICH_SYSTEM_CONTACT_FIELDS: &[&str] = &[
    "phone",
    "phone_number",
    "email",
    "full_name",
    "first_name",
    "last_name",
];

/// `isRichSystemContactField`.
pub fn is_rich_system_contact_field(key: &str) -> bool {
    RICH_SYSTEM_CONTACT_FIELDS.contains(&key)
}

/// One question of `update` (`questionnaireQuestionRequest`, parsed).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct QuestionInput {
    pub id: Option<i64>,
    pub title: String,
    pub r#type: String,
    pub active: bool,
    /// `{ mode, url }` or `None`.
    pub image: Option<Value>,
    /// Default 1.
    pub point: Option<i64>,
    pub retry_message: Option<String>,
    pub custom_field_id: Option<String>,
    pub system_field_key: Option<String>,
    /// `{ options: [{ id, label, points }] }` or `None`.
    pub config: Option<Value>,
}

/// `UpdateQuestionnaireInput`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct UpdateQuestionnaire {
    pub workspace_id: i64,
    pub id: i64,
    pub trigger_flow_id: Option<i64>,
    pub enable_score: bool,
    pub enable_retry_messages: bool,
    pub enable_custom_field_mapping: bool,
    pub questions: Vec<QuestionInput>,
}

/// `list` input.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ListQuestionnaires {
    pub workspace_id: i64,
    pub page: Option<i64>,
    /// Default 10.
    pub per_page: Option<i64>,
    pub name: Option<String>,
    pub sort: Option<Vec<SortItem>>,
}

/// One listed questionnaire with its submission count.
#[derive(Debug, Clone, PartialEq)]
pub struct QuestionnaireListRow {
    pub questionnaire: Questionnaire,
    pub applicants_count: i64,
}

/// `list` result.
#[derive(Debug, Clone, PartialEq)]
pub struct QuestionnairePage {
    pub data: Vec<QuestionnaireListRow>,
    pub page_count: i64,
}

/// `getForEdit`: the questionnaire and its live questions by `orderNo`.
#[derive(Debug, Clone, PartialEq)]
pub struct QuestionnaireForEdit {
    pub questionnaire: Questionnaire,
    pub questions: Vec<QuestionnaireQuestion>,
}

pub(crate) fn find_in(
    conn: &Connection,
    workspace_id: i64,
    id: i64,
) -> Result<Option<Questionnaire>> {
    conn.query_row(
        &format!(
            r#"SELECT {} FROM "Questionnaire" WHERE "id" = ?1 AND "workspaceId" = ?2 AND "deletedAt" IS NULL"#,
            Questionnaire::columns_sql(None)
        ),
        params![id, workspace_id],
        Questionnaire::from_row,
    )
    .optional()
    .map_err(db_error)
}

fn live_questions(conn: &Connection, questionnaire_id: i64) -> Result<Vec<QuestionnaireQuestion>> {
    let mut stmt = conn
        .prepare(&format!(
            r#"SELECT {} FROM "QuestionnaireQuestion" WHERE "questionnaireId" = ?1 AND "deletedAt" IS NULL ORDER BY "orderNo", "id""#,
            QuestionnaireQuestion::columns_sql(None)
        ))
        .map_err(db_error)?;
    let rows = stmt
        .query_map(params![questionnaire_id], QuestionnaireQuestion::from_row)
        .map_err(db_error)?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(db_error)?;
    Ok(rows)
}

/// `findBy`: a live questionnaire of the workspace.
pub async fn find_by(db: &Db, workspace_id: i64, id: i64) -> Result<Option<Questionnaire>> {
    tx::read(db, move |conn| find_in(conn, workspace_id, id)).await
}

/// `findByOrFail` (`Questionnaire not found`).
pub async fn find_by_or_fail(db: &Db, workspace_id: i64, id: i64) -> Result<Questionnaire> {
    find_by(db, workspace_id, id).await?.ok_or_else(not_found)
}

/// `list`: by name (asc unless `sort` asks `name` desc), with the submission
/// count.
pub async fn list(db: &Db, input: ListQuestionnaires) -> Result<QuestionnairePage> {
    tx::read(db, move |conn| {
        let per_page = input.per_page.unwrap_or(10);
        let pagination = get_pagination_with_defaults(PaginationInput {
            page: input.page,
            per_page: Some(per_page),
        });
        let name = input.name.as_deref().filter(|n| !n.is_empty()).map(like_contains);
        let desc = input
            .sort
            .as_ref()
            .and_then(|sort| sort.iter().find(|item| item.id == "name"))
            .is_some_and(|item| item.desc);
        let where_sql = r#"q."workspaceId" = ?1 AND q."deletedAt" IS NULL AND (?2 IS NULL OR ilike(q."name", ?2))"#;
        let total: i64 = conn
            .query_row(
                &format!(r#"SELECT COUNT(*) FROM "Questionnaire" q WHERE {where_sql}"#),
                params![input.workspace_id, name],
                |r| r.get(0),
            )
            .map_err(db_error)?;
        let mut stmt = conn
            .prepare(&format!(
                r#"SELECT {}, (SELECT COUNT(*) FROM "QuestionnaireSubmission" s WHERE s."questionnaireId" = q."id")
                   FROM "Questionnaire" q WHERE {where_sql}
                   ORDER BY q."name" {} LIMIT ?3 OFFSET ?4"#,
                Questionnaire::columns_sql(Some("q")),
                if desc { "DESC" } else { "ASC" }
            ))
            .map_err(db_error)?;
        let data = stmt
            .query_map(
                params![input.workspace_id, name, pagination.limit, pagination.offset],
                |r| {
                    Ok(QuestionnaireListRow {
                        questionnaire: Questionnaire::from_row(r)?,
                        applicants_count: r.get(Questionnaire::COLUMNS.len())?,
                    })
                },
            )
            .map_err(db_error)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(db_error)?;
        let page_count = if per_page > 0 { (total + per_page - 1) / per_page } else { 0 };
        Ok(QuestionnairePage { data, page_count })
    })
    .await
}

/// `listForFlow`: `(id, name)` of the live questionnaires by name.
pub async fn list_for_flow(
    db: &Db,
    workspace_id: i64,
    keyword: Option<String>,
) -> Result<Vec<(i64, String)>> {
    tx::read(db, move |conn| {
        let keyword = keyword.as_deref().filter(|k| !k.is_empty()).map(like_contains);
        let mut stmt = conn
            .prepare(
                r#"SELECT "id", "name" FROM "Questionnaire"
                   WHERE "workspaceId" = ?1 AND "deletedAt" IS NULL AND (?2 IS NULL OR ilike("name", ?2))
                   ORDER BY "name""#,
            )
            .map_err(db_error)?;
        let rows = stmt
            .query_map(params![workspace_id, keyword], |r| Ok((r.get(0)?, r.get(1)?)))
            .map_err(db_error)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(db_error)?;
        Ok(rows)
    })
    .await
}

/// `getForEdit`.
pub async fn get_for_edit(db: &Db, workspace_id: i64, id: i64) -> Result<QuestionnaireForEdit> {
    tx::read(db, move |conn| {
        let questionnaire = find_in(conn, workspace_id, id)?.ok_or_else(not_found)?;
        let questions = live_questions(conn, questionnaire.id)?;
        Ok(QuestionnaireForEdit {
            questionnaire,
            questions,
        })
    })
    .await
}

/// `create`: the new id (the name is trimmed).
pub async fn create(db: &Db, workspace_id: i64, name: &str) -> Result<i64> {
    let name = chatbotx_core::js::trim(name).to_string();
    tx::write(db, move |conn| {
        let id = new_id();
        conn.execute(
            r#"INSERT INTO "Questionnaire"
                 ("id", "workspaceId", "name", "triggerFlowId", "enableScore", "enableRetryMessages", "enableCustomFieldMapping")
               VALUES (?1, ?2, ?3, NULL, 0, 0, 1)"#,
            params![id, workspace_id, name],
        )
        .map_err(map_unique)?;
        Ok(id)
    })
    .await
}

/// `rename`.
pub async fn rename(db: &Db, workspace_id: i64, id: i64, name: &str) -> Result<()> {
    let name = chatbotx_core::js::trim(name).to_string();
    tx::write(db, move |conn| {
        crate::template::guard::assert_content_editable(
            conn,
            workspace_id,
            "questionnaire",
            &[id],
        )?;
        let changed = conn
            .execute(
                r#"UPDATE "Questionnaire" SET "name" = ?1, "updatedAt" = ?2
                   WHERE "id" = ?3 AND "workspaceId" = ?4 AND "deletedAt" IS NULL"#,
                params![name, now_ms(), id, workspace_id],
            )
            .map_err(map_unique)?;
        if changed == 0 {
            return Err(not_found());
        }
        Ok(())
    })
    .await
}

/// `normalizeQuestionImage`: `None` for a missing or blank URL.
fn normalize_image(image: Option<&Value>) -> Option<Value> {
    let image = image?;
    let url = chatbotx_core::js::trim(image.get("url")?.as_str()?);
    if url.is_empty() {
        return None;
    }
    Some(json!({ "mode": image.get("mode").cloned().unwrap_or(Value::Null), "url": url }))
}

fn json_text(value: Option<&Value>) -> Option<String> {
    value.filter(|v| !v.is_null()).map(Value::to_string)
}

/// `duplicate`: the copy's id (`<name> - Copy`, `<name> - Copy 2`, ...).
pub async fn duplicate(db: &Db, workspace_id: i64, id: i64) -> Result<i64> {
    tx::write(db, move |conn| {
        let source = find_in(conn, workspace_id, id)?.ok_or_else(not_found)?;
        let questions = live_questions(conn, source.id)?;
        let copied_id = new_id();
        let prefix = copy_name_prefix(&source.name, QUESTIONNAIRE_COPY_NAME);
        let taken: HashSet<String> = questionnaire_list_names_with_prefix(conn, workspace_id, &prefix)
            .map_err(BusinessError::Repo)?
            .into_iter()
            .collect();
        // A failed INSERT rolls back only its own statement in SQLite, so the
        // next candidate needs no savepoint.
        let inserted = insert_with_free_copy_name(
            &source.name,
            &taken,
            QUESTIONNAIRE_COPY_NAME,
            Some(NAME_KEY),
            |error: &chatbotx_db::repo::Error, constraint| is_unique_violation_error(error, constraint),
            |name| {
                conn.execute(
                    r#"INSERT INTO "Questionnaire"
                         ("id", "workspaceId", "name", "triggerFlowId", "enableScore", "enableRetryMessages", "enableCustomFieldMapping")
                       VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)"#,
                    params![
                        copied_id,
                        workspace_id,
                        name,
                        source.trigger_flow_id,
                        source.enable_score,
                        source.enable_retry_messages,
                        source.enable_custom_field_mapping
                    ],
                )
                .map_err(chatbotx_db::repo::Error::from)
            },
        )
        .map_err(BusinessError::Repo)?;
        if inserted.is_none() {
            return Err(name_exists());
        }
        for question in &questions {
            conn.execute(
                r#"INSERT INTO "QuestionnaireQuestion"
                     ("id", "questionnaireId", "title", "type", "active", "image", "orderNo", "point", "retryMessage",
                      "customFieldId", "systemFieldKey", "config")
                   VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)"#,
                params![
                    new_id(),
                    copied_id,
                    question.title,
                    question.r#type,
                    question.active,
                    json_text(normalize_image(question.image.as_ref().map(|j| &j.0)).as_ref()),
                    question.order_no,
                    question.point,
                    question.retry_message,
                    question.custom_field_id,
                    question.system_field_key,
                    json_text(question.config.as_ref().map(|j| &j.0)),
                ],
            )
            .map_err(map_unique)?;
        }
        Ok(copied_id)
    })
    .await
}

/// `deleteInTx`: cancels the open submissions, soft-deletes.
pub fn delete_in_tx(conn: &Connection, workspace_id: i64, ids: &[i64]) -> Result<()> {
    crate::template::guard::assert_deletable(conn, workspace_id, "questionnaire", ids)?;
    if ids.is_empty() {
        return Ok(());
    }
    let now = now_ms();
    let marks = (0..ids.len())
        .map(|i| format!("?{}", i + 3))
        .collect::<Vec<_>>()
        .join(", ");
    let mut args: Vec<Sql> = vec![Sql::Integer(now), Sql::Integer(workspace_id)];
    args.extend(ids.iter().map(|id| Sql::Integer(*id)));
    conn.execute(
        &format!(
            r#"UPDATE "QuestionnaireSubmission"
               SET "status" = 'cancelled', "cancelledAt" = ?1, "currentQuestionId" = NULL, "currentQuestionSentAt" = NULL, "updatedAt" = ?1
               WHERE "workspaceId" = ?2 AND "questionnaireId" IN ({marks}) AND "status" = 'inProgress'"#
        ),
        params_from_iter(args.iter()),
    )
    .map_err(db_error)?;
    conn.execute(
        &format!(
            r#"UPDATE "Questionnaire" SET "deletedAt" = ?1, "updatedAt" = ?1
               WHERE "workspaceId" = ?2 AND "id" IN ({marks}) AND "deletedAt" IS NULL"#
        ),
        params_from_iter(args.iter()),
    )
    .map_err(db_error)?;
    Ok(())
}

/// `deleteMany`: cancels open submissions, soft-deletes.
pub async fn delete_many(db: &Db, workspace_id: i64, ids: Vec<i64>) -> Result<()> {
    if ids.is_empty() {
        return Ok(());
    }
    tx::write(db, move |conn| delete_in_tx(conn, workspace_id, &ids)).await
}

/// `normalizeFieldMapping`: `(customFieldId, systemFieldKey)`.
fn normalize_field_mapping(
    enabled: bool,
    question: &QuestionInput,
) -> (Option<i64>, Option<String>) {
    if !enabled {
        return (None, None);
    }
    let pick = |value: &Option<String>| {
        value
            .as_deref()
            .map(chatbotx_core::js::trim)
            .filter(|v| !v.is_empty())
            .map(str::to_string)
    };
    let Some(key) = pick(&question.system_field_key).or_else(|| pick(&question.custom_field_id))
    else {
        return (None, None);
    };
    if is_rich_system_contact_field(&key) {
        return (None, Some(key));
    }
    if !is_numeric_id(&key) {
        return (None, None);
    }
    (key.parse().ok(), None)
}

fn bad_request(message: &str, code: &str) -> BusinessError {
    BusinessError::exception(message, code, 400)
}

/// `validateEditInput`.
fn validate_edit_input(conn: &Connection, input: &UpdateQuestionnaire) -> Result<()> {
    if input.questions.len() > 100 {
        return Err(bad_request(
            "Questionnaire can have at most 100 questions",
            "tooManyQuestions",
        ));
    }
    if !input.questions.iter().any(|q| q.active) {
        return Err(bad_request(
            "Questionnaire requires at least one active question",
            "noActiveQuestions",
        ));
    }
    let mappings: Vec<_> = input
        .questions
        .iter()
        .map(|q| normalize_field_mapping(input.enable_custom_field_mapping, q))
        .collect();
    let mut known: HashSet<i64> = HashSet::new();
    for (field_id, _) in &mappings {
        if let Some(field_id) = field_id {
            let exists: bool = conn
                .query_row(
                    r#"SELECT EXISTS (SELECT 1 FROM "CustomField" WHERE "id" = ?1 AND "workspaceId" = ?2)"#,
                    params![field_id, input.workspace_id],
                    |r| r.get(0),
                )
                .map_err(db_error)?;
            if exists {
                known.insert(*field_id);
            }
        }
    }
    for (question, (field_id, system_key)) in input.questions.iter().zip(&mappings) {
        if !SUPPORTED_QUESTION_TYPES.contains(&question.r#type.as_str()) {
            return Err(bad_request(
                "Unsupported question type",
                "unsupportedQuestionType",
            ));
        }
        if question.r#type == "multipleChoice" {
            let labels: Vec<String> = question
                .config
                .as_ref()
                .and_then(|c| c.get("options"))
                .and_then(Value::as_array)
                .map(|options| {
                    options
                        .iter()
                        .map(|o| {
                            chatbotx_core::js::trim(
                                o.get("label").and_then(Value::as_str).unwrap_or(""),
                            )
                            .to_string()
                        })
                        .collect()
                })
                .unwrap_or_default();
            if labels.len() < 2 {
                return Err(bad_request(
                    "Multiple choice questions require at least two options",
                    "invalidOptions",
                ));
            }
            if labels.iter().collect::<HashSet<_>>().len() != labels.len() {
                return Err(bad_request(
                    "Multiple choice option labels must be unique",
                    "invalidOptions",
                ));
            }
        }
        if input.enable_custom_field_mapping {
            if let Some(field_id) = field_id
                && !known.contains(field_id)
            {
                return Err(bad_request(
                    "Custom field does not exist in the workspace",
                    "invalidCustomField",
                ));
            }
            if let Some(key) = system_key
                && !system_fields_for(&question.r#type).contains(&key.as_str())
            {
                return Err(bad_request(
                    "System field does not match question type",
                    "invalidSystemField",
                ));
            }
        }
    }
    Ok(())
}

/// `update`: settings and the question list (in order).
pub async fn update(db: &Db, input: UpdateQuestionnaire) -> Result<()> {
    let checked = input.clone();
    tx::read(db, move |conn| validate_edit_input(conn, &checked)).await?;
    tx::write(db, move |conn| {
        crate::template::guard::assert_content_editable(conn, input.workspace_id, "questionnaire", &[input.id])?;
        let questionnaire = find_in(conn, input.workspace_id, input.id)?.ok_or_else(not_found)?;
        if let Some(flow_id) = input.trigger_flow_id {
            let exists: bool = conn
                .query_row(
                    r#"SELECT EXISTS (SELECT 1 FROM "Flow" WHERE "id" = ?1 AND "workspaceId" = ?2 AND "active" = 1)"#,
                    params![flow_id, input.workspace_id],
                    |r| r.get(0),
                )
                .map_err(db_error)?;
            if !exists {
                return Err(BusinessError::not_found("Flow not found"));
            }
        }
        let now = now_ms();
        conn.execute(
            r#"UPDATE "Questionnaire"
               SET "triggerFlowId" = ?1, "enableScore" = ?2, "enableRetryMessages" = ?3, "enableCustomFieldMapping" = ?4, "updatedAt" = ?5
               WHERE "id" = ?6"#,
            params![
                input.trigger_flow_id,
                input.enable_score,
                input.enable_retry_messages,
                input.enable_custom_field_mapping,
                now,
                questionnaire.id
            ],
        )
        .map_err(map_unique)?;
        let existing: Vec<i64> = live_questions(conn, questionnaire.id)?.iter().map(|q| q.id).collect();
        let submitted: HashSet<i64> = input.questions.iter().filter_map(|q| q.id).collect();
        for removed in existing.iter().filter(|id| !submitted.contains(id)) {
            let answers: i64 = conn
                .query_row(
                    r#"SELECT COUNT(*) FROM "QuestionnaireAnswer" WHERE "questionId" = ?1"#,
                    params![removed],
                    |r| r.get(0),
                )
                .map_err(db_error)?;
            if answers > 0 {
                conn.execute(
                    r#"UPDATE "QuestionnaireQuestion" SET "deletedAt" = ?1, "updatedAt" = ?1 WHERE "id" = ?2"#,
                    params![now, removed],
                )
                .map_err(db_error)?;
            } else {
                conn.execute(r#"DELETE FROM "QuestionnaireQuestion" WHERE "id" = ?1"#, params![removed])
                    .map_err(db_error)?;
            }
        }
        for (index, question) in input.questions.iter().enumerate() {
            let (field_id, system_key) = normalize_field_mapping(input.enable_custom_field_mapping, question);
            let retry = if input.enable_retry_messages {
                question.retry_message.as_deref().map(|m| chatbotx_core::js::trim(m).to_string())
            } else {
                None
            };
            let config = if question.r#type == "multipleChoice" {
                Some(question.config.clone().filter(|c| !c.is_null()).unwrap_or_else(|| json!({ "options": [] })))
            } else {
                None
            };
            let image = json_text(normalize_image(question.image.as_ref()).as_ref());
            let title = chatbotx_core::js::trim(&question.title).to_string();
            let point = question.point.unwrap_or(1);
            let order = i64::try_from(index).unwrap_or(i64::MAX);
            match question.id {
                Some(question_id) => {
                    conn.execute(
                        r#"UPDATE "QuestionnaireQuestion"
                           SET "title" = ?1, "type" = ?2, "active" = ?3, "image" = ?4, "orderNo" = ?5, "point" = ?6,
                               "retryMessage" = ?7, "customFieldId" = ?8, "systemFieldKey" = ?9, "config" = ?10, "updatedAt" = ?11
                           WHERE "id" = ?12 AND "questionnaireId" = ?13 AND "deletedAt" IS NULL"#,
                        params![
                            title,
                            question.r#type,
                            question.active,
                            image,
                            order,
                            point,
                            retry,
                            field_id,
                            system_key,
                            json_text(config.as_ref()),
                            now,
                            question_id,
                            questionnaire.id
                        ],
                    )
                    .map_err(db_error)?;
                }
                None => {
                    conn.execute(
                        r#"INSERT INTO "QuestionnaireQuestion"
                             ("id", "questionnaireId", "title", "type", "active", "image", "orderNo", "point", "retryMessage",
                              "customFieldId", "systemFieldKey", "config")
                           VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)"#,
                        params![
                            new_id(),
                            questionnaire.id,
                            title,
                            question.r#type,
                            question.active,
                            image,
                            order,
                            point,
                            retry,
                            field_id,
                            system_key,
                            json_text(config.as_ref())
                        ],
                    )
                    .map_err(db_error)?;
                }
            }
        }
        Ok(())
    })
    .await
}

/// `getQuestionnaireNameMap`.
pub async fn get_questionnaire_name_map(
    db: &Db,
    workspace_id: i64,
    ids: Vec<i64>,
) -> Result<HashMap<i64, String>> {
    if ids.is_empty() {
        return Ok(HashMap::new());
    }
    tx::read(db, move |conn| {
        let marks = (0..ids.len()).map(|i| format!("?{}", i + 2)).collect::<Vec<_>>().join(", ");
        let mut args: Vec<Sql> = vec![Sql::Integer(workspace_id)];
        args.extend(ids.iter().map(|id| Sql::Integer(*id)));
        let mut stmt = conn
            .prepare(&format!(
                r#"SELECT "id", "name" FROM "Questionnaire" WHERE "workspaceId" = ?1 AND "id" IN ({marks})"#
            ))
            .map_err(db_error)?;
        let rows = stmt
            .query_map(params_from_iter(args.iter()), |r| Ok((r.get(0)?, r.get(1)?)))
            .map_err(db_error)?
            .collect::<std::result::Result<HashMap<_, _>, _>>()
            .map_err(db_error)?;
        Ok(rows)
    })
    .await
}
