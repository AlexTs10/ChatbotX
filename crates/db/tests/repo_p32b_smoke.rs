//! Smoke tests of the P3.2b repository functions that have no TS test (not locked).

mod common;

use chatbotx_db::repo::contact::{self as contacts, ContactListInput};
use chatbotx_db::repo::sql::Sql;
use chatbotx_db::repo::utils::SortDir;
use chatbotx_db::repo::{self};
use chatbotx_db::rusqlite::params;
use common::{Seed, fresh_db, seed};

async fn contact_db() -> (common::TestDb, Seed) {
    let db = fresh_db().await;
    let s = seed(&db).await;
    db.write(move |tx| {
        tx.execute(
            r#"INSERT INTO "Tag" ("id", "name", "workspaceId") VALUES (60, 'vip', ?1)"#,
            params![s.workspace_id],
        )?;
        tx.execute(
            r#"INSERT INTO "ContactToTag" ("contactId", "tagId") VALUES (?1, 60)"#,
            params![s.contact_id],
        )?;
        tx.execute(
            r#"INSERT INTO "CustomField" ("id", "name", "type", "workspaceId") VALUES (70, 'cf', 'shortText', ?1)"#,
            params![s.workspace_id],
        )?;
        tx.execute(
            r#"INSERT INTO "ContactCustomField" ("id", "value", "contactId", "customFieldId") VALUES (71, 'x', ?1, 70)"#,
            params![s.contact_id],
        )?;
        common::insert_contact_inbox(tx, s, 80, "psid-1", None, None)?;
        common::insert_conversation(tx, s, 90, None)?;
        tx.execute(
            r#"UPDATE "Conversation" SET "assignedUserId" = ?1 WHERE "id" = 90"#,
            params![s.user_id],
        )?;
        tx.execute(
            r#"INSERT INTO "Contact" ("id", "workspaceId", "email") VALUES (4002, ?1, 'b@example.test')"#,
            params![s.workspace_id],
        )?;
        tx.execute(
            r#"UPDATE "InboxContactStat" SET "totalContacts" = 7 WHERE "inboxId" = ?1"#,
            params![s.inbox_id],
        )?;
        Ok(())
    })
    .await
    .expect("contact fixture");
    (db, s)
}

fn ws_filter(s: Seed) -> Sql {
    Sql::bind("\"Contact\".\"workspaceId\" = ", s.workspace_id)
}

#[tokio::test]
async fn contact_relational_loaders_read_the_relations() {
    let (db, s) = contact_db().await;
    let public = repo::read(&db, move |c| {
        contacts::find_public_by_id(c, s.workspace_id, s.contact_id)
    })
    .await
    .expect("find")
    .expect("a contact");
    assert_eq!(
        public.tags.iter().map(|t| t.id).collect::<Vec<_>>(),
        vec![60]
    );
    assert_eq!(public.contact_custom_fields.len(), 1);
    assert_eq!(public.contact_inboxes.len(), 1);
    assert_eq!(public.contact_inboxes[0].inbox.id, s.inbox_id);
    let conversation = public.conversation.expect("conversation");
    assert_eq!(conversation.conversation.id, 90);
    assert_eq!(conversation.assigned_user.map(|u| u.id), Some(s.user_id));

    let detail = repo::read(&db, move |c| {
        contacts::find_detail_by_id(c, s.workspace_id, s.contact_id)
    })
    .await
    .expect("detail")
    .expect("a contact");
    assert_eq!(detail.contact_custom_fields[0].custom_field.id, 70);
    assert_eq!(detail.conversation.map(|c| c.id), Some(90));
    let other_ws = repo::read(&db, move |c| {
        contacts::find_detail_by_id(c, s.workspace_id + 1, s.contact_id)
    })
    .await
    .expect("detail");
    assert!(other_ws.is_none());
}

