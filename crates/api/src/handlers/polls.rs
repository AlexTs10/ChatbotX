//! `features/polls/api/public.ts` (+ `lib/operations.ts`, `lib/serialize.ts`,
//! `lib/poll-links.ts`): `polls.*`.

use chatbotx_business::inbox::links::list_connected_link_inboxes;
use chatbotx_business::poll::service::{
    self, CreatePoll, ListPolls, PollWithVoteCounts, UpdatePoll, UpdatePollCandidate,
};
use chatbotx_business::poll::{build_poll_links, get_poll_state};
use chatbotx_business::tx;
use chatbotx_db::repo::models::Poll;
use chatbotx_db::repo::now_ms;
use chatbotx_db::repo::poll::PollCandidateInput;
use serde_json::{Value, json};

use super::input::{Input, Num, Path, Seg, Str, child, key};
use super::{ApiState, Call, Reply, business_error, date, id, json, no_content, number, opt_id};

/// The routes of this module.
pub const ROUTES: &[&str] = &[
    "polls.list",
    "polls.get",
    "polls.create",
    "polls.update",
    "polls.delete",
    "polls.bulkDelete",
    "polls.getLinks",
    "polls.getAnalytics",
];

const POLL_NAME_MAX: usize = 80;
const POLL_CANDIDATE_NAME_MAX: usize = 60;
const POLL_CANDIDATE_DESCRIPTION_MAX: usize = 500;
/// `maxLimit` of `publicListRequest`.
const PUBLIC_LIST_MAX_PER_PAGE: f64 = 50.0;
const POLL_FIELDS: &[&str] = &[
    "name",
    "imageUrl",
    "startsAt",
    "endsAt",
    "afterVoteFlowId",
    "candidates",
];

/// `toBase` of `lib/serialize.ts`.
fn poll_base(poll: &Poll, now: i64) -> serde_json::Map<String, Value> {
    let mut out = serde_json::Map::new();
    out.insert("id".into(), id(poll.id));
    out.insert("name".into(), json!(poll.name));
    out.insert("imageUrl".into(), json!(poll.image_url));
    out.insert("startsAt".into(), date(poll.starts_at));
    out.insert("endsAt".into(), date(poll.ends_at));
    out.insert("afterVoteFlowId".into(), opt_id(poll.after_vote_flow_id));
    out.insert(
        "state".into(),
        json!(get_poll_state(poll.starts_at, poll.ends_at, now).as_str()),
    );
    out.insert("createdAt".into(), date(poll.created_at));
    out.insert("updatedAt".into(), date(poll.updated_at));
    out
}

/// `toPollResource`.
fn poll_json(poll: &PollWithVoteCounts) -> Value {
    let mut out = poll_base(&poll.poll, now_ms());
    out.insert(
        "candidates".into(),
        Value::Array(
            poll.candidates
                .iter()
                .map(|c| {
                    json!({
                        "id": id(c.candidate.id),
                        "position": c.candidate.position,
                        "name": c.candidate.name,
                        "description": c.candidate.description,
                        "imageUrl": c.candidate.image_url,
                        "votesCount": c.votes_count,
                    })
                })
                .collect(),
        ),
    );
    Value::Object(out)
}

/// `z.iso.datetime({ offset: true })`: `YYYY-MM-DDTHH:MM[:SS[.s+]](Z|±HH[[:]MM])`.
fn is_iso_datetime(text: &str) -> bool {
    let b = text.as_bytes();
    let digits = |range: std::ops::Range<usize>| {
        b.get(range)
            .is_some_and(|s| s.iter().all(u8::is_ascii_digit))
    };
    if b.len() < 17
        || !digits(0..4)
        || b[4] != b'-'
        || !digits(5..7)
        || b[7] != b'-'
        || !digits(8..10)
        || b[10] != b'T'
        || !digits(11..13)
        || b[13] != b':'
        || !digits(14..16)
    {
        return false;
    }
    let mut i = 16;
    if b.get(i) == Some(&b':') {
        if !digits(i + 1..i + 3) {
            return false;
        }
        i += 3;
        if b.get(i) == Some(&b'.') {
            let start = i + 1;
            i = start;
            while b.get(i).is_some_and(u8::is_ascii_digit) {
                i += 1;
            }
            if i == start {
                return false;
            }
        }
    }
    match b.get(i) {
        Some(b'Z') => i + 1 == b.len(),
        Some(b'+' | b'-') => {
            let rest = &text[i + 1..];
            let r = rest.as_bytes();
            let all_digits = |s: &[u8]| s.iter().all(u8::is_ascii_digit);
            match r.len() {
                2 | 4 => all_digits(r),
                5 => r[2] == b':' && all_digits(&r[..2]) && all_digits(&r[3..]),
                _ => false,
            }
        }
        _ => false,
    }
}

