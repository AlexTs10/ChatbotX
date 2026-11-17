//! `apps/builder/src/features/broadcasts/api/public.ts` and the
//! `previewRecurrence` route of `public-recurrence.ts` (Q.D5).
//! `listContacts` reads the delivery events (Q.D11); `skipOccurrence`,
//! `restoreOccurrence`, `listRunGroups` and `listOccurrences` need the
//! recurrence history service (Q.D5 slice 4b).

use chatbotx_business::BusinessError;
use chatbotx_business::broadcast::audience::{self, AudienceInput};
use chatbotx_business::broadcast::{self as service, history, recurrence, write};
use chatbotx_business::contact::avatar::{AvatarInbox, resolve_contact_avatar_url};
use chatbotx_business::contact_filter::{Surface, parse_criteria, to_filter_criteria};
use chatbotx_business::count_mode::CountMode;
use chatbotx_business::template_params;
use chatbotx_core::flow_config::deps::channel_types;
use chatbotx_core::flow_config::steps::messenger_template_params_schema;
use chatbotx_core::flow_config::steps::validate_wa_template_send_params;
use chatbotx_core::flow_config::steps::wa_template_params_schema;
use chatbotx_core::flow_config::z::{self, PathSegment, Schema};
use chatbotx_core::utils::recurrence::parse_recurrence_rule;
use chatbotx_db::repo::broadcast::BroadcastListInput;
use chatbotx_db::repo::now_ms;
use chatbotx_db::repo::partials::broadcast::{
    BROADCAST_SCHEDULE_TYPES, BROADCAST_STATUSES, BROADCAST_SUBACTIONS, broadcast_sends_flow,
    broadcast_sends_template, has_duplicate_broadcast_target, has_flow_and_template,
    is_audience_range_ordered, is_targets_flow_send_without_flow,
    is_targets_template_send_without_template, is_template_send_without_page,
    resolve_broadcast_audience_range,
};
use serde_json::{Map, Value, json};

use super::contacts::tenant_urls;
use super::flows::query_bool;
use super::input::{Input, Num, Str, key};
use super::qr_codes::sort_query;
use super::{
    ApiState, Call, ErrorMap, FINDING, LISTING, MUTATING, Reply, business_error, json, no_content,
};

/// The ported routes.
pub const ROUTES: &[&str] = &[
    "broadcasts.list",
    "broadcasts.get",
    "broadcasts.getAudience",
    "broadcasts.listContacts",
    "broadcasts.previewAudience",
    "broadcasts.create",
    "broadcasts.update",
    "broadcasts.updateDraft",
    "broadcasts.schedule",
    "broadcasts.moveToDraft",
    "broadcasts.stop",
    "broadcasts.resume",
    "broadcasts.resend",
    "broadcasts.duplicate",
    "broadcasts.clone",
    "broadcasts.delete",
    "broadcasts.previewRecurrence",
    "broadcasts.skipOccurrence",
    "broadcasts.restoreOccurrence",
    "broadcasts.listRunGroups",
    "broadcasts.listOccurrences",
];

/// `possibleErrorsOnCreatingBroadcast` / `possibleErrorsOnActivatingBroadcast`.
const ACTIVATING: ErrorMap = &[
    ("notFound", 404),
    ("businessError", 400),
    ("broadcastPlanLimit", 403),
    ("idempotencyKeyInvalid", 422),
    ("idempotencyKeyReused", 422),
    ("idempotencyKeyConflict", 409),
];
/// `possibleErrorsOnCountingWithBody`.
const COUNTING: ErrorMap = &[("businessError", 400), ("countTimeout", 503)];

/// `TOKEN_CALLER_CAN_VIEW_EMAIL_AND_PHONE`: a token caller is fully privileged.
const TOKEN_CALLER_CAN_VIEW_EMAIL_AND_PHONE: bool = true;
const FUTURE_SCHEDULE_MESSAGE: &str = "Schedules must be after now.";

/// The keys of `publicBroadcastResource`.
const PUBLIC_KEYS: &[&str] = &[
    "id",
    "name",
    "status",
    "schedulesType",
    "schedulesAt",
    "flowId",
    "contactCount",
    "audienceRangeStart",
    "audienceRangeEnd",
    "sendRatePerMinute",
    "parentBroadcastId",
    "occurrenceAt",
    "recurrence",
    "parent",
    "channel",
    "subaction",
    "targetMode",
    "templateId",
    "templateData",
    "contactFilter",
    "integrationWhatsappId",
    "integrationMessengerId",
    "targets",
];
const PUBLIC_RECURRENCE_KEYS: &[&str] = &[
    "id",
    "rule",
    "nextRunAt",
    "nextSendAt",
    "occurrenceCount",
    "lastOccurrenceAt",
    "pausedAt",
    "endedAt",
    "upcomingRuns",
];
const PUBLIC_TARGET_KEYS: &[&str] = &[
    "inboxId",
    "flowId",
    "templateId",
    "templateData",
    "inbox",
    "flow",
];

