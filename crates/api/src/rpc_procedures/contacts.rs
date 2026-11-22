//! Contacts: `features/contacts/api/private.ts`, `contact-notes`,
//! `contact-sequences`, `contact-filter` private procedures and the
//! `features/contacts/actions` server actions.

use chatbotx_business::BusinessError;
use chatbotx_business::contact::avatar::resolve_contact_avatar_url;
use chatbotx_business::contact::filter_value_labels::list_contact_activity_value_options;
use chatbotx_business::contact::list::{
    ContactListScope, ListOptions, ListRows, ListScope, Projection, count, list,
};
use chatbotx_business::count_mode::CountMode;
use chatbotx_business::{
    contact, contact_custom_field, contact_note, contact_sequence, custom_field, tag, tag_sync,
};
use chatbotx_core::utils::custom_field_operation::FieldOperationType;
use chatbotx_db::repo::contact::ContactTableRow;
use chatbotx_db::rusqlite::params_from_iter;
use serde_json::{Value, json};

use super::{Ctx, In, Outcome, action_fail, arg, fail, invalid, public, query_of};
use crate::handlers::Member;
use crate::handlers::contact_notes::note_json;
use crate::handlers::contacts::{parse_list_input, tenant_urls};
use crate::handlers::input::Input;
use crate::orpc::OrpcError;

/// The ported actions.
pub const ACTIONS: &[&str] = &[
    "peekImportHeadersAction",
    "bulkTagStatsContactsAction",
    "addContactTagAction",
    "removeContactTagAction",
    "addContactSequenceAction",
    "removeContactSequenceAction",
    "addContactCustomFieldAction",
    "deleteContactCustomFieldAction",
    "deleteContactAction",
    "createContactAction",
    "exportContactsAction",
    "createContactNoteAction",
    "editContactNoteAction",
    "deleteContactNoteAction",
    "blockContactAction",
    "unblockContactAction",
    "updateContactFieldAction",
    "updateContactTagAction",
    "updateContactSequenceAction",
    "resetContactCustomFieldsAction",
    "createCustomFieldAction",
    "refreshContactProfileAction",
];

pub(super) async fn procedure(ctx: &Ctx<'_>, path: &str, input: &Value) -> Outcome {
    Some(match path {
        "contactsAPIs.listContactsByPOSTAuthenticatedAPI" => list_table(ctx, input).await,
        "contactsAPIs.countContactsAuthenticatedAPI" => count_contacts(ctx, input).await,
        "contactsAPIs.getContactAuthenticatedAPI" => get_contact(ctx, input).await,
        "contactsAPIs.countContactInboxesAuthenticatedAPI"
        | "contactsAPIs.listContactInboxesAudiencePreviewAuthenticatedAPI" => {
            audience(ctx, path, input).await
        }
        "contactsAPIs.listExportFilesAuthenticatedAPI" => match require_scope(ctx) {
            Ok(_) => {
                let query = query_of(input, &["perPage", "cursor"]);
                public(ctx, "contacts.listExportFiles", &[], Some(&query), None).await
            }
            Err(error) => Err(error),
        },
        "contactsAPIs.getExportFileAuthenticatedAPI" => {
            match (require_scope(ctx), In(input).id("fileId")) {
                (Ok(_), Ok(file_id)) => {
                    public(
                        ctx,
                        "contacts.getExportFile",
                        &[("fileId", file_id.to_string())],
                        None,
                        None,
                    )
                    .await
                }
                (Err(error), _) | (_, Err(error)) => Err(error),
            }
        }
        "contactsAPIs.listContactAdReferralsAuthenticatedAPI" => {
            contact_route(ctx, input, "contacts.listAdReferrals").await
        }
        "contactNotesAPI.listContactNotesAuthenticatedAPI" => {
            contact_route(ctx, input, "contacts.listNotes").await
        }
        "contactSequencesAPI.listContactSequencesAuthenticatedAPI" => {
            contact_route(ctx, input, "contacts.listSequences").await
        }
        "contactFilterAPI.getActivityAvailabilityAPI" => Ok(json!({
            "enabled": chatbotx_db::repo::keys::keys().contact_activity_filters_enabled,
        })),
        "contactFilterAPI.listActivityValueOptionsAPI" => activity_value_options(ctx, input).await,
        "contactFilterAPI.resolveFilterValueLabelsAPI" => {
            let mut body = input.clone();
            if let Some(object) = body.as_object_mut() {
                object.remove("workspaceId");
            }
            public(
                ctx,
                "contacts.resolveFilterValueLabels",
                &[],
                None,
                Some(&body),
            )
            .await
        }
        _ => return None,
    })
}

