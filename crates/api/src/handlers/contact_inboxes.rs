//! `features/contact-inboxes/api/public.ts` (`listInboxes`),
//! `features/contacts/api/public/ad-referrals.ts` (`listAdReferrals`) and
//! `features/contacts/api/public/refresh-profile.ts` (`refreshProfile`).

use base64::Engine as _;
use base64::engine::DecodePaddingMode;
use base64::engine::general_purpose::{GeneralPurpose, GeneralPurposeConfig, STANDARD};
use chatbotx_business::contact::profile_refresh::{
    RefreshOutcome, RefreshServices, SkipReason, refresh_contact_profile,
};
use chatbotx_business::contact::service::{find_public_contact_or_fail, list_contact_inboxes};
use chatbotx_business::contact_referral_event::list_contact_ad_touches;
use chatbotx_db::repo::contact_referral_event::{ContactAdTouch, ContactAdTouchCursor};
use serde_json::{Value, json};

use super::input::{Input, Num, Str};
use super::{
    ApiState, Call, ErrorMap, FINDING, MUTATING, Reply, business_error, date, id, json, opt_date,
};
use crate::orpc::OrpcError;

/// The ported routes.
pub const ROUTES: &[&str] = &[
    "contacts.listInboxes",
    "contacts.listAdReferrals",
    "contacts.refreshProfile",
];

/// `possibleErrorsOnFindingResource` + `possibleErrorsOnListingWithCursor`.
const LISTING_WITH_CURSOR: ErrorMap = &[
    ("notFound", 404),
    ("businessError", 400),
    ("invalidCursor", 400),
];

/// Runs one of [`ROUTES`].
pub async fn handle(state: &ApiState, route_id: &str, call: Call<'_>) -> Reply {
    match route_id {
        "contacts.listInboxes" => list_inboxes(state, call).await,
        "contacts.listAdReferrals" => list_ad_referrals(state, call).await,
        _ => refresh_profile(state, call).await,
    }
}

async fn list_inboxes(state: &ApiState, call: Call<'_>) -> Reply {
    let contact_id = super::contacts::resolve(state, &call, FINDING).await?;
    let rows = list_contact_inboxes(&state.db, call.workspace_id, contact_id)
        .await
        .map_err(|e| business_error(e, FINDING))?;
    let data: Vec<Value> = rows
        .iter()
        .map(|row| {
            json!({
                "id": id(row.id),
                "contactId": id(row.contact_id),
                "inboxId": id(row.inbox_id),
                "channel": row.channel,
                "source": row.source,
                "sourceId": row.source_id,
                "sourceUserId": row.source_user_id,
                "sourceUsername": row.source_username,
                "language": row.language,
                "lastIncomingMessageAt": opt_date(row.last_incoming_message_at),
                "contactLastReadAt": opt_date(row.contact_last_read_at),
                "inbox": { "name": row.inbox_name },
            })
        })
        .collect();
    Ok(json(200, json!({ "data": data })))
}

/// `contactAdReferralResource`.
fn ad_referral_json(touch: &ContactAdTouch) -> Value {
    let title = touch
        .metadata
        .as_ref()
        .and_then(|m| m.0.get("adTitle"))
        .and_then(Value::as_str);
    json!({
        "id": id(touch.id),
        "adId": touch.ad_id,
        "title": title,
        "channel": touch.channel,
        "provider": touch.provider,
        "campaignId": touch.campaign_id,
        "adGroupId": touch.ad_group_id,
        "source": touch.source,
        "sourceUrl": touch.source_url,
        "occurredAt": date(touch.occurred_at),
    })
}

/// `Buffer.from(text, "base64")`: base64 or base64url, invalid characters
/// skipped, no padding check.
pub(super) fn lenient_base64(text: &str) -> Option<Vec<u8>> {
    const LENIENT: GeneralPurpose = GeneralPurpose::new(
        &base64::alphabet::STANDARD,
        GeneralPurposeConfig::new()
            .with_decode_allow_trailing_bits(true)
            .with_decode_padding_mode(DecodePaddingMode::RequireNone),
    );
    let mut clean: String = text
        .chars()
        .take_while(|c| *c != '=')
        .filter_map(|c| match c {
            '-' => Some('+'),
            '_' => Some('/'),
            c if c.is_ascii_alphanumeric() || c == '+' || c == '/' => Some(c),
            _ => None,
        })
        .collect();
    if clean.len() % 4 == 1 {
        clean.pop();
    }
    LENIENT.decode(clean).ok()
}