fn pick(value: &Value, keys: &[&str]) -> Value {
    let Some(obj) = value.as_object() else {
        return value.clone();
    };
    Value::Object(
        keys.iter()
            .filter_map(|k| obj.get(*k).map(|v| ((*k).to_owned(), v.clone())))
            .collect(),
    )
}

/// `publicBroadcastResource.parse` (zod strips the undeclared keys).
pub(crate) fn public_resource(value: &Value) -> Value {
    let mut out = pick(value, PUBLIC_KEYS);
    if let Some(rec) = out.get("recurrence").filter(|r| r.is_object()).cloned() {
        out["recurrence"] = pick(&rec, PUBLIC_RECURRENCE_KEYS);
    }
    if let Some(targets) = out.get("targets").and_then(Value::as_array).cloned() {
        out["targets"] = targets
            .iter()
            .map(|t| pick(t, PUBLIC_TARGET_KEYS))
            .collect();
    }
    out
}

/// `zodBigintAsString()`.
fn bigint() -> Schema {
    z::string().regex(r"^\d+$", "")
}

/// `broadcastSendLimitSchema` fields.
fn position() -> Schema {
    z::number().int().gte(1.0).nullish()
}
pub(crate) fn send_rate() -> Schema {
    z::number().int().gte(1.0).lte(1000.0).nullish()
}

/// `recurrenceRuleSchema` (the hand-written parser of `utils/recurrence`).
pub(crate) fn recurrence_rule() -> Schema {
    z::any().super_refine(|value, ctx| {
        if let Err(issues) = parse_recurrence_rule(value) {
            for issue in issues {
                let path = issue
                    .path
                    .split('.')
                    .filter(|s| !s.is_empty())
                    .map(|s| {
                        s.parse::<usize>()
                            .map_or_else(|_| PathSegment::from(s), PathSegment::from)
                    })
                    .collect();
                ctx.issue(path, issue.message);
            }
        }
    })
}

/// `broadcastTemplateDataSchema`.
fn template_data() -> Schema {
    z::union(vec![
        wa_template_params_schema(),
        messenger_template_params_schema(),
    ])
}

fn buttons() -> Schema {
    z::array(z::object(vec![
        ("id", z::string()),
        ("label", z::string()),
        ("flowId", z::string().optional()),
    ]))
}

fn template_params() -> Schema {
    z::record(z::string(), z::string()).optional()
}

/// `broadcastTargetSchema.extend({ templateParams })`.
fn target() -> Schema {
    z::object(vec![
        ("inboxId", bigint()),
        ("flowId", bigint().optional()),
        ("templateId", bigint().optional()),
        ("templateData", template_data().optional()),
        ("buttons", buttons().optional()),
        ("templateParams", template_params()),
    ])
}

/// `normalizeScheduleTime(value) > now`.
fn is_future_schedule_time(value: &str) -> bool {
    parse_time(value).is_some_and(|ms| write::start_of_minute(ms) > now_ms())
}

/// `new Date(value)` of an ISO string.
pub(crate) fn parse_time(value: &str) -> Option<i64> {
    chatbotx_core::js::parse_date(value)
}

fn rule(
    f: fn(&Value) -> bool,
    message: &'static str,
    path: &'static str,
) -> (fn(&Value) -> bool, &'static str, &'static str) {
    (f, message, path)
}