/// `requireContactPermissionScope` (the caller is a member here).
fn require_scope(ctx: &Ctx<'_>) -> Result<Member, OrpcError> {
    ctx.contact_scope().ok_or_else(|| {
        fail(BusinessError::exception(
            "User is not authorized to access contacts",
            "systemError",
            400,
        ))
    })
}

/// `countContactInboxes` / `listAudienceInboxesPreview`: the broadcast
/// audience under the member's contact scope (`requireContactPermissionScope`).
async fn audience(ctx: &Ctx<'_>, path: &str, input: &Value) -> Result<Value, OrpcError> {
    let member = require_scope(ctx)?;
    let mut parsed = Input::from_object(input.as_object().cloned().unwrap_or_default());
    let (mut audience, page, per_page, mode) =
        crate::handlers::broadcasts::preview_input(&mut parsed, ctx.workspace_id);
    parsed.finish().map_err(|_| invalid())?;
    audience.can_view_email_and_phone = member.can_view_email_and_phone;
    audience.restrict_to_assigned_user_id = member.restrict_to_assigned_user_id;
    if path.ends_with("listContactInboxesAudiencePreviewAuthenticatedAPI") {
        let data = crate::handlers::broadcasts::audience_preview(ctx.api, audience, page, per_page)
            .await
            .map_err(fail)?;
        return Ok(json!({ "data": data }));
    }
    let counted =
        chatbotx_business::broadcast::audience::count_audience(&ctx.api.db, audience, mode)
            .await
            .map_err(fail)?;
    let mut out = json!({ "total": counted.total, "totalCountCapped": counted.total_count_capped });
    if let Some(cap) = counted.count_cap {
        out["countCap"] = json!(cap);
    }
    Ok(out)
}

fn list_scope(member: Member) -> ListScope {
    ListScope::Member(ContactListScope {
        restrict_to_assigned_user_id: member.restrict_to_assigned_user_id,
        restrict_to_team_ids: None,
        can_view_email_and_phone: member.can_view_email_and_phone,
    })
}

/// The `listContactsRequest` keys (the public list parser: same schema).
fn list_input(
    ctx: &Ctx<'_>,
    input: &Value,
) -> Result<(chatbotx_business::contact::list::ListContactsInput, Input), OrpcError> {
    let mut object = input.as_object().cloned().ok_or_else(invalid)?;
    object.remove("workspaceId");
    let mut parsed = Input::from_object(object);
    let list_input = parse_list_input(&mut parsed, ctx.workspace_id);
    if parsed.failed() {
        return Err(invalid());
    }
    Ok((list_input, parsed))
}

/// `listContacts(input, scope)` with `projection: "table"`.
async fn list_table(ctx: &Ctx<'_>, input: &Value) -> Result<Value, OrpcError> {
    let scope = ctx.require_contact_scope()?;
    let (list_input, _) = list_input(ctx, input)?;
    let options = ListOptions {
        scope: list_scope(scope),
        projection: Projection::Table,
        include: None,
        with_count: true,
    };
    let result = list(&ctx.api.db, &list_input, &options, ctx.now_ms)
        .await
        .map_err(fail)?;
    let rows = match &result.data {
        ListRows::Table(rows) => rows.iter().map(|row| table_row_json(ctx, row)).collect(),
        ListRows::Full(_) => Vec::new(),
    };
    Ok(json!({
        "data": rows,
        "pageCount": result.page_count,
        "totalCount": result.total_count,
        "totalCountCapped": result.total_count_capped,
    }))
}

