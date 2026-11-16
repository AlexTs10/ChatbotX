//! `workspaceMembersPublicRouter` (`features/workspace-members/api/public.ts`):
//! `GET /v1/members` and `GET /v1/members/{memberId}` (P3.6-X1).

use chatbotx_business::BusinessError;
use chatbotx_business::workspace_member::{self, MemberWithUser};
use serde_json::{Map, Value, json};

use super::input::{Input, Str, key};
use super::qr_codes::public_paging;
use super::{ApiState, Call, FINDING, LISTING, Reply, business_error, date, id, json, opt_date};

/// The routes of this module.
pub const ROUTES: &[&str] = &["workspaceMembers.list", "workspaceMembers.get"];

/// `workspaceMemberPermissionsSchema` keys (the output keeps only these).
const PERMISSION_KEYS: &[&str] = &[
    "superAdmin",
    "analytics",
    "flows",
    "contacts",
    "onlyAssignedContacts",
    "emailAndPhone",
    "broadcast",
    "ecommerce",
];

fn json_column(text: &str) -> Value {
    serde_json::from_str(text).unwrap_or_else(|_| json!({}))
}

/// `workspaceMemberResource` + `user: { id, name, image }`.
pub(crate) fn member_json(row: &MemberWithUser) -> Value {
    let stored = json_column(&row.permissions);
    let permissions: Map<String, Value> = PERMISSION_KEYS
        .iter()
        .filter_map(|k| stored.get(*k).map(|v| ((*k).to_string(), v.clone())))
        .collect();
    json!({
        "id": id(row.id),
        "createdAt": date(row.created_at),
        "updatedAt": date(row.updated_at),
        "workspaceId": id(row.workspace_id),
        "userId": id(row.user_id),
        "role": row.role,
        "notificationChannels": json_column(&row.notification_channels),
        "notificationTypes": json_column(&row.notification_types),
        "permissions": Value::Object(permissions),
        "onlineSince": opt_date(row.online_since),
        "user": {
            "id": id(row.user_id),
            "name": row.user_name,
            "image": row.user_image,
        },
    })
}

/// `zodBigintAsString()` path param: digits only; beyond `i64` = no row.
fn member_id(input: &mut Input) -> Option<i64> {
    let text = input.string("memberId", Str::default())?;
    if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) {
        input.issue(
            &key("memberId"),
            "invalid_format",
            "Invalid string: must match pattern /^\\d+$/",
        );
        return None;
    }
    Some(text.parse().unwrap_or(-1))
}

/// Runs one of [`ROUTES`].
pub async fn handle(state: &ApiState, route_id: &str, call: Call<'_>) -> Reply {
    let db = &state.db;
    let ws = call.workspace_id;
    match route_id {
        "workspaceMembers.list" => {
            let mut input = Input::from_query(call.params, call.query);
            let (page, per_page) = public_paging(&mut input);
            let keyword = input
                .nullish_string("keyword", Str::max(usize::MAX))
                .flatten();
            input.finish()?;
            let page =
                workspace_member::list_paginated(db, ws, Some(page), Some(per_page), keyword)
                    .await
                    .map_err(|e| business_error(e, LISTING))?;
            let data: Vec<Value> = page.data.iter().map(member_json).collect();
            Ok(json(
                200,
                json!({ "data": data, "pageCount": page.page_count }),
            ))
        }
        "workspaceMembers.get" => {
            let mut input = Input::from_query(call.params, call.query);
            let member = member_id(&mut input);
            input.finish()?;
            let row = workspace_member::find_by_id_with_user(db, ws, member.unwrap_or(-1))
                .await
                .map_err(|e| business_error(e, FINDING))?
                .ok_or_else(|| {
                    business_error(BusinessError::not_found("Member not found"), FINDING)
                })?;
            Ok(json(200, member_json(&row)))
        }
        _ => Err(crate::routes::not_ported_error()),
    }
}