/// `createBroadcastPublicRequest` (`withBroadcastRules(createBroadcastFields...)`)
/// without `contactFilter` (parsed by [`parse_contact_filter`]).
fn create_request() -> Schema {
    let rules = [
        rule(
            |d| {
                is_audience_range_ordered(
                    d.get("audienceRangeStart").and_then(Value::as_i64),
                    d.get("audienceRangeEnd").and_then(Value::as_i64),
                )
            },
            "broadcastSendLimit.rangeEndBeforeStart",
            "audienceRange",
        ),
        rule(
            |d| broadcast_sends_flow(d) || broadcast_sends_template(d),
            "Either flow or template must be selected",
            "flowId",
        ),
        rule(
            |d| !has_flow_and_template(d),
            "A broadcast sends either a flow or a template, not both",
            "flowId",
        ),
        rule(
            |d| {
                (d.get("schedulesType").and_then(Value::as_str) == Some("recurring"))
                    == d.get("recurrence").is_some_and(|r| !r.is_null())
            },
            "Repeat settings are required for a repeating broadcast, and only for one",
            "recurrence",
        ),
        rule(
            |d| !is_targets_template_send_without_template(d),
            "Select a template for at least one page",
            "targets",
        ),
        rule(
            |d| !is_targets_flow_send_without_flow(d),
            "Select a flow for at least one page",
            "targets",
        ),
        rule(
            |d| !has_duplicate_broadcast_target(d),
            "A page can only be selected once",
            "targets",
        ),
        rule(
            |d| !is_template_send_without_page(d),
            "Select the page the template belongs to",
            "inboxIds",
        ),
    ];
    let mut schema = z::object(vec![
        ("channel", channel_types()),
        ("flowId", bigint().optional()),
        ("templateId", bigint().optional()),
        ("integrationWhatsappId", bigint().optional()),
        ("integrationMessengerId", bigint().optional()),
        ("templateData", template_data().optional()),
        ("buttons", buttons().optional()),
        ("targets", z::array(target()).optional()),
        ("inboxIds", z::array(bigint()).optional()),
        ("subaction", z::enum_(BROADCAST_SUBACTIONS)),
        ("schedulesType", z::enum_(BROADCAST_SCHEDULE_TYPES)),
        ("recurrence", recurrence_rule().optional()),
        ("schedulesAt", z::string().nullable()),
        ("contactFilter", z::any()),
        ("audienceRangeStart", position()),
        ("audienceRangeEnd", position()),
        ("sendRatePerMinute", send_rate()),
        ("saveAsDraft", z::boolean().optional()),
        ("templateParams", template_params()),
    ]);
    for (f, message, path) in rules {
        schema = schema.refine(f, Some(message), Some(&[path]));
    }
    schema
        .super_refine(|d, ctx| {
            let draft = d.get("saveAsDraft").and_then(Value::as_bool) == Some(true);
            let at = d.get("schedulesAt").and_then(Value::as_str);
            if !draft
                && d.get("schedulesType").and_then(Value::as_str) == Some("future")
                && !at.is_some_and(is_future_schedule_time)
            {
                ctx.issue(vec!["schedulesAt".into()], FUTURE_SCHEDULE_MESSAGE);
            }
        })
        .super_refine(|d, ctx| {
            if d.get("channel").and_then(Value::as_str) != Some("whatsapp") {
                return;
            }
            if let Some(data) = d.get("templateData").filter(|v| !v.is_null()) {
                validate_wa_template_send_params(data, ctx, &["templateData".into()]);
            }
            for (index, target) in d
                .get("targets")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .enumerate()
            {
                if let Some(data) = target.get("templateData").filter(|v| !v.is_null()) {
                    validate_wa_template_send_params(
                        data,
                        ctx,
                        &["targets".into(), index.into(), "templateData".into()],
                    );
                }
            }
        })
        .refine(
            |d| {
                let both = |e: &Value| {
                    e.get("templateParams").is_some() && e.get("templateData").is_some()
                };
                !(both(d)
                    || d.get("targets")
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                        .any(both))
            },
            Some("Send templateParams or templateData, not both"),
            Some(&["templateParams"]),
        )
}

/// `scheduleBroadcastSchema`.
pub(crate) fn schedule_request() -> Schema {
    z::object(vec![
        ("schedulesType", z::enum_(BROADCAST_SCHEDULE_TYPES)),
        ("schedulesAt", z::string().nullable()),
        ("recurrence", recurrence_rule().optional()),
        ("sendRatePerMinute", send_rate()),
    ])
    .super_refine(|d, ctx| {
        let at = d.get("schedulesAt").and_then(Value::as_str);
        let schedules_type = d.get("schedulesType").and_then(Value::as_str);
        if schedules_type == Some("future") && !at.is_some_and(is_future_schedule_time) {
            ctx.issue(vec!["schedulesAt".into()], FUTURE_SCHEDULE_MESSAGE);
        }
        if d.get("recurrence").is_some_and(|r| !r.is_null()) && schedules_type != Some("recurring")
        {
            ctx.issue(
                vec!["recurrence".into()],
                "Repeat settings are only for a repeating broadcast",
            );
        }
    })
}

/// Runs `schema` on the body; the issues go to `input`.
fn parse_with(input: &mut Input, schema: Schema) -> Value {
    let parsed = schema.safe_parse(&Value::Object(input.obj.clone()));
    if !parsed.success {
        let issues = parsed
            .issues()
            .iter()
            .filter_map(|i| serde_json::to_value(i).ok())
            .collect();
        input.extend_issues(issues);
    }
    parsed.data.unwrap_or(Value::Null)
}

/// `contactFilterRequest.shape.contactFilter` (required).
fn parse_contact_filter(input: &mut Input, required: bool) -> Option<Value> {
    let value = match input.get("contactFilter").cloned() {
        None if !required => return None,
        other => other.unwrap_or(Value::Null),
    };
    match parse_criteria(&value, Surface::Contacts) {
        Ok(parsed) => Some(parsed),
        Err(issues) => {
            let issues = issues
                .into_iter()
                .map(|mut issue| {
                    let mut path = vec![json!("contactFilter")];
                    path.extend(issue["path"].as_array().cloned().unwrap_or_default());
                    issue["path"] = Value::Array(path);
                    issue
                })
                .collect();
            input.extend_issues(issues);
            None
        }
    }
}