/// `contactTableRowResource` (avatar resolved as `resolveContactAvatars`).
// ponytail: the table rows carry no inbox ids, so a stored avatar is used as
// is and an inbox-hydrated one is not minted; pass the inbox rows if a list
// shows a missing avatar.
fn table_row_json(ctx: &Ctx<'_>, row: &ContactTableRow) -> Value {
    let avatar = resolve_contact_avatar_url(
        ctx.workspace_id,
        row.avatar.as_deref(),
        &[],
        &tenant_urls(ctx.api),
        ctx.now_ms,
    );
    json!({
        "id": row.id.to_string(),
        "fullName": row.full_name,
        "email": row.email,
        "phoneNumber": row.phone_number,
        "avatar": avatar,
        "createdAt": chatbotx_db::repo::iso_millis(row.created_at),
        "contactInboxes": row.contact_inboxes.iter().map(|inbox| json!({
            "channel": inbox.channel,
            "source": inbox.source,
            "contactLastReadAt": inbox.contact_last_read_at.map(chatbotx_db::repo::iso_millis),
        })).collect::<Vec<_>>(),
        "conversation": row.conversation.as_ref().map(|conversation| json!({
            "id": conversation.id.to_string(),
            "assignedUser": conversation.assigned_user.as_ref().map(|(name, email)| json!({
                "name": name,
                "email": email,
            })),
        })),
    })
}

/// `countContacts(input, scope)`.
async fn count_contacts(ctx: &Ctx<'_>, input: &Value) -> Result<Value, OrpcError> {
    let scope = ctx.require_contact_scope()?;
    let (list_input, _) = list_input(ctx, input)?;
    let mode = match In(input).opt_str("countMode")?.as_deref() {
        None => None,
        Some(text) => Some(CountMode::parse(text).ok_or_else(invalid)?),
    };
    let outcome = count(
        &ctx.api.db,
        &list_input,
        &list_scope(scope),
        mode,
        ctx.now_ms,
    )
    .await
    .map_err(fail)?;
    let mut body = json!({
        "total": outcome.total,
        "totalCountCapped": outcome.total_count_capped,
    });
    if let Some(cap) = outcome.count_cap {
        body["countCap"] = json!(cap);
    }
    Ok(body)
}

/// The contact ids of `ids` the member may touch (`accessScope`): all for an
/// unrestricted member, else the ones whose conversation is assigned to them.
pub(super) async fn scoped_ids(
    ctx: &Ctx<'_>,
    member: Member,
    ids: Vec<i64>,
) -> Result<Vec<i64>, OrpcError> {
    let Some(user_id) = member.restrict_to_assigned_user_id else {
        return Ok(ids);
    };
    if ids.is_empty() {
        return Ok(ids);
    }
    let workspace_id = ctx.workspace_id;
    let allowed: Vec<i64> = ctx
        .api
        .db
        .read(move |conn| {
            let marks = vec!["?"; ids.len()].join(", ");
            let sql = format!(
                r#"SELECT c."id" FROM "Contact" c
                   JOIN "Conversation" v ON v."contactId" = c."id"
                   WHERE c."workspaceId" = ? AND v."assignedUserId" = ? AND c."id" IN ({marks})"#
            );
            let mut binds = vec![workspace_id, user_id];
            binds.extend(ids.iter().copied());
            let mut stmt = conn.prepare(&sql)?;
            let rows = stmt
                .query_map(params_from_iter(binds.iter()), |row| row.get::<_, i64>(0))?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        })
        .await
        .map_err(|e| crate::handlers::internal_error(&e.to_string()))?;
    Ok(allowed)
}

/// A per-contact public route (`/v1/contacts/{identifier}/...`) after the
/// `requireContactPermissionScopeForMember` + assigned-contact check.
async fn contact_route(ctx: &Ctx<'_>, input: &Value, route_id: &str) -> Result<Value, OrpcError> {
    let scope = ctx.require_contact_scope()?;
    let contact_id = In(input).id("contactId")?;
    contact_call(ctx, scope, contact_id, route_id, None).await
}

/// `contactService.findByIdOrFail({ accessScope })` (404 "Contact not
/// found" outside the member's scope), then the public route of the contact.
async fn contact_call(
    ctx: &Ctx<'_>,
    scope: Member,
    contact_id: i64,
    route_id: &str,
    body: Option<&Value>,
) -> Result<Value, OrpcError> {
    accessible(ctx, scope, contact_id).await?;
    let params = [("identifier", format!("id:{contact_id}"))];
    public(ctx, route_id, &params, None, body).await
}

/// `requireContactAccessForMember`: 404 "Contact not found" without
/// contacts access or outside the member's assigned contacts.
pub(super) async fn require_contact_access(
    ctx: &Ctx<'_>,
    contact_id: i64,
) -> Result<(), OrpcError> {
    let scope = ctx.require_contact_scope()?;
    accessible(ctx, scope, contact_id).await
}

