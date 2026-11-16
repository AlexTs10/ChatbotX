//! `questionnaire/service.ts` (`questionnaireService`) and the read / stats /
//! delete half of `submission-service.ts` ([`submission`]). No template locks
//! (`withTemplateLockedTransaction`, `assertResourceEditable`,
//! `assertContentCopyable`, `withDeleteGuard`): DECISIONS 2026-11-13.
//! `startOrResume`, `answerCurrent` and `conversationService` belong to the
//! flow runtime unit.

pub mod submission;

use std::collections::HashMap;

use chatbotx_db::Db;
use chatbotx_db::repo::SortItem;
use chatbotx_db::repo::models::{Questionnaire, QuestionnaireQuestion};
use serde_json::Value;

use crate::errors::Result;

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

/// `findBy`: a live questionnaire of the workspace.
pub async fn find_by(_db: &Db, _workspace_id: i64, _id: i64) -> Result<Option<Questionnaire>> {
    todo!("P3.6-D1")
}

/// `findByOrFail` (`Questionnaire not found`).
pub async fn find_by_or_fail(_db: &Db, _workspace_id: i64, _id: i64) -> Result<Questionnaire> {
    todo!("P3.6-D1")
}

/// `list`: by name (asc unless `sort` asks `name` desc), with the submission
/// count.
pub async fn list(_db: &Db, _input: ListQuestionnaires) -> Result<QuestionnairePage> {
    todo!("P3.6-D1")
}

/// `listForFlow`: `(id, name)` of the live questionnaires by name.
pub async fn list_for_flow(
    _db: &Db,
    _workspace_id: i64,
    _keyword: Option<String>,
) -> Result<Vec<(i64, String)>> {
    todo!("P3.6-D1")
}

/// `getForEdit`.
pub async fn get_for_edit(_db: &Db, _workspace_id: i64, _id: i64) -> Result<QuestionnaireForEdit> {
    todo!("P3.6-D1")
}

/// `create`: the new id (the name is trimmed).
pub async fn create(_db: &Db, _workspace_id: i64, _name: &str) -> Result<i64> {
    todo!("P3.6-D1")
}

/// `rename`.
pub async fn rename(_db: &Db, _workspace_id: i64, _id: i64, _name: &str) -> Result<()> {
    todo!("P3.6-D1")
}

/// `duplicate`: the copy's id (`<name> - Copy`, `<name> - Copy 2`, ...).
pub async fn duplicate(_db: &Db, _workspace_id: i64, _id: i64) -> Result<i64> {
    todo!("P3.6-D1")
}

/// `deleteMany`: cancels open submissions, soft-deletes.
pub async fn delete_many(_db: &Db, _workspace_id: i64, _ids: Vec<i64>) -> Result<()> {
    todo!("P3.6-D1")
}

/// `update`: settings and the question list (in order).
pub async fn update(_db: &Db, _input: UpdateQuestionnaire) -> Result<()> {
    todo!("P3.6-D1")
}

/// `getQuestionnaireNameMap`.
pub async fn get_questionnaire_name_map(
    _db: &Db,
    _workspace_id: i64,
    _ids: Vec<i64>,
) -> Result<HashMap<i64, String>> {
    todo!("P3.6-D1")
}