/// The `/api/v1` create / updateDraft body: `createBroadcastRequest` with
/// the contact filter; the issues go to `input`.
fn parse_payload(input: &mut Input) -> Value {
    let mut data = parse_with(input, create_request());
    let filter = parse_contact_filter(input, true);
    if let (Some(obj), Some(filter)) = (data.as_object_mut(), filter) {
        obj.insert("contactFilter".into(), filter);
    }
    data
}

/// The create / updateDraft payload: schema, contact filter and
/// `createBroadcastRequest` of the builder actions (no `templateParams`):
/// the parsed payload, `None` when it fails.
pub(crate) fn parse_action_payload(value: &Value) -> Option<Value> {
    let filter = parse_criteria(
        value.get("contactFilter").unwrap_or(&Value::Null),
        Surface::Contacts,
    )
    .ok()?;
    let mut body = value.clone();
    if let Some(obj) = body.as_object_mut() {
        obj.remove("templateParams");
        if let Some(targets) = obj.get_mut("targets").and_then(Value::as_array_mut) {
            targets
                .iter_mut()
                .filter_map(Value::as_object_mut)
                .for_each(|t| {
                    t.remove("templateParams");
                });
        }
    }
    let parsed = create_request().safe_parse(&body);
    let mut data = parsed.data.filter(|_| parsed.success)?;
    data["contactFilter"] = filter;
    Some(data)
}

/// The schedule time of `resolveScheduleTime` (minute start).
pub(crate) fn resolve_schedule_time(data: &Value) -> i64 {
    let at = match data["schedulesAt"].as_str() {
        Some(text) if data["schedulesType"].as_str() == Some("future") => {
            parse_time(text).unwrap_or_else(now_ms)
        }
        _ => now_ms(),
    };
    write::start_of_minute(at)
}

/// `resolvePublicBroadcastTemplateParams`: flat `templateParams` (top
/// level and per target) become the nested `templateData`.
async fn resolve_template_params(
    db: &chatbotx_db::Db,
    workspace_id: i64,
    mut data: Value,
) -> Result<Value, BusinessError> {
    async fn resolve(
        db: &chatbotx_db::Db,
        workspace_id: i64,
        channel: &str,
        entry: &mut Value,
        field: String,
    ) -> Result<(), BusinessError> {
        let Some(obj) = entry.as_object_mut() else {
            return Ok(());
        };
        let Some(params) = obj.remove("templateParams").filter(|p| !p.is_null()) else {
            return Ok(());
        };
        let Some(template_id) = obj
            .get("templateId")
            .and_then(Value::as_str)
            .and_then(|id| id.parse().ok())
        else {
            return Err(BusinessError::validation(
                &field,
                "templateParams needs a templateId",
            ));
        };
        let values = serde_json::from_value(params).unwrap_or_default();
        let resolved = template_params::resolve_template_params(
            db,
            workspace_id,
            channel,
            template_id,
            &values,
            &field,
        )
        .await?;
        obj.insert("templateData".into(), resolved);
        Ok(())
    }
    let channel = data["channel"].as_str().unwrap_or_default().to_owned();
    resolve(
        db,
        workspace_id,
        &channel,
        &mut data,
        "templateParams".into(),
    )
    .await?;
    if let Some(targets) = data.get_mut("targets").and_then(Value::as_array_mut) {
        for (index, target) in targets.iter_mut().enumerate() {
            resolve(
                db,
                workspace_id,
                &channel,
                target,
                format!("targets.{index}.templateParams"),
            )
            .await?;
        }
    }
    Ok(data)
}

/// `broadcastEventType` (message + flow event types).
pub(crate) const BROADCAST_EVENT_TYPES: &[&str] = &[
    "message:sent",
    "message:delivered",
    "message:seen",
    "message:failed",
    "message:received",
    "flow:clicked",
    "flow:ref",
];

/// `eventType` (required), `page`, `perPage` (≤ 50) of the contacts lists.
pub(crate) fn contacts_input(input: &mut Input) -> (Option<String>, Option<i64>, Option<i64>) {
    let value = input.get("eventType").cloned();
    let event_type = input.enum_at(
        &key("eventType"),
        value.as_ref(),
        BROADCAST_EVENT_TYPES,
        true,
    );
    let page = input.opt_coerce_int(
        "page",
        Num {
            min: Some(1.0),
            ..Num::default()
        },
    );
    let per_page = input.opt_coerce_int("perPage", Num::int(1.0, 50.0));
    (event_type, page, per_page)
}

fn broadcast_id(input: &mut Input) -> i64 {
    input.digits_id("id").unwrap_or_default()
}

fn id_only(call: &Call<'_>) -> Result<i64, crate::orpc::OrpcError> {
    let mut input = Input::from_body(call.params, call.body);
    let id = broadcast_id(&mut input);
    input.finish()?;
    Ok(id)
}