fn datetime_at(input: &mut Input, name: &str, required: bool) -> Option<i64> {
    let value = input.get(name).cloned();
    let text = input.string_at(&key(name), value.as_ref(), Str::default(), required)?;
    let parsed = if is_iso_datetime(&text) {
        chatbotx_core::js::parse_date(&text)
    } else {
        None
    };
    if parsed.is_none() {
        input.issue(&key(name), "invalid_format", "Invalid ISO datetime");
    }
    parsed
}

fn url_at(input: &mut Input, path: &Path, value: Option<&Value>, required: bool) -> Option<String> {
    let text = input.string_at(path, value, Str::default(), required)?;
    if url::Url::parse(&text).is_err() {
        input.issue(path, "invalid_format", "Invalid URL");
        return None;
    }
    Some(text)
}

/// `pollCandidateRequest` items of `candidates`.
fn candidates_at(input: &mut Input, required: bool) -> Option<Vec<UpdatePollCandidate>> {
    let path = key("candidates");
    let items = match input.get("candidates").cloned() {
        None => {
            if required {
                input.issue(
                    &path,
                    "invalid_type",
                    "Invalid input: expected array, received undefined",
                );
            }
            return None;
        }
        Some(Value::Array(items)) => items,
        Some(_) => {
            input.issue(&path, "invalid_type", "Invalid input: expected array");
            return None;
        }
    };
    let mut out = Vec::new();
    let mut ok = true;
    for (index, item) in items.iter().enumerate() {
        let mut at = path.clone();
        at.push(Seg::Index(index));
        let Some(obj) = item.as_object() else {
            input.issue(&at, "invalid_type", "Invalid input: expected object");
            ok = false;
            continue;
        };
        let candidate_id = match obj.get("id") {
            None => None,
            Some(value) => match input.id_at(&child(&at, "id"), Some(value), true) {
                Some(v) => Some(v),
                None => {
                    ok = false;
                    None
                }
            },
        };
        let name = input.string_at(
            &child(&at, "name"),
            obj.get("name"),
            Str::trimmed(1, POLL_CANDIDATE_NAME_MAX),
            true,
        );
        let description = match obj.get("description") {
            None | Some(Value::Null) => None,
            Some(value) => input.string_at(
                &child(&at, "description"),
                Some(value),
                Str {
                    trim: true,
                    ..Str::max(POLL_CANDIDATE_DESCRIPTION_MAX)
                },
                true,
            ),
        };
        let image_url = match obj.get("imageUrl") {
            None | Some(Value::Null) => None,
            Some(value) => url_at(input, &child(&at, "imageUrl"), Some(value), true),
        };
        match name {
            Some(name) if ok => out.push(UpdatePollCandidate {
                id: candidate_id,
                candidate: PollCandidateInput {
                    name,
                    description,
                    image_url,
                },
            }),
            _ => ok = false,
        }
    }
    ok.then_some(out)
}

/// The fields of `pollFormRequest` (`partial` = `pollPatchRequest`).
struct PollForm {
    name: Option<String>,
    image_url: Option<String>,
    starts_at: Option<i64>,
    ends_at: Option<i64>,
    after_vote_flow_id: Option<Option<i64>>,
    candidates: Option<Vec<UpdatePollCandidate>>,
}

fn poll_form(input: &mut Input, partial: bool) -> PollForm {
    let required = !partial;
    let name = {
        let value = input.get("name").cloned();
        input.string_at(
            &key("name"),
            value.as_ref(),
            Str::trimmed(1, POLL_NAME_MAX),
            required,
        )
    };
    let image_url = {
        let value = input.get("imageUrl").cloned();
        url_at(input, &key("imageUrl"), value.as_ref(), required)
    };
    let starts_at = datetime_at(input, "startsAt", required);
    let ends_at = datetime_at(input, "endsAt", required);
    let after_vote_flow_id = input.nullish_id("afterVoteFlowId");
    let candidates = candidates_at(input, required);
    PollForm {
        name,
        image_url,
        starts_at,
        ends_at,
        after_vote_flow_id,
        candidates,
    }
}

