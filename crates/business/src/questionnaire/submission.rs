//! The read / stats / delete half of `questionnaire/submission-service.ts`
//! (`questionnaireSubmissionService`).

use chatbotx_db::Db;
use chatbotx_db::repo::SortItem;
use serde_json::Value;

use crate::errors::Result;

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
pub async fn list(_db: &Db, _input: ListSubmissions) -> Result<SubmissionPage> {
    todo!("P3.6-D1")
}

/// `dashboard`: totals and completion rate.
pub async fn dashboard(
    _db: &Db,
    _workspace_id: i64,
    _questionnaire_id: i64,
) -> Result<SubmissionStats> {
    todo!("P3.6-D1")
}

/// `detail` (`Submission not found`).
pub async fn detail(
    _db: &Db,
    _workspace_id: i64,
    _questionnaire_id: i64,
    _submission_id: i64,
) -> Result<SubmissionDetail> {
    todo!("P3.6-D1")
}

/// `findLatestCompletedForContact`.
pub async fn find_latest_completed_for_contact(
    _db: &Db,
    _workspace_id: i64,
    _contact_id: i64,
) -> Result<Option<LatestCompleted>> {
    todo!("P3.6-D1")
}

/// `deleteSubmission`.
pub async fn delete_submission(
    _db: &Db,
    _workspace_id: i64,
    _questionnaire_id: i64,
    _submission_id: i64,
) -> Result<()> {
    todo!("P3.6-D1")
}