/// `listBroadcastsPublicRequest`.
fn list_input(input: &mut Input, workspace_id: i64) -> BroadcastListInput {
    let page = input.opt_coerce_int(
        "page",
        Num {
            min: Some(1.0),
            ..Num::default()
        },
    );
    let per_page = input.opt_coerce_int("perPage", Num::int(1.0, 50.0));
    let status = input
        .get("status")
        .cloned()
        .and_then(|v| input.enum_at(&key("status"), Some(&v), BROADCAST_STATUSES, false));
    let name = input.opt_string("name", Str::trimmed(1, usize::MAX));
    let channel = input.get("channel").cloned().and_then(|v| {
        input.enum_at(
            &key("channel"),
            Some(&v),
            super::contacts::CHANNEL_TYPES,
            false,
        )
    });
    let date = |input: &mut Input, name: &str| match input
        .get(name)
        .and_then(Value::as_str)
        .map(str::to_owned)
    {
        None => None,
        Some(text) => match parse_time(&text) {
            Some(ms) => Some(ms),
            None => {
                input.issue(
                    &key(name),
                    "invalid_type",
                    "Invalid input: expected date, received Date",
                );
                None
            }
        },
    };
    let scheduled_from = date(input, "scheduledFrom");
    let scheduled_to = date(input, "scheduledTo");
    let sort = sort_query(input, None);
    let schedules_type = input.get("schedulesType").cloned().and_then(|v| {
        input.enum_at(
            &key("schedulesType"),
            Some(&v),
            BROADCAST_SCHEDULE_TYPES,
            false,
        )
    });
    let parent_broadcast_id = input
        .get("parentBroadcastId")
        .is_some()
        .then(|| input.digits_id("parentBroadcastId"))
        .flatten();
    let include_runs = query_bool(input, "includeRuns");
    let exclude_templates = query_bool(input, "excludeTemplates");
    BroadcastListInput {
        workspace_id,
        name,
        status,
        page,
        per_page,
        sort: Some(sort.unwrap_or_else(|| {
            vec![chatbotx_db::repo::SortItem {
                id: "createdAt".into(),
                desc: true,
            }]
        })),
        channel,
        scheduled_from,
        scheduled_to,
        include_runs,
        exclude_templates,
        parent_broadcast_id,
        schedules_type,
    }
}

fn id_of(value: &Value, name: &str) -> Option<i64> {
    value
        .get(name)
        .and_then(Value::as_str)
        .and_then(|s| s.parse().ok())
}

/// `previewBroadcastAudiencePublicRequest`.
fn preview_input(
    input: &mut Input,
    workspace_id: i64,
) -> (AudienceInput, Option<i64>, Option<i64>, Option<CountMode>) {
    let filter = parse_contact_filter(input, false);
    let data = parse_with(
        input,
        z::object(vec![
            ("inboxIds", z::array(bigint()).optional()),
            ("channels", z::array(channel_types()).optional()),
            ("integrationWhatsappId", bigint().optional()),
            ("integrationMessengerId", bigint().optional()),
            ("subaction", z::enum_(BROADCAST_SUBACTIONS).optional()),
            ("contactFilter", z::any().optional()),
            ("audienceRangeStart", position()),
            ("audienceRangeEnd", position()),
            ("page", z::number().int().gte(1.0).optional()),
            ("perPage", z::number().int().gte(1.0).lte(50.0).optional()),
            ("countMode", z::enum_(&["exact", "bounded"]).optional()),
        ])
        .refine(
            |d| {
                is_audience_range_ordered(
                    d.get("audienceRangeStart").and_then(Value::as_i64),
                    d.get("audienceRangeEnd").and_then(Value::as_i64),
                )
            },
            Some("`audienceRangeEnd` must not be before `audienceRangeStart`"),
            Some(&["audienceRangeEnd"]),
        ),
    );
    let strings = |name: &str| {
        data.get(name).and_then(Value::as_array).map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect::<Vec<_>>()
        })
    };
    let audience = AudienceInput {
        workspace_id,
        channels: strings("channels"),
        inbox_ids: strings("inboxIds")
            .map(|ids| ids.iter().filter_map(|id| id.parse().ok()).collect()),
        integration_whatsapp_id: id_of(&data, "integrationWhatsappId"),
        integration_messenger_id: id_of(&data, "integrationMessengerId"),
        contact_filter: filter.as_ref().map(to_filter_criteria),
        can_view_email_and_phone: true,
        subaction: data
            .get("subaction")
            .and_then(Value::as_str)
            .map(str::to_owned),
        restrict_to_assigned_user_id: None,
        audience_range: resolve_broadcast_audience_range(
            data.get("audienceRangeStart").and_then(Value::as_i64),
            data.get("audienceRangeEnd").and_then(Value::as_i64),
        ),
    };
    let mode = match data.get("countMode").and_then(Value::as_str) {
        Some("bounded") => Some(CountMode::Bounded),
        Some(_) => Some(CountMode::Exact),
        None => None,
    };
    (
        audience,
        data.get("page").and_then(Value::as_i64),
        data.get("perPage").and_then(Value::as_i64),
        mode,
    )
}