#[tokio::test]
async fn contact_list_loaders_page_order_and_count() {
    let (db, s) = contact_db().await;
    let input = ContactListInput {
        filter: ws_filter(s),
        limit: 1,
        offset: 0,
        order_by: vec![("id".to_owned(), SortDir::Desc)],
    };
    let page = {
        let input = input.clone();
        repo::read(&db, move |c| contacts::list_with_relations(c, &input))
            .await
            .expect("list")
    };
    assert_eq!(
        page.iter().map(|p| p.contact.id).collect::<Vec<_>>(),
        vec![4002]
    );
    let rows = {
        let input = ContactListInput {
            offset: 1,
            ..input.clone()
        };
        repo::read(&db, move |c| contacts::list_table_rows(c, &input))
            .await
            .expect("rows")
    };
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].id, s.contact_id);
    assert_eq!(rows[0].contact_inboxes.len(), 1);
    assert_eq!(
        rows[0]
            .conversation
            .as_ref()
            .and_then(|c| c.assigned_user.clone())
            .map(|u| u.1),
        Some("owner@example.com".to_owned())
    );
    let with_inboxes = {
        let input = ContactListInput {
            limit: 10,
            ..input.clone()
        };
        repo::read(&db, move |c| {
            contacts::list_with_inboxes_and_conversation(c, &input)
        })
        .await
        .expect("list")
    };
    assert_eq!(with_inboxes.len(), 2);
    let by_cf = repo::read(&db, move |c| {
        contacts::list_public_by_custom_field(
            c,
            &ws_filter(s),
            5,
            &[("id".to_owned(), SortDir::Asc)],
        )
    })
    .await
    .expect("by custom field");
    assert_eq!(by_cf[0].contact.id, s.contact_id);
    let n = repo::read(&db, move |c| contacts::count(c, &ws_filter(s)))
        .await
        .expect("count");
    assert_eq!(n, 2);
    let capped = repo::read(&db, move |c| contacts::count_capped(c, &ws_filter(s), 1))
        .await
        .expect("capped");
    assert_eq!((capped.total, capped.capped), (1, true));
    let total = repo::read(&db, move |c| {
        contacts::sum_total_contacts_from_inbox_stats(c, s.workspace_id)
    })
    .await
    .expect("sum");
    assert_eq!(total, 7);
}

#[tokio::test]
async fn contact_identity_lookup_and_enrich_if_null() {
    let (db, s) = contact_db().await;
    let found = repo::read(&db, move |c| {
        contacts::find_id_by_identity_where(c, s.workspace_id, None, Some("b@example.test"), None)
    })
    .await
    .expect("find");
    assert_eq!(found, Some(4002));
    repo::write(&db, move |tx| {
        contacts::enrich_if_null(tx, 4002, Some("+100"), Some("other@example.test"))
    })
    .await
    .expect("enrich");
    let row = common::query_text(
        &db,
        r#"SELECT "phoneNumber" || '|' || "email" FROM "Contact" WHERE "id" = 4002"#,
        vec![],
    )
    .await;
    assert_eq!(row.as_deref(), Some("+100|b@example.test"));
}

#[tokio::test]
async fn contact_custom_field_definition_readers() {
    use chatbotx_db::repo::contact_custom_field as ccf;
    let (db, s) = contact_db().await;
    let selected = repo::read(&db, move |c| {
        ccf::list_selected_definition_values_by_contact(c, s.workspace_id, s.contact_id, &[70, 99])
    })
    .await
    .expect("selected");
    assert_eq!(selected.len(), 1);
    assert_eq!(
        (selected[0].name.as_str(), selected[0].value.as_str()),
        ("cf", "x")
    );
    let by_name = repo::read(&db, move |c| {
        ccf::list_values_by_field_keys(c, s.workspace_id, s.contact_id, &[], &["cf".to_owned()])
    })
    .await
    .expect("by keys");
    assert_eq!(
        by_name
            .iter()
            .map(|v| v.custom_field_id)
            .collect::<Vec<_>>(),
        vec![70]
    );
    let none = repo::read(&db, move |c| {
        ccf::list_values_by_field_keys(c, s.workspace_id + 1, s.contact_id, &[70], &[])
    })
    .await
    .expect("other workspace");
    assert!(none.is_empty());
    let all = repo::read(&db, move |c| {
        ccf::list_with_definition_by_contact(c, s.workspace_id, s.contact_id)
    })
    .await
    .expect("list");
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].custom_field.r#type, "shortText");
    let one = repo::read(&db, move |c| {
        ccf::find_with_definition(c, s.workspace_id, s.contact_id, 70)
    })
    .await
    .expect("find");
    assert_eq!(one.map(|v| v.value.id), Some(71));
}

