//! Port of `packages/business/__tests__/questionnaire-service.test.ts`
//! (describes `questionnaireService.duplicate`, `questionnaireService` and the
//! list-order / `findLatestCompletedForContact` tests of
//! `questionnaireSubmissionService`). Adapted: the mocked Drizzle builders
//! become real rows in a migrated SQLite file; "insert was called with"
//! becomes the stored row. The template-lock tests and the concurrent-copy
//! retry are not-applicable (test map). `startOrResume` / `answerCurrent` are
//! in `submission_flow`, with the `conversationService.updateChallenge` tests.

mod common;

use chatbotx_business::questionnaire::submission::{self, LatestCompleted, ListSubmissions};
use chatbotx_business::questionnaire::{self, QuestionInput, UpdateQuestionnaire};
use chatbotx_db::repo::SortItem;
use chatbotx_db::rusqlite::{Connection, params};
use chatbotx_db::{Db, DbError};
use common::{OTHER_WS, WS, rows, scalar_i64, scalar_text, setup, two_workspaces};
use serde_json::json;

const Q1: i64 = 7001;

fn questionnaire_row(tx: &Connection, id: i64, ws: i64, name: &str) -> Result<i64, DbError> {
    tx.execute(
        r#"INSERT INTO "Questionnaire" ("id", "workspaceId", "name", "enableCustomFieldMapping")
           VALUES (?1, ?2, ?3, 0)"#,
        params![id, ws, name],
    )?;
    Ok(id)
}

#[allow(clippy::too_many_arguments)]
fn submission_row(
    tx: &Connection,
    id: i64,
    questionnaire_id: i64,
    contact_id: i64,
    status: &str,
    total_points: Option<i64>,
    completed_at: Option<i64>,
    created_at: i64,
) -> Result<i64, DbError> {
    tx.execute(
        r#"INSERT INTO "QuestionnaireSubmission"
             ("id", "createdAt", "updatedAt", "workspaceId", "questionnaireId", "contactId", "status",
              "totalPoints", "startedAt", "completedAt")
           VALUES (?1, ?2, ?2, ?3, ?4, ?5, ?6, ?7, ?2, ?8)"#,
        params![
            id,
            created_at,
            WS,
            questionnaire_id,
            contact_id,
            status,
            total_points,
            completed_at
        ],
    )?;
    Ok(id)
}

fn question(title: &str, kind: &str, active: bool) -> QuestionInput {
    QuestionInput {
        title: title.into(),
        r#type: kind.into(),
        active,
        ..QuestionInput::default()
    }
}

fn update_input(mapping: bool, questions: Vec<QuestionInput>) -> UpdateQuestionnaire {
    UpdateQuestionnaire {
        workspace_id: WS,
        id: Q1,
        trigger_flow_id: None,
        enable_score: false,
        enable_retry_messages: false,
        enable_custom_field_mapping: mapping,
        questions,
    }
}

async fn questions_of(
    db: &Db,
    questionnaire_id: i64,
) -> Vec<serde_json::Map<String, serde_json::Value>> {
    rows(
        db,
        r#"SELECT "title", "active", "image", "customFieldId", "systemFieldKey", "deletedAt"
           FROM "QuestionnaireQuestion" WHERE "questionnaireId" = ?1 ORDER BY "orderNo""#,
        questionnaire_id,
    )
    .await
}

async fn questionnaire_names(db: &Db) -> Vec<String> {
    rows(
        db,
        r#"SELECT "name" FROM "Questionnaire" WHERE "workspaceId" = ?1 ORDER BY "id""#,
        WS,
    )
    .await
    .into_iter()
    .map(|row| row["name"].as_str().unwrap_or_default().to_string())
    .collect()
}

// --- questionnaireService.duplicate ------------------------------------------------

async fn duplicate_db() -> common::TestDb {
    let db = two_workspaces().await;
    setup(&db, |tx| {
        questionnaire_row(tx, Q1, WS, "Survey")?;
        Ok(())
    })
    .await;
    db
}

