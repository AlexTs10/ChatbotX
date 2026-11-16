//! `features/questionnaires/api/public.ts`: `questionnaires.*`.

use chatbotx_business::questionnaire::submission::{self, ListSubmissions, SubmissionContact};
use chatbotx_business::questionnaire::{
    self as service, ListQuestionnaires, QuestionInput, SUPPORTED_QUESTION_TYPES,
    UpdateQuestionnaire,
};
use chatbotx_db::repo::models::{Questionnaire, QuestionnaireQuestion};
use serde_json::{Map, Value, json};

use super::input::{Input, Num, Path, Seg, Str, child, key};
use super::qr_codes::{public_paging, sort_query};
use super::{ApiState, Call, Reply, business_error, date, id, json, no_content, opt_date, opt_id};

/// The routes of this module.
pub const ROUTES: &[&str] = &[
    "questionnaires.list",
    "questionnaires.create",
    "questionnaires.get",
    "questionnaires.update",
    "questionnaires.delete",
    "questionnaires.rename",
    "questionnaires.duplicate",
    "questionnaires.listSubmissions",
    "questionnaires.getSubmission",
    "questionnaires.deleteSubmission",
    "questionnaires.getSubmissionStats",
];

/// `questionnaireNameSchema`.
const NAME: Str = Str::trimmed(1, 255);
/// `noQuestionnaireTriggerFlowValue`.
const NO_TRIGGER_FLOW: &str = "__none__";

fn question_json(row: &QuestionnaireQuestion) -> Value {
    json!({
        "id": id(row.id),
        "title": row.title,
        "type": row.r#type,
        "active": row.active,
        "image": row.image.as_ref().map(|j| j.0.clone()),
        "orderNo": row.order_no,
        "point": row.point,
        "retryMessage": row.retry_message,
        "customFieldId": opt_id(row.custom_field_id),
        "systemFieldKey": row.system_field_key,
        "config": row.config.as_ref().map(|j| config_json(&j.0)),
    })
}

/// `questionnaireQuestionConfig`: `{ options? }` with the option keys only.
fn config_json(config: &Value) -> Value {
    let mut out = Map::new();
    if let Some(options) = config.get("options").and_then(Value::as_array) {
        let options: Vec<Value> = options
            .iter()
            .map(|o| json!({ "id": o.get("id"), "label": o.get("label"), "points": o.get("points") }))
            .collect();
        out.insert("options".into(), Value::Array(options));
    }
    Value::Object(out)
}

fn questionnaire_json(row: &Questionnaire, questions: &[QuestionnaireQuestion]) -> Value {
    json!({
        "id": id(row.id),
        "name": row.name,
        "triggerFlowId": opt_id(row.trigger_flow_id),
        "enableScore": row.enable_score,
        "enableRetryMessages": row.enable_retry_messages,
        "enableCustomFieldMapping": row.enable_custom_field_mapping,
        "createdAt": date(row.created_at),
        "updatedAt": date(row.updated_at),
        "questions": questions.iter().map(question_json).collect::<Vec<_>>(),
    })
}

fn contact_json(contact: &SubmissionContact) -> Value {
    json!({
        "id": id(contact.id),
        "fullName": contact.full_name,
        "firstName": contact.first_name,
        "lastName": contact.last_name,
        "email": contact.email,
        "phoneNumber": contact.phone_number,
        "avatar": contact.avatar,
    })
}

fn at(path: &[Seg], index: usize) -> Path {
    let mut next = path.to_vec();
    next.push(Seg::Index(index));
    next
}

/// `z.string().trim().optional().nullable()`.
fn nullish_text(
    input: &mut Input,
    path: &[Seg],
    value: Option<&Value>,
    spec: Str,
) -> Option<String> {
    match value {
        None | Some(Value::Null) => None,
        Some(other) => input.string_at(path, Some(other), spec, true),
    }
}