#[tokio::test]
async fn inbox_readers_and_connection_mirror() {
    use chatbotx_db::repo::inbox;
    use chatbotx_db::rusqlite::types::Value;
    let (db, s) = contact_db().await;
    db.write(move |tx| {
        tx.execute(
            r#"INSERT INTO "Inbox" ("id", "name", "channel", "sourceId", "workspaceId", "status", "disconnectReason") VALUES (3002, 'A smtp', 'smtp', 'mail-1', ?1, 'disconnected', 'tenant_suspended')"#,
            params![s.workspace_id],
        )?;
        Ok(())
    })
    .await
    .expect("inbox fixture");
    let ws = s.workspace_id;
    let ids = repo::read(&db, move |c| {
        inbox::list_identities_by_ids(c, ws, &[s.inbox_id, 3002, 9])
    })
    .await
    .expect("identities");
    assert_eq!(
        ids.iter().map(|i| i.id).collect::<Vec<_>>(),
        vec![s.inbox_id, 3002]
    );
    let status = ids[0].status.clone();
    let connected = repo::read(&db, move |c| {
        inbox::list_connected_identities_by_channels(c, &[ws], &["messenger", "smtp"])
    })
    .await
    .expect("connected");
    assert_eq!(connected.len(), usize::from(status == "connected"));
    let found = repo::read(&db, move |c| inbox::find_by_id(c, 3002))
        .await
        .expect("find");
    assert_eq!(found.map(|i| i.channel), Some("smtp".to_owned()));
    let locked = repo::read(&db, move |c| inbox::find_by_id_for_update(c, ws + 1, 3002))
        .await
        .expect("for update");
    assert!(locked.is_none());
    let options = repo::read(&db, move |c| {
        inbox::list_options_by_workspace_and_channel(c, ws, "smtp")
    })
    .await
    .expect("options");
    assert_eq!(options, vec![(3002, "A smtp".to_owned())]);
    let analytics = repo::read(&db, move |c| {
        inbox::list_analytics_options_by_workspace(c, ws)
    })
    .await
    .expect("analytics");
    assert!(analytics.iter().all(|o| o.channel != "smtp"));
    let suspended = repo::read(&db, move |c| {
        inbox::list_tenant_suspended_without_connection_by_owner(c, s.user_id)
    })
    .await
    .expect("suspended");
    assert_eq!(
        suspended.iter().map(|i| i.id).collect::<Vec<_>>(),
        vec![3002]
    );
    repo::write(&db, move |tx| {
        inbox::update_connection_mirror(
            tx,
            ws,
            3002,
            &vec![
                ("status", Value::Text("connected".into())),
                ("disconnectReason", Value::Null),
            ],
        )
    })
    .await
    .expect("mirror");
    let refused = repo::write(&db, move |tx| {
        inbox::update_connection_mirror(tx, ws, 3002, &vec![("name", Value::Text("x".into()))])
    })
    .await;
    assert!(refused.is_err());
    let after = repo::read(&db, move |c| inbox::find_by_id(c, 3002))
        .await
        .expect("find")
        .expect("inbox");
    assert_eq!(
        (after.status.as_str(), after.disconnect_reason),
        ("connected", None)
    );
}