/// A `null`-able rate: absent = `None`, `null` = `Some(None)`.
pub(crate) fn submitted_rate(data: &Value) -> Option<Option<i64>> {
    data.get("sendRatePerMinute").map(Value::as_i64)
}

/// `SKIP_RUNS_PER_CALL_MAX`.
const SKIP_RUNS_PER_CALL_MAX: usize = 100;

/// `z.coerce.date()`: `new Date(value)` must be valid.
pub(crate) fn coerce_date(value: &Value) -> Option<i64> {
    match value {
        Value::String(text) => chatbotx_core::js::parse_date(text),
        Value::Number(n) => n.as_f64().map(|n| n as i64),
        Value::Null => Some(0),
        Value::Bool(b) => Some(i64::from(*b)),
        _ => None,
    }
}

fn date_schema() -> Schema {
    z::any().super_refine(|value, ctx| {
        if coerce_date(value).is_none() {
            ctx.issue(Vec::new(), "Invalid input: expected date, received Date");
        }
    })
}

/// `broadcastRunDateSchema`.
pub(crate) fn run_date() -> Schema {
    z::string().regex(r"^\d{4}-\d{2}-\d{2}$", "").refine(
        |v| {
            v.as_str()
                .is_some_and(chatbotx_core::utils::datetime::is_real_calendar_date)
        },
        Some("Not a real calendar date."),
        None,
    )
}