#[tokio::test]
async fn duplicate_names_the_first_copy_name_copy() {
    let db = duplicate_db().await;
    let copy = questionnaire::duplicate(&db, WS, Q1)
        .await
        .expect("duplicate");
    assert_ne!(copy, Q1);
    assert_eq!(
        questionnaire_names(&db).await,
        vec!["Survey", "Survey - Copy"]
    );
}

#[tokio::test]
async fn duplicate_a_second_copy_takes_the_next_free_name_instead_of_failing_on_the_unique_index() {
    let db = duplicate_db().await;
    setup(&db, |tx| {
        questionnaire_row(tx, 7002, WS, "Survey - Copy")?;
        Ok(())
    })
    .await;
    let copy = questionnaire::duplicate(&db, WS, Q1)
        .await
        .expect("duplicate");
    assert_eq!(
        scalar_text(
            &db,
            r#"SELECT "name" FROM "Questionnaire" WHERE "id" = ?1"#,
            copy
        )
        .await,
        Some("Survey - Copy 2".into())
    );
}

// --- questionnaireService ----------------------------------------------------------

async fn service_db() -> common::TestDb {
    let db = two_workspaces().await;
    setup(&db, |tx| {
        questionnaire_row(tx, Q1, WS, "Lead survey")?;
        Ok(())
    })
    .await;
    db
}

#[tokio::test]
async fn list_for_flow_does_not_filter_by_questionnaire_active_status() {
    // `Questionnaire` has no active flag: every live row of the workspace whose
    // name matches the keyword is listed, by name; deleted and foreign rows are not.
    let db = service_db().await;
    setup(&db, |tx| {
        questionnaire_row(tx, 7002, WS, "Another lead")?;
        questionnaire_row(tx, 7003, WS, "Lead deleted")?;
        tx.execute(
            r#"UPDATE "Questionnaire" SET "deletedAt" = 1 WHERE "id" = 7003"#,
            [],
        )?;
        questionnaire_row(tx, 7004, WS, "Unrelated")?;
        questionnaire_row(tx, 7005, OTHER_WS, "Lead other")?;
        Ok(())
    })
    .await;
    let listed = questionnaire::list_for_flow(&db, WS, Some("lead".into()))
        .await
        .expect("listForFlow");
    assert_eq!(
        listed,
        vec![
            (7002, "Another lead".to_string()),
            (Q1, "Lead survey".to_string())
        ]
    );
}

#[tokio::test]
async fn update_rejects_questionnaires_without_an_active_question() {
    let db = service_db().await;
    let error = questionnaire::update(
        &db,
        update_input(false, vec![question("Email", "email", false)]),
    )
    .await
    .expect_err("no active question");
    assert_eq!(
        error.message(),
        "Questionnaire requires at least one active question"
    );
    assert_eq!(error.code(), Some("noActiveQuestions"));
    // Nothing was written (the check runs before the transaction).
    assert!(questions_of(&db, Q1).await.is_empty());
}

#[tokio::test]
async fn update_saves_question_active_state_and_normalizes_blank_image_urls() {
    let db = service_db().await;
    let mut email = question("Email", "email", true);
    email.image = Some(json!({ "mode": "url", "url": "" }));
    questionnaire::update(&db, update_input(false, vec![email]))
        .await
        .expect("update");
    let stored = questions_of(&db, Q1).await;
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0]["active"], json!(1));
    assert_eq!(stored[0]["image"], json!(null));
}

#[tokio::test]
async fn update_stores_writable_system_field_mappings_separately_from_custom_fields() {
    let db = service_db().await;
    let mut email = question("Email", "email", true);
    email.custom_field_id = Some("email".into());
    questionnaire::update(&db, update_input(true, vec![email]))
        .await
        .expect("update");
    let stored = questions_of(&db, Q1).await;
    assert_eq!(stored[0]["customFieldId"], json!(null));
    assert_eq!(stored[0]["systemFieldKey"], json!("email"));
}