/// 404 "Contact not found" when the member may not touch the contact.
async fn accessible(ctx: &Ctx<'_>, scope: Member, contact_id: i64) -> Result<(), OrpcError> {
    if scoped_ids(ctx, scope, vec![contact_id]).await?.is_empty() {
        return Err(fail(BusinessError::not_found("Contact not found")));
    }
    Ok(())
}

/// `getContact`: the contact columns (PII masked without `emailAndPhone`),
/// `tags`, `customFields` (definition + `value`).
async fn get_contact(ctx: &Ctx<'_>, input: &Value) -> Result<Value, OrpcError> {
    let scope = ctx.require_contact_scope()?;
    let contact_id = In(input).id("contactId")?;
    let workspace_id = ctx.workspace_id;
    let detail = find_detail(ctx, workspace_id, contact_id)
        .await?
        .ok_or_else(|| fail(BusinessError::not_found("Contact not found")))?;
    if let Some(user_id) = scope.restrict_to_assigned_user_id
        && detail
            .conversation
            .as_ref()
            .and_then(|c| c.assigned_user_id)
            != Some(user_id)
    {
        return Err(fail(BusinessError::not_found("Contact not found")));
    }
    let mut out = detail.contact.to_json();
    out["avatar"] = json!(resolve_contact_avatar_url(
        workspace_id,
        detail.contact.avatar.as_deref(),
        &[],
        &tenant_urls(ctx.api),
        ctx.now_ms,
    ));
    if !scope.can_view_email_and_phone {
        out["email"] = Value::Null;
        out["phoneNumber"] = Value::Null;
    }
    out["tags"] = detail.tags.iter().map(|t| t.to_json()).collect();
    out["customFields"] = detail
        .contact_custom_fields
        .iter()
        .map(|field| {
            let mut value = field.custom_field.to_json();
            value["value"] = json!(field.value.value);
            value
        })
        .collect();
    Ok(out)
}

/// `contactRepository.findDetailById`.
async fn find_detail(
    ctx: &Ctx<'_>,
    workspace_id: i64,
    contact_id: i64,
) -> Result<Option<chatbotx_db::repo::contact::ContactDetail>, OrpcError> {
    chatbotx_business::tx::read(&ctx.api.db, move |conn| {
        Ok(chatbotx_db::repo::contact::find_detail_by_id(
            conn,
            workspace_id,
            contact_id,
        )?)
    })
    .await
    .map_err(fail)
}

/// `listContactActivityValueOptions`.
async fn activity_value_options(ctx: &Ctx<'_>, input: &Value) -> Result<Value, OrpcError> {
    let input = In(input);
    let source = input.opt_str("source")?.ok_or_else(invalid)?;
    if !["keywords", "whatsappFlows", "emailTopics", "ads"].contains(&source.as_str()) {
        return Err(invalid());
    }
    let search = input.opt_str("search")?;
    if search.as_ref().is_some_and(|s| s.chars().count() > 100) {
        return Err(invalid());
    }
    let options = list_contact_activity_value_options(
        &ctx.api.db,
        ctx.workspace_id,
        &source,
        search.as_deref(),
    )
    .await
    .map_err(fail)?;
    Ok(json!({
        "data": options.iter().map(|o| json!({ "value": o.value, "label": o.label })).collect::<Vec<_>>(),
    }))
}

// ---- actions --------------------------------------------------------------