/// `questionnaireQuestionImageSchema.optional().nullable()`.
fn image(input: &mut Input, path: &[Seg], value: Option<&Value>) -> Option<Value> {
    let value = value.filter(|v| !v.is_null())?;
    let Some(obj) = value.as_object() else {
        input.issue(path, "invalid_type", "Invalid input: expected object");
        return None;
    };
    let mode = input.enum_at(
        &child(path, "mode"),
        obj.get("mode"),
        &["file", "url"],
        true,
    );
    let url = input.string_at(&child(path, "url"), obj.get("url"), Str::default(), true);
    if let Some(url) = &url
        && !url.is_empty()
        && !chatbotx_core::utils::zod::is_url(url)
    {
        input.issue(&child(path, "url"), "invalid_format", "Invalid URL");
        return None;
    }
    Some(json!({ "mode": mode?, "url": url? }))
}

/// `questionnaireOptionRequest` items (`points` default 0).
fn config(input: &mut Input, path: &[Seg], value: Option<&Value>) -> Option<Value> {
    let value = value.filter(|v| !v.is_null())?;
    let Some(obj) = value.as_object() else {
        input.issue(path, "invalid_type", "Invalid input: expected object");
        return None;
    };
    let options_path = child(path, "options");
    let mut options = Vec::new();
    match obj.get("options") {
        None => {}
        Some(Value::Array(items)) => {
            for (index, item) in items.iter().enumerate() {
                let item_path = at(&options_path, index);
                let Some(item) = item.as_object() else {
                    input.issue(&item_path, "invalid_type", "Invalid input: expected object");
                    continue;
                };
                let option_id = input.string_at(
                    &child(&item_path, "id"),
                    item.get("id"),
                    Str::trimmed(1, usize::MAX),
                    true,
                );
                let label = input.string_at(
                    &child(&item_path, "label"),
                    item.get("label"),
                    Str::trimmed(1, 255),
                    true,
                );
                let points = match item.get("points") {
                    None => Some(0.0),
                    Some(v) => input.coerce_number_at(
                        &child(&item_path, "points"),
                        Some(v),
                        Num {
                            int: true,
                            min: Some(0.0),
                            max: None,
                        },
                        true,
                    ),
                };
                if let (Some(option_id), Some(label), Some(points)) = (option_id, label, points) {
                    options
                        .push(json!({ "id": option_id, "label": label, "points": points as i64 }));
                }
            }
        }
        Some(_) => input.issue(
            &options_path,
            "invalid_type",
            "Invalid input: expected array",
        ),
    }
    Some(json!({ "options": options }))
}

/// `questionnaireQuestionRequest`.
fn question(input: &mut Input, path: &[Seg], value: &Value) -> Option<QuestionInput> {
    let Some(obj) = value.as_object() else {
        input.issue(path, "invalid_type", "Invalid input: expected object");
        return None;
    };
    let question_id = match obj.get("id") {
        None => Some(None),
        Some(v) => input.id_at(&child(path, "id"), Some(v), true).map(Some),
    };
    let title = input.string_at(
        &child(path, "title"),
        obj.get("title"),
        Str::trimmed(1, 1000),
        true,
    );
    let kind = input.enum_at(
        &child(path, "type"),
        obj.get("type"),
        SUPPORTED_QUESTION_TYPES,
        true,
    );
    let active = input.bool_at(&child(path, "active"), obj.get("active"), true);
    let image = image(input, &child(path, "image"), obj.get("image"));
    let point = match obj.get("point") {
        None => Some(1.0),
        Some(v) => input.coerce_number_at(
            &child(path, "point"),
            Some(v),
            Num {
                int: true,
                min: Some(0.0),
                max: None,
            },
            true,
        ),
    };
    let retry_message = nullish_text(
        input,
        &child(path, "retryMessage"),
        obj.get("retryMessage"),
        Str::trimmed(0, 1000),
    );
    let custom_field_id = nullish_text(
        input,
        &child(path, "customFieldId"),
        obj.get("customFieldId"),
        Str::trimmed(0, usize::MAX),
    );
    let system_field_key = nullish_text(
        input,
        &child(path, "systemFieldKey"),
        obj.get("systemFieldKey"),
        Str::trimmed(0, usize::MAX),
    );
    let config = config(input, &child(path, "config"), obj.get("config"));
    Some(QuestionInput {
        id: question_id?,
        title: title?,
        r#type: kind?,
        active: active?,
        image,
        point: Some(point? as i64),
        retry_message,
        custom_field_id,
        system_field_key,
        config,
    })
}