#[tokio::test]
async fn appointment_calendar_crud() {
    use chatbotx_db::repo::appointment_calendar::{
        self as cal, AvailabilityInput, CalendarListInput, CreateCalendarInput,
    };
    use chatbotx_db::rusqlite::types::Value;
    let (db, s) = contact_db().await;
    let ws = s.workspace_id;
    let mk = |name: &str, slug: &str| CreateCalendarInput {
        workspace_id: ws,
        name: name.to_owned(),
        timezone: "UTC".to_owned(),
        location_type: "onlineMeeting".to_owned(),
        public_link_slug: slug.to_owned(),
        active: Some(false),
    };
    let (a, b) = (mk("Consult", "consult"), mk("Consult 2", "consult-2"));
    let (a, b) = repo::write(&db, move |tx| {
        Ok((cal::create(tx, &a)?, cal::create(tx, &b)?))
    })
    .await
    .expect("create");
    assert!(!a.active);
    let names = repo::read(&db, move |c| cal::list_names_with_prefix(c, ws, "Consult"))
        .await
        .expect("names");
    assert_eq!(names.len(), 2);
    let page = repo::read(&db, move |c| {
        cal::list(
            c,
            &CalendarListInput {
                workspace_id: ws,
                search: Some("2".to_owned()),
                ..CalendarListInput::default()
            },
        )
    })
    .await
    .expect("list");
    assert_eq!((page.total, page.page_count, page.data[0].id), (1, 1, b.id));
    let (a_id, b_id) = (a.id, b.id);
    let updated = repo::write(&db, move |tx| {
        cal::update(
            tx,
            ws,
            a_id,
            &vec![
                ("name", Value::Text("Renamed".into())),
                ("active", Value::Integer(1)),
            ],
        )
    })
    .await
    .expect("update")
    .expect("a row");
    assert_eq!((updated.name.as_str(), updated.active), ("Renamed", true));
    let slots = repo::write(&db, move |tx| {
        cal::replace_availability(
            tx,
            a_id,
            &[
                AvailabilityInput {
                    weekday: 2,
                    start_minute: 60,
                    end_minute: 120,
                },
                AvailabilityInput {
                    weekday: 1,
                    start_minute: 0,
                    end_minute: 30,
                },
            ],
        )
    })
    .await
    .expect("availability");
    assert_eq!(slots.len(), 2);
    let edit = repo::read(&db, move |c| cal::get_for_edit(c, ws, a_id))
        .await
        .expect("edit")
        .expect("calendar");
    assert_eq!(
        edit.availability
            .iter()
            .map(|a| a.weekday)
            .collect::<Vec<_>>(),
        vec![1, 2]
    );
    let flow = repo::read(&db, move |c| cal::list_for_flow(c, ws, Some("renamed")))
        .await
        .expect("flow");
    assert_eq!(flow.len(), 1);
    let slug = repo::read(&db, move |c| {
        cal::find_by_public_link_slug(c, ws, "consult-2")
    })
    .await
    .expect("slug");
    assert_eq!(slug.map(|c| c.id), Some(b_id));
    let deleted = repo::write(&db, move |tx| cal::soft_delete_many(tx, ws, &[b_id, 1]))
        .await
        .expect("delete");
    assert_eq!(deleted, vec![b_id]);
    let gone = repo::read(&db, move |c| cal::find_by(c, ws, b_id, false))
        .await
        .expect("find");
    assert!(gone.is_none());
    let kept = repo::read(&db, move |c| cal::find_by(c, ws, b_id, true))
        .await
        .expect("find");
    assert!(kept.is_some_and(|c| c.deleted_at.is_some()));
}