pub(super) async fn action(ctx: &Ctx<'_>, name: &str, args: &[Value]) -> Outcome {
    if !ACTIONS.contains(&name) {
        return None;
    }
    let input = In(arg(args, 1));
    let db = &ctx.api.db;
    Some(
        async {
            match name {
                // Any member (`workspaceActionClient`), no import-type check
                // (the session UI leaves it out).
                // ponytail: the error copy is the business English message,
                // not the builder's `fields.import.*` translation.
                "peekImportHeadersAction" => {
                    let file_id = input.id("fileId")?;
                    let files = ctx.api.files.as_deref().ok_or_else(|| {
                        OrpcError::new(
                            "INTERNAL_SERVER_ERROR",
                            Some(500),
                            Some("file storage is not configured"),
                        )
                    })?;
                    return chatbotx_business::import::peek_import_headers(
                        db,
                        files,
                        ctx.workspace_id,
                        file_id,
                        None,
                    )
                    .await
                    .map(|headers| json!(headers))
                    .map_err(action_fail);
                }
                // [workspaceId], bulkTagStatsContactsRequest: the signed-in
                // user requests it, under their contact scope. Runs in the
                // request (no default-queue worker, as `contacts.bulkTagByStats`).
                "bulkTagStatsContactsAction" => {
                    let member = require_scope(ctx)?;
                    let body = arg(args, 1).to_string();
                    let call = crate::handlers::Call {
                        workspace_id: ctx.workspace_id,
                        params: &[],
                        query: None,
                        body: body.as_bytes(),
                    };
                    let request = crate::handlers::contacts_bulk::by_stats_request(&call)
                        .map_err(super::schema_errors)?;
                    chatbotx_business::contact::bulk_tag_stats::assert_source_owned(db, ctx.workspace_id, &request.source)
                        .await
                        .map_err(action_fail)?;
                    let job = chatbotx_business::contact::bulk_tag_stats::prepare(
                        db,
                        ctx.workspace_id,
                        ctx.user_id,
                        &request,
                        member.restrict_to_assigned_user_id,
                    )
                    .await
                    .map_err(action_fail)?;
                    if let Some(job) = job {
                        let linked = chatbotx_business::contact::bulk_tag_stats::run(db, &job).await.map_err(action_fail)?;
                        let workspace = ctx.workspace_id.to_string();
                        let events: Vec<_> = linked
                            .iter()
                            .map(|(contact, tag)| {
                                chatbotx_business::events::trigger::tag_applied(
                                    &workspace,
                                    &contact.to_string(),
                                    &tag.to_string(),
                                    None,
                                )
                            })
                            .collect();
                        ctx.api.emit_triggers(events).await;
                        if let Some(jobs) = ctx.api.jobs.as_deref() {
                            tag_sync::enqueue_attach_many(jobs, ctx.workspace_id, &linked).await;
                        }
                    }
                    return Ok(Value::Null);
                }
                "createContactAction" => {
                    let created =
                        public(ctx, "contacts.create", &[], None, Some(arg(args, 1))).await?;
                    let id = created.get("id").and_then(Value::as_str).unwrap_or_default();
                    let contact_id = id.parse::<i64>().map_err(|_| invalid())?;
                    let workspace_id = ctx.workspace_id;
                    let contact = find_detail(ctx, workspace_id, contact_id).await?;
                    return Ok(contact.map_or(Value::Null, |c| c.contact.to_json()));
                }
                "exportContactsAction" => {
                    require_scope(ctx)?;
                    let started =
                        public(ctx, "contacts.export", &[], None, Some(arg(args, 1))).await?;
                    return Ok(started);
                }
                "createCustomFieldAction" => return create_custom_field(ctx, arg(args, 1)).await,
                _ => {}
            }
            if let Some(outcome) = detail_action(ctx, name, args).await {
                return outcome;
            }
            let scope = require_scope(ctx)?;
            let ids = scoped_ids(ctx, scope, input.ids("ids")?).await?;
            let ws = ctx.workspace_id;
            match name {
                "addContactTagAction" | "removeContactTagAction" => {
                    let names = input.strings("tags")?;
                    let add = name == "addContactTagAction";
                    let (outcome, pairs) = if add {
                        tag::attach_by_names_to_contacts_with_pairs(db, ws, &ids, &names).await
                    } else {
                        tag::detach_by_names_from_contacts_with_pairs(db, ws, &ids, &names).await
                    }
                    .map_err(action_fail)?;
                    if let Some(jobs) = ctx.api.jobs.as_deref() {
                        if add {
                            tag_sync::enqueue_attach_many(jobs, ws, &pairs).await;
                        } else {
                            tag_sync::enqueue_detach_many(jobs, ws, &pairs).await;
                        }
                    }
                    Ok(json!({
                        "processedContactIds": outcome.processed_contact_ids.iter().map(i64::to_string).collect::<Vec<_>>(),
                        "skippedContactIds": outcome.skipped_contact_ids.iter().map(i64::to_string).collect::<Vec<_>>(),
                    }))
                }
                "addContactSequenceAction" | "removeContactSequenceAction" => {
                    let sequences = input.ids("sequences")?;
                    if name == "addContactSequenceAction" {
                        contact_sequence::subscribe_contacts(db, ws, &ids, &sequences).await
                    } else {
                        contact_sequence::unsubscribe_contacts(db, ws, &ids, &sequences).await
                    }
                    .map_err(action_fail)?;
                    Ok(Value::Null)
                }
                "addContactCustomFieldAction" => {
                    let custom_field_id = input.id("customFieldId")?;
                    let operation = input
                        .opt_str("operation")?
                        .and_then(|text| FieldOperationType::parse(&text))
                        .ok_or_else(invalid)?;
                    let value = input.opt_str("value")?.ok_or_else(invalid)?;
                    let timezone = input.opt_str("clientTimezone")?;
                    contact_custom_field::apply_operation_to_contacts(
                        db,
                        ws,
                        &ids,
                        custom_field_id,
                        operation,
                        value.trim(),
                        timezone.as_deref().map(str::trim),
                    )
                    .await
                    .map_err(action_fail)?;
                    Ok(Value::Null)
                }
                "deleteContactCustomFieldAction" => {
                    let custom_field_id = input.id("customFieldId")?;
                    contact_custom_field::delete_by_custom_field_id(db, ws, &ids, custom_field_id)
                        .await
                        .map_err(action_fail)?;
                    Ok(Value::Null)
                }
                _ => {
                    let outcome = contact::delete_and_record(db, ws, &ids)
                        .await
                        .map_err(action_fail)?;
                    Ok(json!({
                        "processedContactIds": outcome.processed_contact_ids.iter().map(i64::to_string).collect::<Vec<_>>(),
                        "skippedContactIds": outcome.skipped_contact_ids.iter().map(i64::to_string).collect::<Vec<_>>(),
                    }))
                }
            }
        }
        .await,
    )
}