#[tokio::test]
async fn update_allows_custom_fields_whose_types_differ_from_the_question_type() {
    let db = service_db().await;
    setup(&db, |tx| {
        common::custom_field(tx, 123, WS, "score", "shortText")?;
        Ok(())
    })
    .await;
    let mut score = question("Score", "number", true);
    score.custom_field_id = Some("123".into());
    questionnaire::update(&db, update_input(true, vec![score]))
        .await
        .expect("update");
    let stored = questions_of(&db, Q1).await;
    assert_eq!(stored[0]["customFieldId"], json!(123));
    assert_eq!(stored[0]["systemFieldKey"], json!(null));
}

#[tokio::test]
async fn update_rejects_missing_custom_fields() {
    let db = service_db().await;
    setup(&db, |tx| {
        // A field of another workspace does not count.
        common::custom_field(tx, 123, OTHER_WS, "score", "shortText")?;
        Ok(())
    })
    .await;
    let mut score = question("Score", "number", true);
    score.custom_field_id = Some("123".into());
    let error = questionnaire::update(&db, update_input(true, vec![score]))
        .await
        .expect_err("missing custom field");
    assert_eq!(
        error.message(),
        "Custom field does not exist in the workspace"
    );
    assert_eq!(error.code(), Some("invalidCustomField"));
}

#[tokio::test]
async fn delete_many_soft_deletes_questionnaires_instead_of_deleting_rows() {
    let db = service_db().await;
    setup(&db, |tx| {
        common::contact(tx, 3001, WS)?;
        submission_row(tx, 8001, Q1, 3001, "inProgress", None, None, 1_000)?;
        Ok(())
    })
    .await;
    questionnaire::delete_many(&db, WS, vec![Q1])
        .await
        .expect("deleteMany");
    let deleted_at = scalar_i64(
        &db,
        r#"SELECT "deletedAt" FROM "Questionnaire" WHERE "id" = ?1"#,
        Q1,
    )
    .await;
    assert!(
        deleted_at.is_some_and(|at| at > 0),
        "row kept with deletedAt"
    );
    // `deleteInTx` cancels the open submissions first.
    assert_eq!(
        scalar_text(
            &db,
            r#"SELECT "status" FROM "QuestionnaireSubmission" WHERE "id" = ?1"#,
            8001
        )
        .await,
        Some("cancelled".into())
    );
    assert!(
        questionnaire::find_by(&db, WS, Q1)
            .await
            .expect("findBy")
            .is_none(),
        "a deleted questionnaire is not found"
    );
}

#[tokio::test]
async fn update_rejects_system_fields_that_do_not_match_the_question_type() {
    let db = service_db().await;
    let mut score = question("Score", "number", true);
    score.system_field_key = Some("email".into());
    let error = questionnaire::update(&db, update_input(true, vec![score]))
        .await
        .expect_err("system field mismatch");
    assert_eq!(error.message(), "System field does not match question type");
    assert_eq!(error.code(), Some("invalidSystemField"));
}

// --- questionnaireSubmissionService ------------------------------------------------

/// Three submissions: (id, contact first name, status, points, completedAt, createdAt).
async fn submissions_db() -> common::TestDb {
    let db = service_db().await;
    setup(&db, |tx| {
        common::contact_with(tx, 3001, WS, Some("Bea"), None, None)?;
        common::contact_with(tx, 3002, WS, Some("Ann"), None, None)?;
        common::contact_with(tx, 3003, WS, Some("Cid"), None, None)?;
        submission_row(tx, 8001, Q1, 3001, "inProgress", None, None, 1_000)?;
        submission_row(tx, 8002, Q1, 3002, "completed", Some(5), Some(5_000), 2_000)?;
        submission_row(tx, 8003, Q1, 3003, "cancelled", Some(2), Some(4_000), 3_000)?;
        Ok(())
    })
    .await;
    db
}