/// `updateQuestionnairePublicRequest`.
fn update_input(input: &mut Input, workspace_id: i64) -> Option<UpdateQuestionnaire> {
    let questionnaire_id = input.id("id");
    let trigger_flow_id = match input.get("triggerFlowId").cloned() {
        None | Some(Value::Null) => Some(None),
        Some(Value::String(text)) if text == NO_TRIGGER_FLOW => Some(None),
        Some(other) => input
            .id_at(&key("triggerFlowId"), Some(&other), true)
            .map(Some),
    };
    let enable_score = input.bool("enableScore");
    let enable_retry_messages = input.bool("enableRetryMessages");
    let enable_custom_field_mapping = input.bool("enableCustomFieldMapping");
    let mut questions = Vec::new();
    match input.get("questions").cloned() {
        Some(Value::Array(items)) => {
            if items.len() > 100 {
                input.issue(
                    &key("questions"),
                    "too_big",
                    "Too big: expected array to have <=100 items",
                );
            }
            for (index, item) in items.iter().enumerate() {
                if let Some(parsed) = question(input, &at(&key("questions"), index), item) {
                    questions.push(parsed);
                }
            }
        }
        None => input.issue(
            &key("questions"),
            "invalid_type",
            "Invalid input: expected array, received undefined",
        ),
        Some(_) => input.issue(
            &key("questions"),
            "invalid_type",
            "Invalid input: expected array",
        ),
    }
    Some(UpdateQuestionnaire {
        workspace_id,
        id: questionnaire_id?,
        trigger_flow_id: trigger_flow_id?,
        enable_score: enable_score?,
        enable_retry_messages: enable_retry_messages?,
        enable_custom_field_mapping: enable_custom_field_mapping?,
        questions,
    })
}