/// Runs one of [`ROUTES`].
pub async fn handle(state: &ApiState, route_id: &str, call: Call<'_>) -> Reply {
    let db = &state.db;
    let ws = call.workspace_id;
    match route_id {
        "polls.list" => {
            let mut input = Input::from_query(call.params, call.query);
            let page = input.opt_coerce_int(
                "page",
                Num {
                    min: Some(1.0),
                    ..Num::default()
                },
            );
            let per_page = input.opt_coerce_int("perPage", Num::int(1.0, PUBLIC_LIST_MAX_PER_PAGE));
            let name = input.opt_string(
                "name",
                Str {
                    trim: true,
                    min: Some(1),
                    max: None,
                    pattern: None,
                },
            );
            input.finish()?;
            let page = service::list(
                db,
                ListPolls {
                    workspace_id: ws,
                    name,
                    page: Some(page.unwrap_or(1)),
                    per_page: Some(per_page.unwrap_or(50)),
                },
            )
            .await
            .map_err(|e| business_error(e, super::LISTING))?;
            let now = now_ms();
            let data: Vec<Value> = page
                .data
                .iter()
                .map(|row| {
                    let mut out = poll_base(&row.poll, now);
                    out.insert("votesCount".into(), json!(row.votes_count));
                    Value::Object(out)
                })
                .collect();
            Ok(json(
                200,
                json!({ "data": data, "pageCount": page.page_count }),
            ))
        }
        "polls.get" => {
            let mut input = Input::from_query(call.params, call.query);
            let poll_id = input.id("id");
            input.finish()?;
            service::find_with_vote_counts(db, ws, poll_id.unwrap_or_default())
                .await
                .map(|poll| json(200, poll_json(&poll)))
                .map_err(|e| business_error(e, super::FINDING))
        }
        "polls.create" => {
            let mut input = Input::from_body(call.params, call.body);
            let form = poll_form(&mut input, false);
            input.finish()?;
            let created = service::create(
                db,
                CreatePoll {
                    workspace_id: ws,
                    name: form.name.unwrap_or_default(),
                    image_url: form.image_url.unwrap_or_default(),
                    starts_at: form.starts_at.unwrap_or_default(),
                    ends_at: form.ends_at.unwrap_or_default(),
                    after_vote_flow_id: form.after_vote_flow_id.flatten(),
                    candidates: form
                        .candidates
                        .unwrap_or_default()
                        .into_iter()
                        .map(|c| c.candidate)
                        .collect(),
                },
            )
            .await
            .map_err(|e| business_error(e, super::CREATING))?;
            let poll = service::with_vote_counts(db, created)
                .await
                .map_err(|e| business_error(e, super::CREATING))?;
            Ok(json(200, poll_json(&poll)))
        }
        "polls.update" => {
            let mut input = Input::from_body(call.params, call.body);
            let form = poll_form(&mut input, true);
            let poll_id = input.id("id");
            input.require_one_of(POLL_FIELDS);
            input.finish()?;
            let updated = service::update(
                db,
                UpdatePoll {
                    workspace_id: ws,
                    id: poll_id.unwrap_or_default(),
                    name: form.name,
                    image_url: form.image_url,
                    starts_at: form.starts_at,
                    ends_at: form.ends_at,
                    after_vote_flow_id: form.after_vote_flow_id,
                    candidates: form.candidates,
                },
            )
            .await
            .map_err(|e| business_error(e, super::MUTATING))?;
            let poll = service::with_vote_counts(db, updated)
                .await
                .map_err(|e| business_error(e, super::MUTATING))?;
            Ok(json(200, poll_json(&poll)))
        }
        "polls.delete" => {
            let mut input = Input::from_query(call.params, call.query);
            let poll_id = input.id("id");
            input.finish()?;
            service::delete(db, ws, poll_id.unwrap_or_default())
                .await
                .map(|()| no_content())
                .map_err(|e| business_error(e, super::MUTATING))
        }
        "polls.bulkDelete" => {
            let mut input = Input::from_body(call.params, call.body);
            let ids = match input.get("ids").cloned() {
                Some(Value::Array(items)) => {
                    if items.is_empty() {
                        input.issue(
                            &key("ids"),
                            "too_small",
                            "Too small: expected array to have >=1 items",
                        );
                    }
                    let mut ids = Vec::new();
                    for (index, item) in items.iter().enumerate() {
                        let path = vec![Seg::Key("ids".into()), Seg::Index(index)];
                        match item.as_str() {
                            Some(text)
                                if !text.is_empty() && text.bytes().all(|b| b.is_ascii_digit()) =>
                            {
                                if let Ok(value) = text.parse::<i64>() {
                                    ids.push(value);
                                }
                            }
                            _ => input.issue(
                                &path,
                                "invalid_format",
                                "Invalid string: must match pattern /^\\d+$/",
                            ),
                        }
                    }
                    ids
                }
                other => {
                    let received = if other.is_none() {
                        "undefined"
                    } else {
                        "other"
                    };
                    input.issue(
                        &key("ids"),
                        "invalid_type",
                        &format!("Invalid input: expected array, received {received}"),
                    );
                    Vec::new()
                }
            };
            input.finish()?;
            service::bulk_delete(db, ws, ids)
                .await
                .map(|deleted| json(200, json!({ "deleted": deleted })))
                .map_err(|e| business_error(e, super::MUTATING))
        }
        "polls.getLinks" => {
            let mut input = Input::from_query(call.params, call.query);
            let poll_id = input.id("id").unwrap_or_default();
            input.finish()?;
            service::find(db, ws, poll_id)
                .await
                .map_err(|e| business_error(e, super::FINDING))?;
            let inboxes = tx::read(db, move |conn| list_connected_link_inboxes(conn, ws))
                .await
                .map_err(|e| business_error(e, super::FINDING))?;
            let links = build_poll_links(&state.app_url, ws, &inboxes, poll_id);
            Ok(json(
                200,
                json!({
                    "omnichannelUrl": links.omnichannel_url,
                    "links": links.links.iter().map(|l| json!({
                        "inboxId": id(l.inbox_id),
                        "channel": l.channel,
                        "name": l.name,
                        "url": l.url,
                    })).collect::<Vec<_>>(),
                }),
            ))
        }
        "polls.getAnalytics" => {
            let mut input = Input::from_query(call.params, call.query);
            let poll_id = input.id("id");
            input.finish()?;
            let (poll, analytics) = service::get_analytics(db, ws, poll_id.unwrap_or_default())
                .await
                .map_err(|e| business_error(e, super::FINDING))?;
            Ok(json(
                200,
                json!({
                    "poll": {
                        "id": id(poll.id),
                        "name": poll.name,
                        "imageUrl": poll.image_url,
                        "startsAt": date(poll.starts_at),
                        "endsAt": date(poll.ends_at),
                    },
                    "totalVotes": analytics.total_votes,
                    "gender": {
                        "male": analytics.gender.male,
                        "female": analytics.gender.female,
                        "unknown": analytics.gender.unknown,
                    },
                    "candidates": analytics.candidates.iter().map(|c| json!({
                        "id": id(c.id),
                        "name": c.name,
                        "imageUrl": c.image_url,
                        "totalVotes": c.total_votes,
                        "totalPercent": number(c.total_percent),
                        "maleVotes": c.male_votes,
                        "malePercent": number(c.male_percent),
                        "femaleVotes": c.female_votes,
                        "femalePercent": number(c.female_percent),
                    })).collect::<Vec<_>>(),
                }),
            ))
        }
        _ => Err(crate::routes::not_ported_error()),
    }
}

#[cfg(test)]
mod tests {
    use super::is_iso_datetime;

    #[test]
    fn iso_datetime_with_offset() {
        assert!(is_iso_datetime("2026-01-01T00:00:00+02:00"));
        assert!(is_iso_datetime("2099-01-01T00:00:00.000Z"));
        assert!(is_iso_datetime("2026-01-01T00:00Z"));
        assert!(!is_iso_datetime("2026-01-01"));
        assert!(!is_iso_datetime("2026-01-01T00:00:00"));
    }
}