async fn listed_ids(db: &Db, sort: Option<Vec<SortItem>>) -> Vec<i64> {
    submission::list(
        db,
        ListSubmissions {
            workspace_id: WS,
            questionnaire_id: Q1,
            sort,
            ..ListSubmissions::default()
        },
    )
    .await
    .expect("list submissions")
    .data
    .iter()
    .map(|row| row.id)
    .collect()
}

fn sort(id: &str, desc: bool) -> Option<Vec<SortItem>> {
    Some(vec![SortItem {
        id: id.into(),
        desc,
    }])
}

#[tokio::test]
async fn builds_questionnaire_submission_list_order_for_name_total_points_and_status_asc_and_desc()
{
    // Postgres order: NULLS LAST ascending, NULLS FIRST descending.
    let db = submissions_db().await;
    let cases: [(&str, Vec<i64>); 3] = [
        // fullName: Ann (8002), Bea (8001), Cid (8003).
        ("name", vec![8002, 8001, 8003]),
        // totalPoints: 2 (8003), 5 (8002), null (8001).
        ("totalPoints", vec![8003, 8002, 8001]),
        // status is a Postgres enum: declaration order inProgress, completed, cancelled.
        ("status", vec![8001, 8002, 8003]),
    ];
    for (sort_id, ascending) in cases {
        assert_eq!(
            listed_ids(&db, sort(sort_id, false)).await,
            ascending,
            "{sort_id} asc"
        );
        let mut descending = ascending.clone();
        descending.reverse();
        assert_eq!(
            listed_ids(&db, sort(sort_id, true)).await,
            descending,
            "{sort_id} desc"
        );
    }
}

#[tokio::test]
async fn builds_completed_at_sort_with_nulls_last_for_asc_and_desc() {
    let db = submissions_db().await;
    // completedAt: 4000 (8003), 5000 (8002), null (8001) last both ways.
    assert_eq!(
        listed_ids(&db, sort("completedAt", false)).await,
        vec![8003, 8002, 8001]
    );
    assert_eq!(
        listed_ids(&db, sort("completedAt", true)).await,
        vec![8002, 8003, 8001]
    );
}

#[tokio::test]
async fn falls_back_to_created_at_desc_for_empty_or_invalid_submission_list_sort() {
    let db = submissions_db().await;
    let newest_first = vec![8003, 8002, 8001];
    assert_eq!(listed_ids(&db, Some(vec![])).await, newest_first);
    assert_eq!(listed_ids(&db, None).await, newest_first);
    assert_eq!(
        listed_ids(&db, sort("notAllowed", false)).await,
        newest_first
    );
}

#[tokio::test]
async fn find_latest_completed_for_contact_reads_the_newest_scored_completed_submission_of_the_contact()
 {
    let db = service_db().await;
    setup(&db, |tx| {
        questionnaire_row(tx, 7002, WS, "Other survey")?;
        common::contact(tx, 3001, WS)?;
        common::contact(tx, 3002, WS)?;
        // Older completed and scored.
        submission_row(tx, 8001, Q1, 3001, "completed", Some(4), Some(1_000), 500)?;
        // Newest scored completed, in another questionnaire (all questionnaires count).
        submission_row(
            tx,
            8002,
            7002,
            3001,
            "completed",
            Some(12),
            Some(3_000),
            600,
        )?;
        // Newer but without a score / not completed / another contact.
        submission_row(tx, 8003, Q1, 3001, "completed", None, Some(4_000), 700)?;
        submission_row(tx, 8004, Q1, 3001, "inProgress", Some(20), None, 800)?;
        submission_row(tx, 8005, Q1, 3002, "completed", Some(30), Some(5_000), 900)?;
        Ok(())
    })
    .await;
    let latest = submission::find_latest_completed_for_contact(&db, WS, 3001)
        .await
        .expect("findLatestCompletedForContact");
    assert_eq!(
        latest,
        Some(LatestCompleted {
            id: 8002,
            total_points: 12
        })
    );
}

