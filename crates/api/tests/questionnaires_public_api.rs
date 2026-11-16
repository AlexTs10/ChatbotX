//! Port of `apps/builder/__tests__/questionnaires-public-api.test.ts`.
//! Adapted: the captured oRPC handlers become `handlers::dispatch` calls over
//! a real migrated database; "the service was called with" becomes the stored
//! rows / the response.

mod handlers_common;

use chatbotx_api::routes::{routes_of_module, scopes_of_module};
use chatbotx_db::rusqlite::params;
use handlers_common::{Fixture, OTHER_WS, WS, call, fixture, scalar};
use serde_json::{Value, json};

const MODULE: &str = "features/questionnaires/api/public";

async fn new_questionnaire(f: &Fixture, name: &str) -> String {
    let (status, body) = call(
        f,
        "questionnaires.create",
        WS,
        &[],
        None,
        Some(json!({ "name": name })),
    )
    .await;
    assert_eq!(status, 201, "{body}");
    body["id"].as_str().expect("id").to_string()
}

fn one_question() -> Value {
    json!([{ "title": "Your name?", "type": "text", "active": true }])
}

/// Contacts 3001 (Ada Lovelace) and 3002 (Bob) and `count` submissions of
/// questionnaire `qid`; the first `completed` of them are completed.
async fn seed_submissions(f: &Fixture, qid: i64, count: i64, completed: i64) {
    f.state
        .db
        .write(move |tx| {
            tx.execute(
                r#"INSERT INTO "Contact" ("id", "workspaceId", "firstName", "lastName") VALUES (3001, ?1, 'Ada', 'Lovelace')"#,
                params![WS],
            )?;
            tx.execute(
                r#"INSERT INTO "Contact" ("id", "workspaceId", "firstName") VALUES (3002, ?1, 'Bob')"#,
                params![WS],
            )?;
            for n in 0..count {
                // Orchestrator fix (P3.6-D1): at most one open submission per contact
            // (`QuestionnaireSubmission_workspaceId_contactId_active_key`).
            let status = if n < completed {
                "completed"
            } else if n < completed + 2 {
                "inProgress"
            } else {
                "cancelled"
            };
                tx.execute(
                    r#"INSERT INTO "QuestionnaireSubmission"
                         ("id", "createdAt", "updatedAt", "workspaceId", "questionnaireId", "contactId", "status",
                          "totalPoints", "startedAt", "completedAt")
                       VALUES (?1, ?2, ?2, ?3, ?4, ?5, ?6, ?7, ?2, ?8)"#,
                    params![
                        8001 + n,
                        1_000 + n,
                        WS,
                        qid,
                        if n % 2 == 0 { 3001 } else { 3002 },
                        status,
                        (n < completed).then_some(n),
                        (n < completed).then_some(2_000 + n)
                    ],
                )?;
            }
            Ok(())
        })
        .await
        .expect("seed submissions");
}

#[tokio::test]
async fn registers_the_questionnaires_public_router_under_the_automation_scope() {
    assert_eq!(scopes_of_module(MODULE), vec!["automation"]);
}

#[tokio::test]
async fn registers_each_questionnaire_route_with_its_public_method_and_path() {
    // The route table is matched by method and path, not by declaration
    // order: compare the sets.
    let mut routes: Vec<(&str, &str)> = routes_of_module(MODULE)
        .iter()
        .map(|route| (route.method, route.path))
        .collect();
    routes.sort_unstable();
    let mut expected = vec![
        ("GET", "/v1/questionnaires"),
        ("GET", "/v1/questionnaires/{id}"),
        ("POST", "/v1/questionnaires"),
        ("PUT", "/v1/questionnaires/{id}"),
        ("DELETE", "/v1/questionnaires/{id}"),
        ("PATCH", "/v1/questionnaires/{id}/rename"),
        ("POST", "/v1/questionnaires/{id}/duplicate"),
        ("GET", "/v1/questionnaires/{id}/submissions"),
        ("GET", "/v1/questionnaires/{id}/submissions/{submissionId}"),
        (
            "DELETE",
            "/v1/questionnaires/{id}/submissions/{submissionId}",
        ),
        ("GET", "/v1/questionnaires/{id}/submissions/stats"),
    ];
    expected.sort_unstable();
    assert_eq!(routes, expected);
}

