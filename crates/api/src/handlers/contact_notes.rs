//! `apps/builder/src/features/contact-notes/api/public.ts`
//! (`contactsNotesPublicRouter`).

use chatbotx_business::contact_note as service;
use chatbotx_db::repo::models::ContactNote;
use serde_json::{Value, json};

use super::input::{Input, Str, key};
use super::{
    ApiState, Call, FINDING, MUTATING, Reply, business_error, date, id, json, no_content, opt_id,
};

/// The ported routes.
pub const ROUTES: &[&str] = &[
    "contacts.listNotes",
    "contacts.createNote",
    "contacts.updateNote",
    "contacts.deleteNote",
];

/// `contactNoteResource`.
pub fn note_json(note: &ContactNote) -> Value {
    json!({
        "id": id(note.id),
        "createdAt": date(note.created_at),
        "updatedAt": date(note.updated_at),
        "text": note.text,
        "contactId": id(note.contact_id),
        "createdById": opt_id(note.created_by_id),
    })
}

/// `zodBigintAsString()` (`noteId`); beyond `i64` = `-1` (no row).
fn note_id(input: &mut Input) -> Option<i64> {
    let value = input.get("noteId").cloned();
    let text = input.string_at(&key("noteId"), value.as_ref(), Str::default(), true)?;
    if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) {
        input.issue(
            &key("noteId"),
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
        "contacts.listNotes" => {
            let contact_id = super::contacts::resolve(state, &call, FINDING).await?;
            let notes = service::list_by_contact_id(db, ws, contact_id)
                .await
                .map_err(|e| business_error(e, FINDING))?;
            let data: Vec<Value> = notes.iter().map(note_json).collect();
            Ok(json(200, json!({ "data": data })))
        }
        "contacts.createNote" | "contacts.updateNote" => {
            let mut input = Input::from_body(call.params, call.body);
            let text = input.string("text", Str::trimmed(1, 1000));
            let note = if route_id == "contacts.updateNote" {
                note_id(&mut input)
            } else {
                None
            };
            input.finish()?;
            let contact_id = super::contacts::resolve(state, &call, MUTATING).await?;
            let text = text.unwrap_or_default();
            let result = match note {
                Some(note) => service::update(db, ws, contact_id, note, Some(&text)).await,
                None => service::create(db, ws, contact_id, &text, None).await,
            };
            result
                .map(|row| json(200, note_json(&row)))
                .map_err(|e| business_error(e, MUTATING))
        }
        "contacts.deleteNote" => {
            let mut input = Input::from_query(call.params, call.query);
            let note = note_id(&mut input);
            input.finish()?;
            let contact_id = super::contacts::resolve(state, &call, MUTATING).await?;
            service::delete(db, ws, contact_id, note.unwrap_or_default())
                .await
                .map(|()| no_content())
                .map_err(|e| business_error(e, MUTATING))
        }
        _ => Err(crate::routes::not_ported_error()),
    }
}