/// Runs one of [`ROUTES`].
pub async fn handle(state: &ApiState, route_id: &str, call: Call<'_>) -> Reply {
    let db = &state.db;
    let ws = call.workspace_id;
    match route_id {
        "questionnaires.list" => {
            let mut input = Input::from_query(call.params, call.query);
            let (page, per_page) = public_paging(&mut input);
            let name = input.opt_string("name", NAME);
            let sort = sort_query(&mut input, Some(&["name"]));
            input.finish()?;
            let page = service::list(
                db,
                ListQuestionnaires {
                    workspace_id: ws,
                    page: Some(page),
                    per_page: Some(per_page),
                    name,
                    sort,
                },
            )
            .await
            .map_err(|e| business_error(e, super::LISTING))?;
            let data: Vec<Value> = page
                .data
                .iter()
                .map(|row| {
                    json!({
                        "id": id(row.questionnaire.id),
                        "name": row.questionnaire.name,
                        "createdAt": date(row.questionnaire.created_at),
                        "updatedAt": date(row.questionnaire.updated_at),
                        "applicantsCount": row.applicants_count,
                    })
                })
                .collect();
            Ok(json(
                200,
                json!({ "data": data, "pageCount": page.page_count }),
            ))
        }
        "questionnaires.get" => {
            let mut input = Input::from_query(call.params, call.query);
            let questionnaire_id = input.id("id");
            input.finish()?;
            service::get_for_edit(db, ws, questionnaire_id.unwrap_or_default())
                .await
                .map(|found| {
                    json(
                        200,
                        questionnaire_json(&found.questionnaire, &found.questions),
                    )
                })
                .map_err(|e| business_error(e, super::FINDING))
        }
        "questionnaires.create" => {
            let mut input = Input::from_body(call.params, call.body);
            let name = input.string("name", NAME);
            input.finish()?;
            service::create(db, ws, &name.unwrap_or_default())
                .await
                .map(|new_id| json(201, json!({ "id": id(new_id) })))
                .map_err(|e| business_error(e, super::CREATING))
        }
        "questionnaires.update" => {
            let mut input = Input::from_body(call.params, call.body);
            let parsed = update_input(&mut input, ws);
            input.finish()?;
            let Some(parsed) = parsed else {
                return Err(super::internal_error("questionnaire update input"));
            };
            service::update(db, parsed)
                .await
                .map(|()| no_content())
                .map_err(|e| business_error(e, super::MUTATING))
        }
        "questionnaires.delete" => {
            let mut input = Input::from_query(call.params, call.query);
            let questionnaire_id = input.id("id");
            input.finish()?;
            service::delete_many(db, ws, vec![questionnaire_id.unwrap_or_default()])
                .await
                .map(|()| no_content())
                .map_err(|e| business_error(e, super::MUTATING))
        }
        "questionnaires.rename" => {
            let mut input = Input::from_body(call.params, call.body);
            let questionnaire_id = input.id("id");
            let name = input.string("name", NAME);
            input.finish()?;
            service::rename(
                db,
                ws,
                questionnaire_id.unwrap_or_default(),
                &name.unwrap_or_default(),
            )
            .await
            .map(|()| no_content())
            .map_err(|e| business_error(e, super::MUTATING))
        }
        "questionnaires.duplicate" => {
            let mut input = Input::from_body(call.params, call.body);
            let questionnaire_id = input.id("id");
            input.finish()?;
            service::duplicate(db, ws, questionnaire_id.unwrap_or_default())
                .await
                .map(|new_id| json(201, json!({ "id": id(new_id) })))
                .map_err(|e| business_error(e, super::MUTATING))
        }
        "questionnaires.listSubmissions" => {
            let mut input = Input::from_query(call.params, call.query);
            let questionnaire_id = input.id("id");
            let (page, per_page) = public_paging(&mut input);
            let name = input.opt_string("name", NAME);
            let sort = sort_query(
                &mut input,
                Some(&["name", "totalPoints", "status", "completedAt"]),
            );
            input.finish()?;
            let page = submission::list(
                db,
                ListSubmissions {
                    workspace_id: ws,
                    questionnaire_id: questionnaire_id.unwrap_or_default(),
                    page: Some(page),
                    per_page: Some(per_page),
                    name,
                    sort,
                },
            )
            .await
            .map_err(|e| business_error(e, super::FINDING))?;
            let data: Vec<Value> = page
                .data
                .iter()
                .map(|row| {
                    json!({
                        "id": id(row.id),
                        "status": row.status,
                        "totalPoints": row.total_points,
                        "completedAt": opt_date(row.completed_at),
                        "conversationId": opt_id(row.conversation_id),
                        "contact": contact_json(&row.contact),
                    })
                })
                .collect();
            Ok(json(
                200,
                json!({ "enableScore": page.enable_score, "data": data, "pageCount": page.page_count }),
            ))
        }
        "questionnaires.getSubmission" => {
            let mut input = Input::from_query(call.params, call.query);
            let questionnaire_id = input.id("id");
            let submission_id = input.id("submissionId");
            input.finish()?;
            let found = submission::detail(
                db,
                ws,
                questionnaire_id.unwrap_or_default(),
                submission_id.unwrap_or_default(),
            )
            .await
            .map_err(|e| business_error(e, super::FINDING))?;
            let answers: Vec<Value> = found
                .answers
                .iter()
                .map(|answer| {
                    json!({
                        "questionId": answer.question_id,
                        "label": answer.label,
                        "value": answer.value,
                        "pointsEarned": answer.points_earned,
                    })
                })
                .collect();
            Ok(json(
                200,
                json!({
                    "id": id(found.id),
                    "contact": { "id": id(found.contact.id), "fullName": found.contact.full_name },
                    "conversationId": opt_id(found.conversation_id),
                    "status": found.status,
                    "totalPoints": found.total_points,
                    "answers": answers,
                }),
            ))
        }
        "questionnaires.deleteSubmission" => {
            let mut input = Input::from_query(call.params, call.query);
            let questionnaire_id = input.id("id");
            let submission_id = input.id("submissionId");
            input.finish()?;
            submission::delete_submission(
                db,
                ws,
                questionnaire_id.unwrap_or_default(),
                submission_id.unwrap_or_default(),
            )
            .await
            .map(|()| no_content())
            .map_err(|e| business_error(e, super::MUTATING))
        }
        "questionnaires.getSubmissionStats" => {
            let mut input = Input::from_query(call.params, call.query);
            let questionnaire_id = input.id("id");
            input.finish()?;
            submission::dashboard(db, ws, questionnaire_id.unwrap_or_default())
                .await
                .map(|stats| {
                    json(
                        200,
                        json!({
                            "totalApplicants": stats.total_applicants,
                            "completed": stats.completed,
                            "completionRate": stats.completion_rate,
                        }),
                    )
                })
                .map_err(|e| business_error(e, super::FINDING))
        }
        _ => Err(crate::routes::not_ported_error()),
    }
}