// --- questionnaire resource handlers -----------------------------------------------

#[tokio::test]
async fn lists_workspace_questionnaires() {
    let f = fixture().await;
    for name in ["Quiz A", "Quiz B", "Quiz C", "Other"] {
        new_questionnaire(&f, name).await;
    }
    let (status, body) = call(
        &f,
        "questionnaires.list",
        WS,
        &[],
        Some("page=2&perPage=2&name=Quiz"),
        None,
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["pageCount"], json!(2));
    let data = body["data"].as_array().expect("data");
    assert_eq!(data.len(), 1);
    assert_eq!(data[0]["name"], json!("Quiz C"));
    assert_eq!(data[0]["applicantsCount"], json!(0));
    // Another workspace sees none of them.
    let (_, other) = call(&f, "questionnaires.list", OTHER_WS, &[], None, None).await;
    assert_eq!(other, json!({ "data": [], "pageCount": 0 }));
}

#[tokio::test]
async fn gets_a_questionnaire_through_the_workspace_scoped_service() {
    let f = fixture().await;
    let id = new_questionnaire(&f, "Quiz").await;
    let (status, body) = call(&f, "questionnaires.get", WS, &[("id", &id)], None, None).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["id"], json!(id));
    assert_eq!(body["name"], json!("Quiz"));
    assert_eq!(body["questions"], json!([]));
    let (status, _) = call(
        &f,
        "questionnaires.get",
        OTHER_WS,
        &[("id", &id)],
        None,
        None,
    )
    .await;
    assert_eq!(status, 404);
}

#[tokio::test]
async fn propagates_a_declared_not_found_error_from_get() {
    let f = fixture().await;
    let (status, body) = call(&f, "questionnaires.get", WS, &[("id", "999")], None, None).await;
    assert_eq!(status, 404);
    // Orchestrator fix (P3.6-D1): the declared error code is "notFound" (parity expectNotFound).
    assert_eq!(body["code"], json!("notFound"));
    assert_eq!(body["message"], json!("Questionnaire not found"));
    assert_eq!(body["defined"], json!(true));
}

#[tokio::test]
async fn creates_a_questionnaire_and_returns_its_id() {
    let f = fixture().await;
    let id = new_questionnaire(&f, "Quiz").await;
    let id_num: i64 = id.parse().expect("numeric id");
    assert_eq!(
        scalar(
            &f,
            r#"SELECT "name" FROM "Questionnaire" WHERE "id" = ?1"#,
            id_num
        )
        .await,
        Some(json!("Quiz"))
    );
    assert_eq!(
        scalar(
            &f,
            r#"SELECT "workspaceId" FROM "Questionnaire" WHERE "id" = ?1"#,
            id_num
        )
        .await,
        Some(json!(WS))
    );
}

#[tokio::test]
async fn updates_a_questionnaire_with_its_id_from_the_route() {
    let f = fixture().await;
    let id = new_questionnaire(&f, "Quiz").await;
    let (status, body) = call(
        &f,
        "questionnaires.update",
        WS,
        &[("id", &id)],
        None,
        Some(json!({
            "triggerFlowId": null,
            "enableScore": true,
            "enableRetryMessages": false,
            "enableCustomFieldMapping": true,
            "questions": one_question(),
        })),
    )
    .await;
    assert_eq!(status, 204, "{body}");
    assert_eq!(body, Value::Null);
    let id_num: i64 = id.parse().expect("numeric id");
    assert_eq!(
        scalar(
            &f,
            r#"SELECT "enableScore" FROM "Questionnaire" WHERE "id" = ?1"#,
            id_num
        )
        .await,
        Some(json!(1))
    );
    assert_eq!(
        scalar(
            &f,
            r#"SELECT "title" FROM "QuestionnaireQuestion" WHERE "questionnaireId" = ?1"#,
            id_num
        )
        .await,
        Some(json!("Your name?"))
    );
}

