//! `features/contact-sequences/api/public.ts` (P3.6-D2 slice contact
//! sequences): `contacts.listSequences / subscribeSequences /
//! unsubscribeSequences / setSequences`.

use chatbotx_business::contact_sequence::{
    list_by_contact_id, remove_contact_sequences_for_contacts, subscribe_contacts,
    update_contact_sequences,
};
use serde_json::{Value, json};

use super::input::{Input, Seg, key};
use super::{ApiState, Call, FINDING, MUTATING, Reply, business_error, json, no_content};

/// The routes of this module.
pub const ROUTES: &[&str] = &[
    "contacts.listSequences",
    "contacts.subscribeSequences",
    "contacts.unsubscribeSequences",
    "contacts.setSequences",
];

/// Runs one of [`ROUTES`].
pub async fn handle(state: &ApiState, route_id: &str, call: Call<'_>) -> Reply {
    match route_id {
        "contacts.listSequences" => {
            let contact_id = super::contacts::resolve(state, &call, FINDING).await?;
            let rows = list_by_contact_id(&state.db, call.workspace_id, contact_id)
                .await
                .map_err(|e| business_error(e, FINDING))?;
            let data: Vec<Value> = rows
                .iter()
                .map(|row| {
                    json!({
                        "sequenceId": row.sequence_id.to_string(),
                        "sequenceName": row.sequence_name,
                    })
                })
                .collect();
            Ok(json(200, json!({ "data": data })))
        }
        "contacts.subscribeSequences" => {
            let ids = sequence_ids(&call, 1)?;
            let contact_id = super::contacts::resolve(state, &call, MUTATING).await?;
            subscribe_contacts(&state.db, call.workspace_id, &[contact_id], &ids)
                .await
                .map_err(|e| business_error(e, MUTATING))?;
            Ok(no_content())
        }
        "contacts.unsubscribeSequences" => {
            let ids = sequence_ids(&call, 1)?;
            let contact_id = super::contacts::resolve(state, &call, MUTATING).await?;
            remove_contact_sequences_for_contacts(
                &state.db,
                call.workspace_id,
                &[contact_id],
                &ids,
            )
            .await
            .map_err(|e| business_error(e, MUTATING))?;
            Ok(no_content())
        }
        "contacts.setSequences" => {
            let ids = sequence_ids(&call, 0)?;
            let contact_id = super::contacts::resolve(state, &call, MUTATING).await?;
            update_contact_sequences(&state.db, call.workspace_id, contact_id, &ids)
                .await
                .map_err(|e| business_error(e, MUTATING))?;
            Ok(no_content())
        }
        _ => Err(crate::routes::not_ported_error()),
    }
}

/// `sequenceIds`: `z.array(zodBigintAsString()).min(min).max(100)`.
fn sequence_ids(call: &Call<'_>, min: usize) -> Result<Vec<i64>, crate::orpc::OrpcError> {
    let mut input = Input::from_body(call.params, call.body);
    let mut ids = Vec::new();
    match input.get("sequenceIds").cloned() {
        Some(Value::Array(items)) => {
            if items.len() < min {
                input.issue(
                    &key("sequenceIds"),
                    "too_small",
                    "At least one sequence id is required",
                );
            } else if items.len() > 100 {
                input.issue(
                    &key("sequenceIds"),
                    "too_big",
                    "Too big: expected array to have <=100 items",
                );
            }
            for (index, item) in items.iter().enumerate() {
                let path = vec![Seg::Key("sequenceIds".into()), Seg::Index(index)];
                match item.as_str() {
                    Some(text) if !text.is_empty() && text.bytes().all(|b| b.is_ascii_digit()) => {
                        // An id beyond `i64` matches no sequence.
                        ids.push(text.parse::<i64>().unwrap_or(-1));
                    }
                    Some(_) => input.issue(
                        &path,
                        "invalid_format",
                        "Invalid string: must match pattern /^\\d+$/",
                    ),
                    None => input.issue(&path, "invalid_type", "Invalid input: expected string"),
                }
            }
        }
        other => input.issue(
            &key("sequenceIds"),
            "invalid_type",
            &format!(
                "Invalid input: expected array, received {}",
                if other.is_none() {
                    "undefined"
                } else {
                    "other"
                }
            ),
        ),
    }
    input.finish()?;
    Ok(ids)
}