/// `decodeCursor` (`cursorPagination`: `direction`, `createdAt` coerced to a
/// date, `id` digits); the time rides in `createdAt`.
fn decode_cursor(text: &str) -> Option<ContactAdTouchCursor> {
    let bytes = lenient_base64(text)?;
    let value: Value = serde_json::from_slice(&bytes).ok()?;
    let object = value.as_object()?;
    if !matches!(object.get("direction")?.as_str()?, "next" | "prev") {
        return None;
    }
    let occurred_at = match object.get("createdAt")? {
        Value::String(text) => chatbotx_core::js::parse_date(text)?,
        Value::Number(n) => n.as_i64()?,
        _ => return None,
    };
    let id = match object.get("id")? {
        Value::String(text) if !text.is_empty() && text.bytes().all(|b| b.is_ascii_digit()) => {
            text.parse().ok()?
        }
        _ => return None,
    };
    if let Some(shard) = object.get("shardId")
        && !shard.is_string()
    {
        return None;
    }
    Some(ContactAdTouchCursor { occurred_at, id })
}

/// `encodeCursor({direction: "next", createdAt, id})`.
fn encode_cursor(cursor: ContactAdTouchCursor) -> String {
    let text = format!(
        r#"{{"direction":"next","createdAt":"{}","id":"{}"}}"#,
        chatbotx_db::repo::iso(cursor.occurred_at),
        cursor.id
    );
    STANDARD.encode(text)
}

async fn list_ad_referrals(state: &ApiState, call: Call<'_>) -> Reply {
    let mut input = Input::from_query(call.params, call.query);
    let per_page = input.opt_coerce_int("perPage", Num::int(1.0, 100.0));
    let cursor = input.opt_string("cursor", Str::default());
    input.finish()?;
    let contact_id = super::contacts::resolve(state, &call, LISTING_WITH_CURSOR).await?;
    let decoded = match cursor.as_deref().filter(|c| !c.is_empty()) {
        Some(text) => Some(decode_cursor(text).ok_or_else(invalid_cursor)?),
        None => None,
    };
    let page = list_contact_ad_touches(
        &state.db,
        call.workspace_id,
        contact_id,
        per_page.unwrap_or(20),
        decoded,
    )
    .await
    .map_err(|e| business_error(e, LISTING_WITH_CURSOR))?;
    let data: Vec<Value> = page.data.iter().map(ad_referral_json).collect();
    Ok(json(
        200,
        json!({ "data": data, "nextCursor": page.next_cursor.map(encode_cursor) }),
    ))
}

fn invalid_cursor() -> OrpcError {
    let mut error = OrpcError::new(
        "invalidCursor",
        Some(400),
        Some("The cursor is invalid. Start again without a cursor."),
    );
    error.defined = true;
    error
}

async fn refresh_profile(state: &ApiState, call: Call<'_>) -> Reply {
    let mut input = Input::from_body(call.params, call.body);
    let contact_inbox_id = input.id("contactInboxId");
    input.finish()?;
    let contact_id = super::contacts::resolve(state, &call, MUTATING).await?;
    let services = RefreshServices {
        kv: state.kv.as_deref().map(|kv| &kv.mem),
        uploader: None,
    };
    let outcome = refresh_contact_profile(
        &state.db,
        services,
        call.workspace_id,
        contact_id,
        contact_inbox_id.unwrap_or(-1),
    )
    .await
    .map_err(|e| business_error(e, MUTATING))?;
    let body = match outcome {
        RefreshOutcome::Updated(contact) => {
            let public = find_public_contact_or_fail(&state.db, call.workspace_id, contact.id)
                .await
                .map_err(|e| business_error(e, MUTATING))?;
            json!({
                "status": "updated",
                "contact": super::contacts::public_contact_json(state, &public, false),
            })
        }
        RefreshOutcome::Skipped(reason) => json!({
            "status": "skipped",
            "reason": match reason {
                SkipReason::ProfileComplete => "profileComplete",
                SkipReason::CoolingDown => "coolingDown",
                SkipReason::ChannelNotCapable => "channelNotCapable",
            },
        }),
        RefreshOutcome::Unavailable => json!({ "status": "unavailable" }),
        RefreshOutcome::Failed => json!({ "status": "failed" }),
    };
    Ok(json(200, body))
}