#[tokio::test]
async fn renames_a_questionnaire_with_its_id_from_the_route() {
    let f = fixture().await;
    let id = new_questionnaire(&f, "Quiz").await;
    let (status, _) = call(
        &f,
        "questionnaires.rename",
        WS,
        &[("id", &id)],
        None,
        Some(json!({ "name": "Renamed Quiz" })),
    )
    .await;
    assert_eq!(status, 204);
    let id_num: i64 = id.parse().expect("numeric id");
    assert_eq!(
        scalar(
            &f,
            r#"SELECT "name" FROM "Questionnaire" WHERE "id" = ?1"#,
            id_num
        )
        .await,
        Some(json!("Renamed Quiz"))
    );
}

#[tokio::test]
async fn deletes_one_questionnaire_through_delete_many() {
    let f = fixture().await;
    let id = new_questionnaire(&f, "Quiz").await;
    let keep = new_questionnaire(&f, "Keep").await;
    let (status, _) = call(&f, "questionnaires.delete", WS, &[("id", &id)], None, None).await;
    assert_eq!(status, 204);
    let deleted = scalar(
        &f,
        r#"SELECT "deletedAt" FROM "Questionnaire" WHERE "id" = ?1"#,
        id.parse().expect("numeric id"),
    )
    .await;
    assert!(deleted.is_some_and(|v| v.is_i64()), "soft-deleted");
    let kept = scalar(
        &f,
        r#"SELECT "deletedAt" FROM "Questionnaire" WHERE "id" = ?1"#,
        keep.parse().expect("numeric id"),
    )
    .await;
    assert_eq!(kept, Some(Value::Null));
}

#[tokio::test]
async fn duplicates_a_questionnaire_and_returns_its_id() {
    let f = fixture().await;
    let id = new_questionnaire(&f, "Quiz").await;
    let (status, body) = call(
        &f,
        "questionnaires.duplicate",
        WS,
        &[("id", &id)],
        None,
        Some(json!({})),
    )
    .await;
    assert_eq!(status, 201, "{body}");
    let copy = body["id"].as_str().expect("id");
    assert_ne!(copy, id);
    assert_eq!(body.as_object().map(|o| o.len()), Some(1));
    assert_eq!(
        scalar(
            &f,
            r#"SELECT "name" FROM "Questionnaire" WHERE "id" = ?1"#,
            copy.parse().expect("numeric id")
        )
        .await,
        Some(json!("Quiz - Copy"))
    );
}

// --- questionnaire submission handlers ---------------------------------------------

#[tokio::test]
async fn lists_submissions_under_the_questionnaire_identified_by_the_route() {
    let f = fixture().await;
    let id = new_questionnaire(&f, "Quiz").await;
    let other = new_questionnaire(&f, "Other").await;
    seed_submissions(&f, id.parse().expect("numeric id"), 3, 1).await;
    let (status, body) = call(
        &f,
        "questionnaires.listSubmissions",
        WS,
        &[("id", &id)],
        Some("page=1&perPage=20&name=Ada"),
        None,
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["enableScore"], json!(false));
    assert_eq!(body["pageCount"], json!(1));
    // Submissions 8001 and 8003 belong to Ada; newest first.
    let ids: Vec<&str> = body["data"]
        .as_array()
        .expect("data")
        .iter()
        .map(|row| row["id"].as_str().unwrap_or_default())
        .collect();
    assert_eq!(ids, vec!["8003", "8001"]);
    assert_eq!(
        body["data"][1],
        json!({
            "id": "8001",
            "status": "completed",
            "totalPoints": 0,
            "completedAt": "1970-01-01T00:00:02.000Z",
            "conversationId": null,
            "contact": {
                "id": "3001",
                "fullName": "Ada Lovelace",
                "firstName": "Ada",
                "lastName": "Lovelace",
                "email": null,
                "phoneNumber": null,
                "avatar": null,
            },
        })
    );
    let (_, empty) = call(
        &f,
        "questionnaires.listSubmissions",
        WS,
        &[("id", &other)],
        None,
        None,
    )
    .await;
    assert_eq!(
        empty,
        json!({ "enableScore": false, "data": [], "pageCount": 0 })
    );
}