#[tokio::test]
async fn find_latest_completed_for_contact_returns_undefined_when_the_contact_has_none() {
    let db = service_db().await;
    setup(&db, |tx| {
        common::contact(tx, 3001, WS)?;
        submission_row(tx, 8001, Q1, 3001, "inProgress", None, None, 500)?;
        Ok(())
    })
    .await;
    assert_eq!(
        submission::find_latest_completed_for_contact(&db, WS, 3001)
            .await
            .expect("findLatestCompletedForContact"),
        None
    );
}

// --- questionnaireSubmissionService.startOrResume / answerCurrent ----------------
// Adapted: "update/insert was (not) called" becomes the stored submission row.

mod submission_flow {
    use super::*;
    use chatbotx_business::questionnaire::flow::{self, AnswerInput, Answered, Started};

    const CONTACT: i64 = 3010;
    const CONV_OLD: i64 = 601;
    const CONV_NEW: i64 = 602;
    const QA: i64 = 9001;
    const QB: i64 = 9002;
    const SUB: i64 = 8101;

    fn question_row(
        tx: &Connection,
        id: i64,
        order: i64,
        kind: &str,
        title: &str,
    ) -> Result<i64, DbError> {
        tx.execute(
            r#"INSERT INTO "QuestionnaireQuestion" ("id", "questionnaireId", "title", "type", "orderNo", "retryMessage")
               VALUES (?1, ?2, ?3, ?4, ?5, 'Try again')"#,
            params![id, Q1, title, kind, order],
        )?;
        Ok(id)
    }