#[tokio::test]
async fn contact_inbox_relational_finders() {
    use chatbotx_db::repo::contact_inbox::{self as ci, IdentityField};
    let (db, s) = contact_db().await;
    let ws = s.workspace_id;
    db.write(move |tx| {
        tx.execute(
            r#"UPDATE "Contact" SET "lastName" = 'Lovelace', "phoneNumber" = '+1' WHERE "id" = ?1"#,
            params![s.contact_id],
        )?;
        tx.execute(
            r#"UPDATE "ContactInbox" SET "lastMessageAt" = 5, "referral" = '{"gclid":"g1","googleClickReceivedAt":"2026-01-01T00:00:00.000Z"}' WHERE "id" = 80"#,
            [],
        )?;
        Ok(())
    })
    .await
    .expect("fixture");
    let named = repo::read(&db, move |c| {
        ci::list_with_inbox_name_by_contact_id(c, ws, s.contact_id)
    })
    .await
    .expect("named");
    assert_eq!((named.len(), named[0].inbox_name.as_str()), (1, "Inbox"));
    let by_phone = repo::read(&db, move |c| {
        ci::find_latest_by_inbox_and_contact_identity(
            c,
            ws,
            s.inbox_id,
            IdentityField::PhoneNumber,
            "+1",
        )
    })
    .await
    .expect("phone");
    assert_eq!(by_phone.map(|r| r.contact_inbox_id), Some(80));
    let by_name = repo::read(&db, move |c| {
        ci::find_by_inbox_and_full_name(c, ws, s.inbox_id, "Ada Lovelace")
    })
    .await
    .expect("name");
    assert_eq!(
        by_name.iter().map(|r| r.contact_id).collect::<Vec<_>>(),
        vec![s.contact_id]
    );
    let full = repo::read(&db, move |c| ci::find_full_by_id_for_workspace(c, ws, 80))
        .await
        .expect("full");
    assert!(full.is_some());
    let foreign = repo::read(&db, move |c| {
        ci::find_full_by_id_for_workspace(c, ws + 1, 80)
    })
    .await
    .expect("foreign");
    assert!(foreign.is_none());
    let for_contact = repo::read(&db, move |c| {
        ci::find_by_id_for_contact(c, ws, s.contact_id, 80)
    })
    .await
    .expect("for contact");
    assert_eq!(for_contact.map(|r| r.inbox_id), Some(s.inbox_id));
    let recent = repo::read(&db, move |c| {
        ci::find_most_recent_by_contact(c, ws, s.contact_id)
    })
    .await
    .expect("recent");
    assert_eq!(recent.map(|r| r.id), Some(80));
    let click = repo::read(&db, move |c| ci::find_google_click_attribution(c, ws, 80))
        .await
        .expect("click");
    assert_eq!(click.map(|r| r.contact_id), Some(s.contact_id));
    let latest = repo::read(&db, move |c| {
        ci::find_latest_google_click_inbox_by_contact(c, ws, s.contact_id)
    })
    .await
    .expect("latest");
    assert_eq!(latest.map(|r| r.id), Some(80));
    let with = repo::read(&db, move |c| {
        ci::find_with_conversation_and_contact(c, &Sql::bind("\"ContactInbox\".\"id\" = ", 80))
    })
    .await
    .expect("with")
    .expect("a row");
    assert_eq!(
        (with.contact.id, with.conversation.map(|c| c.id)),
        (s.contact_id, Some(90))
    );
    let only = repo::read(&db, move |c| {
        ci::find_with_contact(c, &Sql::bind("\"ContactInbox\".\"id\" = ", 80))
    })
    .await
    .expect("with contact")
    .expect("a row");
    assert!(only.conversation.is_none());
    let ids = repo::read(&db, move |c| {
        ci::list_ids_by_inbox_and_source_ids(c, s.inbox_id, &["psid-1".to_owned()])
    })
    .await
    .expect("ids");
    assert_eq!(ids, vec![(80, s.contact_id, "psid-1".to_owned())]);
    let identity = repo::read(&db, move |c| {
        ci::list_identity_columns_by_inbox_and_source_ids(c, s.inbox_id, &["psid-1".to_owned()])
    })
    .await
    .expect("identity");
    assert_eq!(identity.len(), 1);
    let conv = repo::read(&db, move |c| ci::find_by_id_with_conversation(c, 80))
        .await
        .expect("conv")
        .expect("row");
    assert_eq!(conv.conversation.map(|c| c.id), Some(90));
    let page = repo::read(&db, move |c| {
        ci::list_by_inbox_page(c, s.inbox_id, Some(80), 10)
    })
    .await
    .expect("page");
    assert!(page.is_empty());
    let contact_ids = repo::read(&db, move |c| ci::list_contact_ids_by_ids(c, &[80]))
        .await
        .expect("contact ids");
    assert_eq!(contact_ids, vec![s.contact_id]);
    let by_contact = repo::read(&db, move |c| ci::list_by_contact_id(c, s.contact_id))
        .await
        .expect("by contact");
    assert_eq!(by_contact.len(), 1);
}

// ---------------------------------------------------------------------------
// Slice E1 (flow, file, inbound-webhook): the functions without a TS test.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn flow_prefix_names_and_existence_are_scoped() {
    use chatbotx_db::repo::flow;
    let db = fresh_db().await;
    let s = seed(&db).await;
    db.write(move |tx| {
        for (id, name) in [
            (61, "Welcome"),
            (62, "Welcome 2"),
            (63, "welcome lower"),
            (64, "Other"),
        ] {
            tx.execute(
                r#"INSERT INTO "Flow" ("id", "name", "workspaceId") VALUES (?1, ?2, ?3)"#,
                params![id, name, s.workspace_id],
            )?;
        }
        Ok(())
    })
    .await
    .expect("seed flows");
    let ws = s.workspace_id;
    let names = repo::read(&db, move |c| flow::list_names_with_prefix(c, ws, "Welcome"))
        .await
        .expect("names");
    assert_eq!(names, vec!["Welcome".to_owned(), "Welcome 2".to_owned()]);
    let pct = repo::read(&db, move |c| flow::list_names_with_prefix(c, ws, "%"))
        .await
        .expect("names");
    assert!(pct.is_empty());
    let found = repo::read(&db, move |c| flow::exists_in_workspace(c, ws, 61))
        .await
        .expect("exists");
    let foreign = repo::read(&db, move |c| flow::exists_in_workspace(c, ws + 1, 61))
        .await
        .expect("exists");
    assert!(found && !foreign);
}