#[tokio::test]
async fn gets_one_submission_under_its_questionnaire() {
    let f = fixture().await;
    let id = new_questionnaire(&f, "Quiz").await;
    let other = new_questionnaire(&f, "Other").await;
    seed_submissions(&f, id.parse().expect("numeric id"), 1, 1).await;
    let qid: i64 = id.parse().expect("numeric id");
    f.state
        .db
        .write(move |tx| {
            // The answer's question (FK `questionId`).
            tx.execute(
                r#"INSERT INTO "QuestionnaireQuestion" ("id", "questionnaireId", "title", "type") VALUES (77, ?1, 'Your name?', 'text')"#,
                params![qid],
            )?;
            tx.execute(
                r#"INSERT INTO "QuestionnaireAnswer"
                     ("id", "createdAt", "updatedAt", "submissionId", "questionId", "questionIdSnapshot",
                      "questionTitleSnapshot", "questionTypeSnapshot", "labelSnapshot", "value", "pointsEarned", "answeredAt")
                   VALUES (9001, 5, 5, 8001, 77, '77', 'Your name?', 'text', 'Your name?', '"Ada"', NULL, 5)"#,
                [],
            )?;
            Ok(())
        })
        .await
        .expect("seed answer");
    let (status, body) = call(
        &f,
        "questionnaires.getSubmission",
        WS,
        &[("id", &id), ("submissionId", "8001")],
        None,
        None,
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(
        body,
        json!({
            "id": "8001",
            "contact": { "id": "3001", "fullName": "Ada Lovelace" },
            "conversationId": null,
            "status": "completed",
            "totalPoints": 0,
            "answers": [
                { "questionId": "77", "label": "Your name?", "value": "Ada", "pointsEarned": null },
            ],
        })
    );
    // Under another questionnaire the submission is not found.
    let (status, body) = call(
        &f,
        "questionnaires.getSubmission",
        WS,
        &[("id", &other), ("submissionId", "8001")],
        None,
        None,
    )
    .await;
    assert_eq!(status, 404);
    assert_eq!(body["message"], json!("Submission not found"));
}

#[tokio::test]
async fn deletes_one_submission_through_the_single_id_service_method() {
    let f = fixture().await;
    let id = new_questionnaire(&f, "Quiz").await;
    seed_submissions(&f, id.parse().expect("numeric id"), 2, 0).await;
    let (status, _) = call(
        &f,
        "questionnaires.deleteSubmission",
        WS,
        &[("id", &id), ("submissionId", "8001")],
        None,
        None,
    )
    .await;
    assert_eq!(status, 204);
    assert_eq!(
        scalar(
            &f,
            r#"SELECT "id" FROM "QuestionnaireSubmission" WHERE "id" = ?1"#,
            8001
        )
        .await,
        None
    );
    assert_eq!(
        scalar(
            &f,
            r#"SELECT "id" FROM "QuestionnaireSubmission" WHERE "id" = ?1"#,
            8002
        )
        .await,
        Some(json!(8002))
    );
}

#[tokio::test]
async fn gets_submission_stats_for_a_questionnaire() {
    let f = fixture().await;
    let id = new_questionnaire(&f, "Quiz").await;
    seed_submissions(&f, id.parse().expect("numeric id"), 10, 4).await;
    let (status, body) = call(
        &f,
        "questionnaires.getSubmissionStats",
        WS,
        &[("id", &id)],
        None,
        None,
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(
        body,
        json!({ "totalApplicants": 10, "completed": 4, "completionRate": 40 })
    );
}