/// The contact detail actions (`[workspaceId, contactId?]` bind args).
async fn detail_action(ctx: &Ctx<'_>, name: &str, args: &[Value]) -> Outcome {
    const DETAIL: &[&str] = &[
        "createContactNoteAction",
        "editContactNoteAction",
        "deleteContactNoteAction",
        "blockContactAction",
        "unblockContactAction",
        "updateContactFieldAction",
        "updateContactTagAction",
        "updateContactSequenceAction",
        "resetContactCustomFieldsAction",
        "refreshContactProfileAction",
    ];
    if !DETAIL.contains(&name) {
        return None;
    }
    let db = &ctx.api.db;
    let ws = ctx.workspace_id;
    let bound_id = || In(&json!({ "id": arg(args, 1) })).id("id");
    Some(
        async {
            let scope = require_scope(ctx)?;
            match name {
                "createContactNoteAction" | "editContactNoteAction" | "deleteContactNoteAction" => {
                    let contact_id = bound_id()?;
                    let input = In(arg(args, 2));
                    let text = match name {
                        "createContactNoteAction" => Some(input.str("text", true, 1, 1000)?),
                        "editContactNoteAction" if input.get("text").is_some() => {
                            Some(input.str("text", true, 1, 1000)?)
                        }
                        _ => None,
                    };
                    let note_id = match name {
                        "createContactNoteAction" => None,
                        _ => Some(input.id("contactNoteId")?),
                    };
                    accessible(ctx, scope, contact_id).await?;
                    let note = match (name, note_id) {
                        ("createContactNoteAction", _) => {
                            contact_note::create(
                                db,
                                ws,
                                contact_id,
                                &text.unwrap_or_default(),
                                Some(ctx.user_id),
                            )
                            .await
                        }
                        ("editContactNoteAction", Some(note)) => {
                            contact_note::update(db, ws, contact_id, note, text.as_deref()).await
                        }
                        (_, note) => {
                            // A deleted note is a no-op for the optimistic list.
                            return match contact_note::delete(
                                db,
                                ws,
                                contact_id,
                                note.unwrap_or(-1),
                            )
                            .await
                            {
                                Err(BusinessError::Exception { code, .. })
                                    if code == "notFound" =>
                                {
                                    Ok(Value::Null)
                                }
                                other => other.map(|()| Value::Null).map_err(action_fail),
                            };
                        }
                    };
                    note.map(|row| note_json(&row)).map_err(action_fail)
                }
                // `refreshContactProfileAction` (`{ contactInboxId }`): the
                // public `contacts.refreshProfile` under the contact scope.
                "refreshContactProfileAction" => {
                    let contact_inbox_id = In(arg(args, 2)).id("contactInboxId")?;
                    let body = json!({ "contactInboxId": contact_inbox_id.to_string() });
                    contact_call(
                        ctx,
                        scope,
                        bound_id()?,
                        "contacts.refreshProfile",
                        Some(&body),
                    )
                    .await
                }
                "blockContactAction" | "unblockContactAction" => {
                    let route = if name == "blockContactAction" {
                        "contacts.block"
                    } else {
                        "contacts.unblock"
                    };
                    contact_call(ctx, scope, bound_id()?, route, None).await
                }
                "updateContactFieldAction" => {
                    let body = arg(args, 2);
                    let valid = body
                        .as_object()
                        .is_some_and(|fields| fields.values().all(Value::is_string));
                    if !valid {
                        return Err(invalid());
                    }
                    contact_call(ctx, scope, bound_id()?, "contacts.updateLegacy", Some(body)).await
                }
                "updateContactTagAction" => {
                    let input = In(arg(args, 1));
                    let contact_id = input.id("contactId")?;
                    let tags: Vec<String> = input
                        .strings("tags")?
                        .iter()
                        .map(|t| t.trim().to_string())
                        .collect();
                    accessible(ctx, scope, contact_id).await?;
                    let (rows, added, removed) =
                        tag::replace_contact_tags_by_names_with_changes(db, ws, contact_id, &tags)
                            .await
                            .map_err(action_fail)?;
                    if let Some(jobs) = ctx.api.jobs.as_deref() {
                        let pairs =
                            |t: &[i64]| t.iter().map(|t| (contact_id, *t)).collect::<Vec<_>>();
                        tag_sync::enqueue_attach_many(jobs, ws, &pairs(&added)).await;
                        tag_sync::enqueue_detach_many(jobs, ws, &pairs(&removed)).await;
                    }
                    Ok(rows.iter().map(|t| t.to_json()).collect())
                }
                "updateContactSequenceAction" => {
                    let input = In(arg(args, 1));
                    let contact_id = input.id("contactId")?;
                    let sequences = input.ids("sequences")?;
                    accessible(ctx, scope, contact_id).await?;
                    let rows =
                        contact_sequence::update_contact_sequences(db, ws, contact_id, &sequences)
                            .await
                            .map_err(action_fail)?;
                    Ok(rows
                        .iter()
                        .map(|row| {
                            json!({ "sequence": {
                                "id": row.sequence_id.to_string(),
                                "name": row.sequence_name,
                            } })
                        })
                        .collect())
                }
                _ => {
                    // `resetContactCustomFieldsAction`.
                    let contact_id = In(arg(args, 1)).id("contactId")?;
                    accessible(ctx, scope, contact_id).await?;
                    contact_custom_field::clear_by_contact_id(db, ws, contact_id)
                        .await
                        .map_err(action_fail)?;
                    Ok(Value::Null)
                }
            }
        }
        .await,
    )
}

/// `createCustomFieldAction`: `customFieldService.create` (a taken name is
/// the `name` validation error).
async fn create_custom_field(ctx: &Ctx<'_>, input: &Value) -> Result<Value, OrpcError> {
    use crate::handlers::custom_fields::{CUSTOM_FIELD_TYPES, field_name, nullish_digits};
    let object = input.as_object().cloned().ok_or_else(invalid)?;
    let mut input = Input::from_object(object);
    let name = field_name(&mut input);
    let kind = input.enum_at(
        &crate::handlers::input::key("type"),
        input.get("type").cloned().as_ref(),
        CUSTOM_FIELD_TYPES,
        true,
    );
    let folder_id = nullish_digits(&mut input, "folderId").flatten();
    let description = input
        .nullish_string("description", crate::handlers::input::Str::max(usize::MAX))
        .flatten();
    if input.failed() {
        return Err(invalid());
    }
    custom_field::create(
        &ctx.api.db,
        ctx.workspace_id,
        custom_field::CreateCustomField {
            name: name.unwrap_or_default(),
            r#type: kind.unwrap_or_default(),
            description,
            folder_id,
        },
    )
    .await
    .map(|_| Value::Null)
    .map_err(action_fail)
}