/// `setBroadcastOccurrenceSkippedPublicRequest` (without `id`): `(dates, occurrenceAts)`.
fn skip_request(input: &mut Input) -> (Vec<String>, Vec<i64>) {
    let data = parse_with(
        input,
        z::object(vec![
            (
                "dates",
                z::array(run_date()).max(SKIP_RUNS_PER_CALL_MAX).optional(),
            ),
            (
                "occurrenceAts",
                z::array(date_schema())
                    .max(SKIP_RUNS_PER_CALL_MAX)
                    .optional(),
            ),
        ])
        .refine(
            |d| {
                let len = |k: &str| d.get(k).and_then(Value::as_array).map_or(0, Vec::len);
                len("dates") + len("occurrenceAts") > 0
            },
            Some("Give at least one date or occurrenceAt."),
            Some(&["dates"]),
        ),
    );
    let dates = data["dates"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default();
    let ats = data["occurrenceAts"]
        .as_array()
        .map(|a| a.iter().filter_map(coerce_date).collect())
        .unwrap_or_default();
    (dates, ats)
}

/// An optional `z.coerce.date()` query key.
fn opt_date(input: &mut Input, name: &str) -> Option<i64> {
    let value = input.get(name)?.clone();
    let parsed = coerce_date(&value);
    if parsed.is_none() {
        input.issue(
            &key(name),
            "invalid_type",
            "Invalid input: expected date, received Date",
        );
    }
    parsed
}

fn id_or_name_param(input: &Input) -> String {
    input
        .get("idOrName")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

/// Runs one of [`ROUTES`].
pub async fn handle(state: &ApiState, route_id: &str, call: Call<'_>) -> Reply {
    let db = &state.db;
    let ws = call.workspace_id;
    let audit = &state.audit;
    let jobs = state.jobs.as_ref();
    match route_id {
        "broadcasts.list" => {
            let mut input = Input::from_query(call.params, call.query);
            let list = list_input(&mut input, ws);
            input.finish()?;
            let mut out = service::list(db, list)
                .await
                .map_err(|e| business_error(e, LISTING))?;
            if let Some(items) = out["data"].as_array() {
                out["data"] = items.iter().map(public_resource).collect();
            }
            Ok(json(200, out))
        }
        "broadcasts.get" => {
            let input = Input::from_query(call.params, call.query);
            let id_or_name = input
                .get("idOrName")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned();
            input.finish()?;
            let found = service::find_by_id_or_name(db, ws, id_or_name)
                .await
                .map_err(|e| business_error(e, FINDING))?;
            Ok(json(200, public_resource(&found)))
        }
        "broadcasts.getAudience" => {
            let mut input = Input::from_query(call.params, call.query);
            let id_or_name = input
                .get("idOrName")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned();
            let page = input.opt_coerce_int(
                "page",
                Num {
                    min: Some(1.0),
                    ..Num::default()
                },
            );
            let per_page = input.opt_coerce_int(
                "perPage",
                Num {
                    min: Some(1.0),
                    ..Num::default()
                },
            );
            input.finish()?;
            let out = service::list_audience(db, ws, id_or_name, page, per_page)
                .await
                .map_err(|e| business_error(e, FINDING))?;
            Ok(json(200, out))
        }
        "broadcasts.listContacts" => {
            let mut input = Input::from_query(call.params, call.query);
            let id = input.digits_id("id");
            let (event_type, page, per_page) = contacts_input(&mut input);
            input.finish()?;
            let out = service::list_contacts_page(
                db,
                ws,
                id.unwrap_or_default(),
                event_type.unwrap_or_default(),
                page.unwrap_or(1),
                per_page.unwrap_or(50),
            )
            .await
            .map_err(|e| business_error(e, FINDING))?;
            Ok(json(200, out))
        }
        "broadcasts.previewAudience" => {
            let mut input = Input::from_body(call.params, call.body);
            let (audience, page, per_page, mode) = preview_input(&mut input, ws);
            input.finish()?;
            let counted = audience::count_audience(db, audience.clone(), mode)
                .await
                .map_err(|e| business_error(e, COUNTING))?;
            let rows = audience::list_audience_preview(
                db,
                audience,
                Some(page.unwrap_or(1)),
                Some(per_page.unwrap_or(20)),
            )
            .await
            .map_err(|e| business_error(e, COUNTING))?;
            let urls = tenant_urls(state);
            let data: Vec<Value> = rows
                .into_iter()
                .map(|mut row| {
                    // ponytail: the avatar resolves against this row's contact inbox only.
                    let inbox = AvatarInbox {
                        id: id_of(&row, "contactInboxId").unwrap_or_default(),
                        channel: row["channel"].as_str().unwrap_or_default().to_owned(),
                        last_message_at: None,
                    };
                    row["avatar"] = json!(resolve_contact_avatar_url(
                        ws,
                        row["avatar"].as_str(),
                        &[inbox],
                        &urls,
                        now_ms()
                    ));
                    if let Some(obj) = row.as_object_mut() {
                        let created = obj.remove("createdAt").unwrap_or(Value::Null);
                        obj.insert("occurredAt".into(), created);
                    }
                    row
                })
                .collect();
            let mut out = Map::new();
            out.insert("total".into(), json!(counted.total));
            out.insert("totalCountCapped".into(), json!(counted.total_count_capped));
            if let Some(cap) = counted.count_cap {
                out.insert("countCap".into(), json!(cap));
            }
            out.insert("data".into(), Value::Array(data));
            Ok(json(200, Value::Object(out)))
        }
        "broadcasts.create" => {
            let mut input = Input::from_body(call.params, call.body);
            let data = parse_payload(&mut input);
            input.finish()?;
            let data = resolve_template_params(db, ws, data)
                .await
                .map_err(|e| business_error(e, ACTIVATING))?;
            let created = write::create(db, audit, ws, data, TOKEN_CALLER_CAN_VIEW_EMAIL_AND_PHONE)
                .await
                .map_err(|e| business_error(e, ACTIVATING))?;
            Ok(json(201, public_resource(&created.to_json())))
        }
        "broadcasts.update" => {
            let mut input = Input::from_body(call.params, call.body);
            let name = input.string("name", Str::trimmed(1, 255));
            let id = broadcast_id(&mut input);
            input.finish()?;
            service::update(db, audit, ws, id, name.unwrap_or_default())
                .await
                .map_err(|e| business_error(e, MUTATING))?;
            let found = service::find_by_id_or_name(db, ws, id.to_string())
                .await
                .map_err(|e| business_error(e, MUTATING))?;
            Ok(json(200, public_resource(&found)))
        }
        "broadcasts.updateDraft" => {
            let mut input = Input::from_body(call.params, call.body);
            let data = parse_payload(&mut input);
            let id = broadcast_id(&mut input);
            input.finish()?;
            let data = resolve_template_params(db, ws, data)
                .await
                .map_err(|e| business_error(e, ACTIVATING))?;
            let out = write::update_draft(
                db,
                audit,
                jobs,
                ws,
                id,
                data,
                TOKEN_CALLER_CAN_VIEW_EMAIL_AND_PHONE,
            )
            .await
            .map_err(|e| business_error(e, ACTIVATING))?;
            Ok(json(200, out))
        }
        "broadcasts.schedule" => {
            let mut input = Input::from_body(call.params, call.body);
            let data = parse_with(&mut input, schedule_request());
            let id = broadcast_id(&mut input);
            input.finish()?;
            let schedules_type = data["schedulesType"]
                .as_str()
                .unwrap_or_default()
                .to_owned();
            let at = resolve_schedule_time(&data);
            let recurrence = data.get("recurrence").filter(|r| !r.is_null()).cloned();
            let out = write::schedule_draft(
                db,
                audit,
                jobs,
                ws,
                id,
                schedules_type,
                at,
                submitted_rate(&data),
                recurrence,
            )
            .await
            .map_err(|e| business_error(e, ACTIVATING))?;
            Ok(json(200, out))
        }
        "broadcasts.moveToDraft" => {
            let id = id_only(&call)?;
            let out = service::move_to_draft(db, audit, ws, id)
                .await
                .map_err(|e| business_error(e, MUTATING))?;
            Ok(json(200, out))
        }
        "broadcasts.stop" => {
            let id = id_only(&call)?;
            let out = service::stop_sending(db, audit, ws, id)
                .await
                .map_err(|e| business_error(e, MUTATING))?;
            Ok(json(200, out))
        }
        "broadcasts.resume" => {
            let mut input = Input::from_body(call.params, call.body);
            let data = parse_with(
                &mut input,
                z::object(vec![("sendRatePerMinute", send_rate())]),
            );
            let id = broadcast_id(&mut input);
            input.finish()?;
            let out = service::resume_sending(db, audit, ws, id, submitted_rate(&data))
                .await
                .map_err(|e| business_error(e, ACTIVATING))?;
            Ok(json(200, out))
        }
        "broadcasts.resend" => {
            let id = id_only(&call)?;
            let created = write::resend_with_pruning(
                db,
                audit,
                ws,
                id,
                TOKEN_CALLER_CAN_VIEW_EMAIL_AND_PHONE,
            )
            .await
            .map_err(|e| business_error(e, ACTIVATING))?;
            Ok(json(201, public_resource(&created.to_json())))
        }
        "broadcasts.duplicate" | "broadcasts.clone" => {
            let id = id_only(&call)?;
            let cloned =
                write::clone_broadcast(db, audit, ws, id, TOKEN_CALLER_CAN_VIEW_EMAIL_AND_PHONE)
                    .await
                    .map_err(|e| business_error(e, MUTATING))?;
            Ok(json(201, public_resource(&cloned.to_json())))
        }
        "broadcasts.delete" => {
            let id = id_only(&call)?;
            let (deleted, _) = service::soft_delete_broadcasts(db, audit, ws, vec![id])
                .await
                .map_err(|e| business_error(e, MUTATING))?;
            if deleted == 0 {
                return Err(business_error(
                    BusinessError::not_found("Broadcast not found or cannot be deleted"),
                    MUTATING,
                ));
            }
            Ok(no_content())
        }
        "broadcasts.previewRecurrence" => {
            let mut input = Input::from_body(call.params, call.body);
            let data = parse_with(
                &mut input,
                z::object(vec![
                    ("recurrence", recurrence_rule()),
                    (
                        "count",
                        z::number()
                            .int()
                            .gte(1.0)
                            .lte(recurrence::BROADCAST_PREVIEW_MAX_COUNT as f64)
                            .optional(),
                    ),
                ]),
            );
            input.finish()?;
            let count = data["count"].as_u64().map_or(3, |c| c as usize);
            let out = recurrence::preview_upcoming(&data["recurrence"], count, now_ms())
                .map_err(|e| business_error(e, FINDING))?;
            Ok(json(200, out))
        }
        "broadcasts.skipOccurrence" | "broadcasts.restoreOccurrence" => {
            let mut input = Input::from_body(call.params, call.body);
            let id = broadcast_id(&mut input);
            let (dates, occurrence_ats) = skip_request(&mut input);
            input.finish()?;
            let excluded = route_id == "broadcasts.skipOccurrence";
            let out = history::set_occurrence_excluded(
                db,
                audit,
                ws,
                id.to_string(),
                dates,
                occurrence_ats,
                excluded,
            )
            .await
            .map_err(|e| business_error(e, MUTATING))?;
            Ok(json(200, out))
        }
        "broadcasts.listRunGroups" => {
            let mut input = Input::from_query(call.params, call.query);
            let id_or_name = id_or_name_param(&input);
            let group_by = input
                .enum_at(
                    &key("groupBy"),
                    input.get("groupBy").cloned().as_ref(),
                    &["day", "week", "month"],
                    false,
                )
                .unwrap_or_else(|| "day".to_owned());
            let timezone = input.opt_string("timezone", Str::default());
            let limit = input.opt_coerce_int(
                "limit",
                Num {
                    int: true,
                    min: Some(1.0),
                    max: Some(31.0),
                },
            );
            let cursor = input.opt_string("cursor", Str::default());
            input.finish()?;
            let granularity = history::granularity_of(&group_by)
                .unwrap_or(chatbotx_db::repo::queries::date_bucket::Granularity::Day);
            let out = history::list_run_groups(
                db,
                ws,
                id_or_name,
                granularity,
                timezone,
                limit.unwrap_or(31),
                cursor,
            )
            .await
            .map_err(|e| business_error(e, FINDING))?;
            Ok(json(200, out))
        }
        "broadcasts.listOccurrences" => {
            let mut input = Input::from_query(call.params, call.query);
            let id_or_name = id_or_name_param(&input);
            let from = opt_date(&mut input, "from");
            let to = opt_date(&mut input, "to");
            let limit = input.opt_coerce_int(
                "limit",
                Num {
                    int: true,
                    min: Some(1.0),
                    max: Some(100.0),
                },
            );
            let cursor = input.opt_string("cursor", Str::default());
            input.finish()?;
            let out = history::list_occurrences(
                db,
                ws,
                id_or_name,
                from,
                to,
                cursor,
                limit.unwrap_or(50),
            )
            .await
            .map_err(|e| business_error(e, FINDING))?;
            Ok(json(200, out))
        }
        _ => unreachable!("not a broadcast route: {route_id}"),
    }
}