    fn submission(
        tx: &Connection,
        status: &str,
        conversation: i64,
        current: Option<i64>,
        questionnaire: i64,
    ) -> Result<(), DbError> {
        tx.execute(
            r#"INSERT INTO "QuestionnaireSubmission"
                 ("id", "workspaceId", "questionnaireId", "contactId", "conversationId", "status", "currentQuestionId")
               VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)"#,
            params![SUB, WS, questionnaire, CONTACT, conversation, status, current],
        )?;
        Ok(())
    }

    /// Q1 with questions `QA` (email) and `QB` (text), a contact and two conversations.
    async fn flow_db(
        kinds: [&'static str; 2],
        extra: impl FnOnce(&Connection) -> Result<(), DbError> + Send + 'static,
    ) -> common::TestDb {
        let db = service_db().await;
        setup(&db, move |tx| {
            common::contact(tx, CONTACT, WS)?;
            for conv in [CONV_OLD, CONV_NEW] {
                tx.execute(
                    r#"INSERT INTO "Conversation" ("id", "workspaceId", "contactId") VALUES (?1, ?2, ?3)"#,
                    params![conv, WS, CONTACT],
                )?;
            }
            question_row(tx, QA, 0, kinds[0], "Question 1")?;
            question_row(tx, QB, 1, kinds[1], "Question 2")?;
            extra(tx)
        })
        .await;
        db
    }

    async fn submission_state(db: &Db) -> serde_json::Map<String, serde_json::Value> {
        rows(
            db,
            r#"SELECT "conversationId", "status", "currentQuestionId", "lastAnsweredMessageId", "completedAt"
               FROM "QuestionnaireSubmission" WHERE "id" = ?1"#,
            SUB,
        )
        .await
        .remove(0)
    }

    async fn submission_count(db: &Db) -> Option<i64> {
        scalar_i64(
            db,
            r#"SELECT COUNT(*) FROM "QuestionnaireSubmission" WHERE "contactId" = ?1"#,
            CONTACT,
        )
        .await
    }

    async fn start(db: &Db, conversation: i64) -> Started {
        flow::start_or_resume(db, WS, Q1, CONTACT, Some(conversation))
            .await
            .expect("start_or_resume")
    }

    fn question_id(started: &Started) -> i64 {
        match started {
            Started::Wait { question, .. } => question.id,
            other => panic!("expected wait, got {other:?}"),
        }
    }

    fn answer(text: &str, attempts: i64, trigger: Option<i64>) -> AnswerInput {
        AnswerInput {
            workspace_id: WS,
            contact_id: CONTACT,
            conversation_id: CONV_OLD,
            questionnaire_id: Q1,
            raw_text: text.into(),
            attempts,
            trigger_message_id: trigger,
        }
    }

    #[tokio::test]
    async fn start_or_resume_skips_when_the_contact_already_has_a_submission_for_the_questionnaire()
    {
        let db = flow_db(["text", "text"], |tx| {
            submission(tx, "completed", CONV_OLD, None, Q1)
        })
        .await;
        assert_eq!(
            start(&db, CONV_NEW).await,
            Started::Skip("questionnaire_already_submitted")
        );
        assert_eq!(submission_count(&db).await, Some(1));
        assert_eq!(
            submission_state(&db).await["conversationId"],
            json!(CONV_OLD)
        );
    }

    #[tokio::test]
    async fn start_or_resume_restarts_a_cancelled_applicant_from_the_first_question() {
        let db = flow_db(["text", "text"], |tx| {
            submission(tx, "cancelled", CONV_OLD, None, Q1)?;
            tx.execute(
                r#"INSERT INTO "QuestionnaireAnswer" ("id", "submissionId", "questionId", "questionIdSnapshot",
                     "questionTitleSnapshot", "questionTypeSnapshot", "labelSnapshot", "value")
                   VALUES (1, ?1, ?2, ?3, 'Question 1', 'text', 'Question 1', '"old"')"#,
                params![SUB, QA, QA.to_string()],
            )?;
            Ok(())
        })
        .await;
        let started = start(&db, CONV_NEW).await;
        assert_eq!(question_id(&started), QA);
        let state = submission_state(&db).await;
        assert_eq!(state["conversationId"], json!(CONV_NEW));
        assert_eq!(state["status"], json!("inProgress"));
        assert_eq!(state["currentQuestionId"], json!(QA));
        assert_eq!(state["completedAt"], json!(null));
        assert_eq!(submission_count(&db).await, Some(1));
        assert_eq!(
            scalar_i64(
                &db,
                r#"SELECT COUNT(*) FROM "QuestionnaireAnswer" WHERE "submissionId" = ?1"#,
                SUB
            )
            .await,
            Some(0)
        );
    }

    #[tokio::test]
    async fn start_or_resume_reassigns_active_same_questionnaire_submission_to_current_conversation()
     {
        let db = flow_db(["text", "text"], |tx| {
            submission(tx, "inProgress", CONV_OLD, Some(QB), Q1)
        })
        .await;
        assert_eq!(question_id(&start(&db, CONV_NEW).await), QB);
        assert_eq!(
            submission_state(&db).await["conversationId"],
            json!(CONV_NEW)
        );
        assert_eq!(submission_count(&db).await, Some(1));
    }

    #[tokio::test]
    async fn start_or_resume_resumes_active_same_questionnaire_submission_without_update_when_conversation_is_unchanged()
     {
        let db = flow_db(["text", "text"], |tx| {
            submission(tx, "inProgress", CONV_OLD, Some(QB), Q1)
        })
        .await;
        let before = scalar_i64(
            &db,
            r#"SELECT "updatedAt" FROM "QuestionnaireSubmission" WHERE "id" = ?1"#,
            SUB,
        )
        .await;
        assert_eq!(question_id(&start(&db, CONV_OLD).await), QB);
        let after = scalar_i64(
            &db,
            r#"SELECT "updatedAt" FROM "QuestionnaireSubmission" WHERE "id" = ?1"#,
            SUB,
        )
        .await;
        assert_eq!(before, after);
        assert_eq!(submission_count(&db).await, Some(1));
    }

    #[tokio::test]
    async fn start_or_resume_skips_active_same_questionnaire_submission_when_the_current_question_is_missing()
     {
        let db = flow_db(["text", "text"], |tx| {
            submission(tx, "inProgress", CONV_OLD, Some(QB), Q1)?;
            tx.execute(
                r#"UPDATE "QuestionnaireQuestion" SET "active" = 0 WHERE "id" = ?1"#,
                [QB],
            )?;
            Ok(())
        })
        .await;
        assert_eq!(
            start(&db, CONV_OLD).await,
            Started::Skip("questionnaire_current_question_missing")
        );
    }

    #[tokio::test]
    async fn start_or_resume_creates_the_first_submission_when_the_contact_has_no_applicant_record()
    {
        // Inactive and deleted questions are not asked; the first active one by orderNo is.
        let db = flow_db(["text", "text"], |tx| {
            tx.execute(
                r#"UPDATE "QuestionnaireQuestion" SET "active" = 0 WHERE "id" = ?1"#,
                [QA],
            )?;
            Ok(())
        })
        .await;
        assert_eq!(question_id(&start(&db, CONV_OLD).await), QB);
        let stored = rows(
            &db,
            r#"SELECT "questionnaireId", "conversationId", "status", "currentQuestionId"
               FROM "QuestionnaireSubmission" WHERE "contactId" = ?1"#,
            CONTACT,
        )
        .await;
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0]["questionnaireId"], json!(Q1));
        assert_eq!(stored[0]["conversationId"], json!(CONV_OLD));
        assert_eq!(stored[0]["status"], json!("inProgress"));
        assert_eq!(stored[0]["currentQuestionId"], json!(QB));
    }

    #[tokio::test]
    async fn start_or_resume_skips_when_the_contact_has_another_active_questionnaire() {
        let db = flow_db(["text", "text"], |tx| {
            questionnaire_row(tx, 7002, WS, "Other survey")?;
            submission(tx, "inProgress", CONV_OLD, None, 7002)
        })
        .await;
        assert_eq!(
            start(&db, CONV_OLD).await,
            Started::Skip("contact_has_other_active_questionnaire")
        );
        assert_eq!(submission_count(&db).await, Some(1));
    }

    #[tokio::test]
    async fn answer_current_locks_the_active_submission_before_reading_typed_state() {
        // The read runs inside the one-writer transaction; no submission → skip.
        let db = flow_db(["text", "text"], |_| Ok(())).await;
        assert_eq!(
            flow::answer_current(&db, answer("hello", 0, None))
                .await
                .unwrap(),
            Answered::Skip("submission_not_found")
        );
    }

    #[tokio::test]
    async fn answer_current_treats_repeated_trigger_messages_as_already_processed() {
        let db = flow_db(["text", "text"], |tx| {
            submission(tx, "inProgress", CONV_OLD, Some(QB), Q1)?;
            tx.execute(
                r#"UPDATE "QuestionnaireSubmission" SET "lastAnsweredMessageId" = 77, "currentQuestionSentAt" = 1767398400000 WHERE "id" = ?1"#,
                [SUB],
            )?;
            Ok(())
        })
        .await;
        match flow::answer_current(&db, answer("should not be applied again", 0, Some(77)))
            .await
            .unwrap()
        {
            Answered::Wait {
                submission_id,
                question,
                sent_at,
            } => {
                assert_eq!(
                    (submission_id, question.id, sent_at),
                    (SUB, QB, 1_767_398_400_000)
                );
            }
            other => panic!("expected wait, got {other:?}"),
        }
        assert_eq!(
            scalar_i64(
                &db,
                r#"SELECT COUNT(*) FROM "QuestionnaireAnswer" WHERE "submissionId" = ?1"#,
                SUB
            )
            .await,
            Some(0)
        );
    }

    #[tokio::test]
    async fn answer_current_rejects_overlong_free_text_answers() {
        let db = flow_db(["text", "text"], |tx| {
            submission(tx, "inProgress", CONV_OLD, Some(QA), Q1)
        })
        .await;
        match flow::answer_current(&db, answer(&"x".repeat(1001), 0, None))
            .await
            .unwrap()
        {
            Answered::Retry {
                submission_id,
                question,
                attempts,
                retry_message,
                reason,
            } => {
                assert_eq!((submission_id, question.id, attempts), (SUB, QA, 1));
                assert_eq!(retry_message.as_deref(), Some("Try again"));
                assert_eq!(reason, "too_long");
            }
            other => panic!("expected retry, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn answer_current_advances_to_the_next_question_after_max_invalid_attempts() {
        let db = flow_db(["email", "text"], |tx| {
            submission(tx, "inProgress", CONV_OLD, Some(QA), Q1)
        })
        .await;
        match flow::answer_current(&db, answer("not-an-email", 2, Some(78)))
            .await
            .unwrap()
        {
            Answered::Wait {
                submission_id,
                question,
                ..
            } => assert_eq!((submission_id, question.id), (SUB, QB)),
            other => panic!("expected wait, got {other:?}"),
        }
        let state = submission_state(&db).await;
        assert_eq!(state["currentQuestionId"], json!(QB));
        assert_eq!(state["lastAnsweredMessageId"], json!(78));
        assert_eq!(state["status"], json!("inProgress"));
    }

    #[tokio::test]
    async fn answer_current_completes_the_submission_when_max_invalid_attempts_happen_on_the_last_question()
     {
        let db = flow_db(["text", "email"], |tx| {
            tx.execute(
                r#"UPDATE "Questionnaire" SET "triggerFlowId" = NULL WHERE "id" = ?1"#,
                [Q1],
            )?;
            submission(tx, "inProgress", CONV_OLD, Some(QB), Q1)
        })
        .await;
        match flow::answer_current(&db, answer("not-an-email", 2, Some(79)))
            .await
            .unwrap()
        {
            Answered::Completed {
                submission_id,
                trigger_flow_id,
                ..
            } => {
                assert_eq!((submission_id, trigger_flow_id), (SUB, None));
            }
            other => panic!("expected completed, got {other:?}"),
        }
        let state = submission_state(&db).await;
        assert_eq!(state["status"], json!("completed"));
        assert_eq!(state["currentQuestionId"], json!(null));
        assert_eq!(state["lastAnsweredMessageId"], json!(79));
        assert!(state["completedAt"].is_i64());
    }

    #[tokio::test]
    async fn update_challenge_updates_only_the_challenge_key_in_additional_attributes() {
        let db = flow_db(["text", "text"], |tx| {
            tx.execute(
                r#"UPDATE "Conversation" SET "additionalAttributes" = '{"other":1}' WHERE "id" = ?1"#,
                [CONV_OLD],
            )?;
            Ok(())
        })
        .await;
        let challenge = json!({ "type": "step", "data": { "stepId": "s1" } });
        chatbotx_business::conversation_challenge::update_challenge(
            &db,
            WS,
            CONV_OLD,
            Some(challenge.clone()),
        )
        .await
        .unwrap();
        let raw = scalar_text(
            &db,
            r#"SELECT "additionalAttributes" FROM "Conversation" WHERE "id" = ?1"#,
            CONV_OLD,
        )
        .await
        .unwrap();
        let stored: serde_json::Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(stored, json!({ "other": 1, "challenge": challenge }));
    }

    #[tokio::test]
    async fn update_challenge_throws_when_conversation_update_matches_no_rows() {
        let db = flow_db(["text", "text"], |_| Ok(())).await;
        assert!(
            chatbotx_business::conversation_challenge::update_challenge(
                &db, OTHER_WS, CONV_OLD, None
            )
            .await
            .is_err()
        );
    }
}