#[tokio::test]
async fn file_create_fills_id_and_timestamps() {
    use chatbotx_db::repo::file;
    use chatbotx_db::rusqlite::types::Value;
    let db = fresh_db().await;
    let s = seed(&db).await;
    let ws = s.workspace_id;
    let row = repo::write(&db, move |c| {
        file::create(
            c,
            &vec![
                ("workspaceId", Value::Integer(ws)),
                ("contextType", Value::Text("export".to_owned())),
                ("subType", Value::Text("contacts".to_owned())),
                ("path", Value::Text("exports/a.csv".to_owned())),
                ("fileName", Value::Text("a.csv".to_owned())),
                ("mimeType", Value::Text("text/csv".to_owned())),
            ],
        )
    })
    .await
    .expect("create");
    assert!(row.id > 0);
    assert_eq!(row.status, "pending");
    assert_eq!(row.created_at, row.updated_at);
    let id = row.id;
    let again = repo::read(&db, move |c| {
        file::find_by_id_for_workspace(c, id, ws, Some(1))
    })
    .await
    .expect("find");
    assert_eq!(again, None, "user filter narrows to the uploader");
    let bad = repo::write(&db, move |c| {
        file::create(c, &vec![("nope", Value::Integer(1))])
    })
    .await;
    assert!(bad.is_err());
}

#[tokio::test]
async fn inbound_webhook_insert_update_find_count_and_request_lookup() {
    use chatbotx_db::repo::inbound_webhook as ib;
    use chatbotx_db::rusqlite::types::Value;
    let db = fresh_db().await;
    let s = seed(&db).await;
    let ws = s.workspace_id;
    let hook = repo::write(&db, move |c| {
        ib::insert_inbound_webhook(
            c,
            &vec![
                ("workspaceId", Value::Integer(ws)),
                ("name", Value::Text("Orders".to_owned())),
                ("token", Value::Text("iwh_1".to_owned())),
                ("identifiers", Value::Text("[]".to_owned())),
                ("fieldMappings", Value::Text("[]".to_owned())),
                ("actions", Value::Text("[]".to_owned())),
            ],
        )
    })
    .await
    .expect("insert");
    assert_eq!((hook.name.as_str(), hook.request_count), ("Orders", 0));
    let id = hook.id;
    let renamed = repo::write(&db, move |c| {
        ib::update_inbound_webhook(c, id, ws, &vec![("name", Value::Text("Sales".to_owned()))])
    })
    .await
    .expect("update");
    assert_eq!(renamed.map(|h| h.name), Some("Sales".to_owned()));
    let foreign = repo::write(&db, move |c| {
        ib::update_inbound_webhook(c, id, ws + 1, &vec![("name", Value::Text("X".to_owned()))])
    })
    .await
    .expect("update");
    assert_eq!(foreign, None);
    let not_allowed = repo::write(&db, move |c| {
        ib::update_inbound_webhook(c, id, ws, &vec![("requestCount", Value::Integer(9))])
    })
    .await;
    assert!(not_allowed.is_err());
    let found = repo::read(&db, move |c| ib::find_inbound_webhook_by_id(c, id, ws))
        .await
        .expect("find");
    assert_eq!(found.map(|h| h.name), Some("Sales".to_owned()));
    let count = repo::read(&db, move |c| ib::count_inbound_webhooks(c, ws))
        .await
        .expect("count");
    assert_eq!(count, 1);
    let request = repo::write(&db, move |c| {
        ib::insert_inbound_webhook_request(
            c,
            &vec![
                ("workspaceId", Value::Integer(ws)),
                ("inboundWebhookId", Value::Integer(id)),
                ("payload", Value::Text("{}".to_owned())),
            ],
        )
    })
    .await
    .expect("insert request")
    .expect("no key, no conflict");
    let rid = request.id;
    let by_hook = repo::read(&db, move |c| {
        ib::find_inbound_webhook_request(c, rid, ws, Some(id))
    })
    .await
    .expect("find request");
    let other_hook = repo::read(&db, move |c| {
        ib::find_inbound_webhook_request(c, rid, ws, Some(id + 1))
    })
    .await
    .expect("find request");
    assert_eq!(by_hook.map(|r| r.status), Some("received".to_owned()));
    assert_eq!(other_hook, None);
}
