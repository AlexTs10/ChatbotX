//! Smoke tests of the P3.2b repository functions that have no TS test (not locked).

mod common;
#[macro_use]
#[path = "common/repo.rs"]
mod repo_common;
#[path = "common/repo10.rs"]
mod repo10;
#[path = "common/repo11.rs"]
mod repo11;
#[path = "common/repo12.rs"]
mod repo12;
#[path = "common/repo3.rs"]
mod repo3;

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

// ---------------------------------------------------------------------------
// coexist-sync-run (slice F1): functions without a TS test
// ---------------------------------------------------------------------------

#[tokio::test]
async fn coexist_sync_run_untested_readers_and_terminal_writes() {
    use chatbotx_db::repo::coexist_sync_run as csr;
    use chatbotx_db::repo::utils::SortItem;
    use repo10::{MESSENGER, WHATSAPP, WS, coexist_db, put_runs, read, run_row, write};
    let db = coexist_db().await;
    let now = repo::now_ms();
    put_runs(
        &db,
        &[
            (1, MESSENGER, "messenger", ("coexist", "partial"), now - 3_000, "\"startedAt\" = 100, \"lastSyncedAt\" = 150"),
            (2, MESSENGER, "messenger", ("coexist", "succeeded"), now - 2_000, "\"startedAt\" = 50"),
            (3, MESSENGER, "messenger", ("coexist", "running"), now - 1_000, "\"claimToken\" = 't', \"attempts\" = 2, \"currentError\" = 'e', \"importedMessageCount\" = 4, \"skippedCount\" = 1, \"failedCount\" = 3"),
            (4, 81, "messenger", ("contact_scan", "succeeded"), now - 2_000, ""),
            (5, 82, "messenger", ("contact_scan", "succeeded"), now - 1_000, ""),
        ],
    )
    .await;
    // Latest by createdAt; by id; owner of the workspace.
    assert_eq!(
        read(&db, |c| csr::find_latest_run(c, WS, MESSENGER, "messenger"))
            .await
            .map(|r| r.id),
        Some(3)
    );
    assert_eq!(
        read(&db, |c| csr::find_run_by_id(c, 2))
            .await
            .map(|r| r.status),
        Some("succeeded".to_owned())
    );
    assert!(read(&db, |c| csr::find_run_by_id(c, 99)).await.is_none());
    assert_eq!(
        read(&db, |c| csr::find_workspace_owner_id(c, WS)).await,
        Some(1001)
    );
    assert_eq!(
        read(&db, |c| csr::find_workspace_owner_id(c, 9)).await,
        None
    );
    // A partial prior run resumes from lastSyncedAt (it sorts first: startedAt 100 > 50).
    assert_eq!(
        read(&db, |c| csr::find_resume_ceiling(
            c,
            MESSENGER,
            "messenger",
            3
        ))
        .await,
        Some(150)
    );
    assert_eq!(
        read(&db, |c| csr::find_last_synced_at(c, 1)).await,
        Some(Some(150))
    );
    assert_eq!(read(&db, |c| csr::find_last_synced_at(c, 99)).await, None);
    let init = read(&db, |c| csr::find_init_state(c, 3))
        .await
        .expect("init state");
    assert_eq!(
        (
            init.attempts,
            init.current_error.as_deref(),
            init.messenger_sync_phase.as_str()
        ),
        (2, Some("e"), "contacts")
    );
    let counters = read(&db, |c| csr::find_terminal_counters(c, 3))
        .await
        .expect("counters");
    assert_eq!(
        (
            counters.imported_message_count,
            counters.skipped_count,
            counters.failed_count
        ),
        (4, 1, 3)
    );
    // markPartial without an error keeps currentError; markSucceeded clears the cursor.
    let guard = csr::WriteGuard {
        status: "running",
        claim_token: Some("t"),
    };
    assert_eq!(
        write(&db, move |c| csr::mark_partial(c, 3, None, Some(&guard))).await,
        1
    );
    let row = run_row(&db, 3).await.expect("run 3");
    assert_eq!(
        (row.status.as_str(), row.current_error.as_deref()),
        ("partial", Some("e"))
    );
    assert!(row.finished_at.is_some());
    assert_eq!(write(&db, |c| csr::mark_succeeded(c, 3, None)).await, 1);
    assert_eq!(run_row(&db, 3).await.expect("run 3").status, "succeeded");
    // Coexist toggle on the channel's integration table, workspace-scoped.
    let row = write(&db, |c| {
        csr::set_integration_coexist_enabled(c, "whatsapp", WS, WHATSAPP, true, Some(true))
    })
    .await;
    let row = row.expect("updated");
    assert_eq!(
        (
            row.channel(),
            row.id(),
            row.coexist_enabled(),
            row.coexist_ai_reads_synced_history()
        ),
        ("whatsapp", WHATSAPP, true, true)
    );
    let row = write(&db, |c| {
        csr::set_integration_coexist_enabled(c, "whatsapp", WS, WHATSAPP, false, None)
    })
    .await;
    let row = row.expect("updated");
    assert_eq!(
        (row.coexist_enabled(), row.coexist_ai_reads_synced_history()),
        (false, true)
    );
    assert!(
        write(&db, |c| csr::set_integration_coexist_enabled(
            c,
            "messenger",
            2002,
            MESSENGER,
            true,
            None
        ))
        .await
        .is_none()
    );
    let unknown = repo::read(&db, |c| {
        csr::find_integration_for_coexist(c, WS, 1, "telegram")
    })
    .await;
    assert!(unknown.is_err());
    // A caller sort replaces the createdAt DESC default.
    let page = read(&db, |c| {
        csr::list_contact_scan_runs(
            c,
            &csr::ListContactScanRunsInput {
                workspace_id: WS,
                sort: Some(vec![SortItem {
                    id: "createdAt".to_owned(),
                    desc: false,
                }]),
                ..Default::default()
            },
        )
    })
    .await;
    assert_eq!(
        page.data.iter().map(|r| r.id).collect::<Vec<_>>(),
        vec![4, 5]
    );
}

// ---------------------------------------------------------------------------
// integration-messenger (slice G1): functions without a TS test
// ---------------------------------------------------------------------------

#[tokio::test]
async fn integration_messenger_untested_readers_and_capi_writes() {
    use chatbotx_db::repo::Json;
    use chatbotx_db::repo::integration_messenger as im;
    use repo10::{MESSENGER, OTHER_WS, WS, coexist_db, read, write};
    let db = coexist_db().await;
    db.write(|tx| {
        repo3::integration_messenger(tx, 67, OTHER_WS, 3067, "[]")?;
        tx.execute_batch(
            r#"UPDATE "Inbox" SET "status" = 'connected' WHERE "id" IN (3061, 3067)"#,
        )?;
        Ok(())
    })
    .await
    .expect("pages");
    let refs = read(&db, |c| im::list_by_workspace_id(c, WS)).await;
    assert_eq!(
        refs.iter()
            .map(|r| (r.id, r.page_id.as_str()))
            .collect::<Vec<_>>(),
        vec![(MESSENGER, "page-61")]
    );
    assert!(
        read(&db, |c| im::find_workspace_integration(
            c, MESSENGER, OTHER_WS
        ))
        .await
        .is_none()
    );
    assert_eq!(
        read(&db, |c| im::find_workspace_integration_by_inbox_id(
            c, WS, 3061
        ))
        .await,
        Some(MESSENGER)
    );
    assert_eq!(
        read(&db, |c| im::find_by_id(c, 67))
            .await
            .map(|r| r.workspace_id),
        Some(OTHER_WS)
    );
    assert_eq!(
        read(&db, |c| im::find_by_page_id_unscoped(c, "page-67"))
            .await
            .map(|r| r.id),
        Some(67)
    );
    assert_eq!(
        read(&db, |c| im::find_by_inbox_id(c, 3061))
            .await
            .map(|r| r.id),
        Some(MESSENGER)
    );
    assert_eq!(
        read(&db, |c| im::list_by_workspace(c, OTHER_WS))
            .await
            .len(),
        1
    );
    // Scope-cache compare-and-set: a stale expectation returns the current row unchanged.
    let row = write(&db, |c| {
        im::update_capi_scope_cache(c, MESSENGER, WS, true, Some(500), None)
    })
    .await
    .expect("row");
    assert_eq!(
        (row.has_capi_scope, row.capi_scope_checked_at),
        (true, Some(500))
    );
    let row = write(&db, |c| {
        im::update_capi_scope_cache(c, MESSENGER, WS, false, Some(600), None)
    })
    .await
    .expect("row");
    assert_eq!(
        (row.has_capi_scope, row.capi_scope_checked_at),
        (true, Some(500))
    );
    assert!(
        write(&db, |c| im::claim_capi_scope_cache_refresh(
            c, MESSENGER, WS, 700, None
        ))
        .await
        .is_none()
    );
    let row = write(&db, |c| {
        im::claim_capi_scope_cache_refresh(c, MESSENGER, WS, 700, Some(500))
    })
    .await
    .expect("row");
    assert_eq!(row.capi_scope_checked_at, Some(700));
    // Dataset: only-if-null, then unconditional.
    assert_eq!(
        write(&db, |c| im::update_dataset_id_if_null(
            c, MESSENGER, WS, "d1"
        ))
        .await
        .and_then(|r| r.dataset_id),
        Some("d1".to_owned())
    );
    assert!(
        write(&db, |c| im::update_dataset_id_if_null(
            c, MESSENGER, WS, "d2"
        ))
        .await
        .is_none()
    );
    assert_eq!(
        write(&db, |c| im::update_dataset_id(c, MESSENGER, WS, "d3"))
            .await
            .and_then(|r| r.dataset_id),
        Some("d3".to_owned())
    );
    assert_eq!(
        write(&db, |c| im::update_capi_test_event_code(
            c,
            MESSENGER,
            WS,
            Some("T1")
        ))
        .await
        .and_then(|r| r.capi_test_event_code),
        Some("T1".to_owned())
    );
    let token = Json(serde_json::json!({ "iv": "a", "data": "b" }));
    let t = token.clone();
    assert_eq!(
        write(&db, move |c| im::update_capi_access_token(
            c, MESSENGER, WS, &t
        ))
        .await
        .and_then(|r| r.capi_access_token),
        Some(token.clone())
    );
    let row = write(&db, |c| im::set_capi_disconnected_at(c, MESSENGER, WS, 900))
        .await
        .expect("row");
    assert_eq!(
        (row.capi_disconnected_at, row.capi_access_token),
        (Some(900), None)
    );
    let t = token.clone();
    let row = write(&db, move |c| {
        im::connect_custom_capi(c, MESSENGER, WS, "d4", &t)
    })
    .await
    .expect("row");
    assert_eq!(
        (
            row.dataset_id.as_deref(),
            row.capi_disconnected_at,
            row.capi_access_token
        ),
        (Some("d4"), None, Some(token))
    );
    assert_eq!(
        write(&db, |c| im::clear_capi_access_token(c, MESSENGER, WS))
            .await
            .and_then(|r| r.capi_access_token),
        None
    );
    write(&db, |c| im::set_capi_disconnected_at(c, MESSENGER, WS, 901)).await;
    assert_eq!(
        write(&db, |c| im::clear_capi_disconnected_at(c, MESSENGER, WS))
            .await
            .and_then(|r| r.capi_disconnected_at),
        None
    );
    assert!(
        write(&db, |c| im::clear_capi_access_token(c, MESSENGER, OTHER_WS))
            .await
            .is_none()
    );
    // Webhook targets: connected inboxes only, keyset by id.
    let all = read(&db, |c| {
        im::list_connected_for_webhook_subscription(c, None, 10)
    })
    .await;
    assert_eq!(
        all.iter().map(|t| t.id).collect::<Vec<_>>(),
        vec![MESSENGER, 67]
    );
    let after = read(&db, |c| {
        im::list_connected_for_webhook_subscription(c, Some(MESSENGER), 10)
    })
    .await;
    assert_eq!(after.iter().map(|t| t.id).collect::<Vec<_>>(), vec![67]);
}

#[tokio::test]
async fn google_ads_conversion_event_finders_claim_fence_and_options_guard() {
    use chatbotx_db::repo::google_ads_conversion_event::{self as gace, EventRef, RedriveInput};
    use repo11::{OTHER_WS, WS, event_values, i, put_event, read, t, two_ws_db, write};
    let db = two_ws_db().await;
    put_event(&db, 1, WS, vec![("status", t("failed")), ("attempt", i(1))]).await;
    let found = read(&db, |c| gace::find_by_transaction_id(c, WS, "t-1")).await;
    assert_eq!(found.map(|r| r.id), Some(1));
    assert!(
        read(&db, |c| gace::find_by_transaction_id(c, OTHER_WS, "t-1"))
            .await
            .is_none()
    );
    let ev = |workspace_id| EventRef {
        id: 1,
        workspace_id,
    };
    assert!(
        read(&db, move |c| gace::find_workspace_event(c, ev(WS)))
            .await
            .is_some()
    );
    assert!(
        read(&db, move |c| gace::find_workspace_event(c, ev(OTHER_WS)))
            .await
            .is_none()
    );
    // `expectedClaimToken`: a token fence spares an unleased row, `null` matches it.
    let input = |token: Option<Option<String>>| RedriveInput {
        id: 1,
        workspace_id: WS,
        from_statuses: vec!["failed".to_owned()],
        expected_attempt: 1,
        expected_request_id: None,
        expected_claim_token: token,
    };
    let leased = input(Some(Some("token".to_owned())));
    assert!(
        write(&db, move |c| gace::redrive(c, &leased))
            .await
            .is_none()
    );
    let unleased = input(Some(None));
    assert!(
        write(&db, move |c| gace::redrive(c, &unleased))
            .await
            .is_some()
    );
    // The options snapshot is required.
    let mut values = event_values(WS, "t-x", &[]);
    values.retain(|(c, _)| *c != "options");
    let r = repo::write(&db, move |tx| gace::insert_ignore_duplicate(tx, &values)).await;
    assert!(repo_common::err_message(r).contains("options"));
}

#[tokio::test]
async fn facebook_marketing_message_campaign_and_auth_functions() {
    use chatbotx_db::repo::facebook_marketing_message::{
        self as fmm, AuthCreateInput, AuthUpdateInput, CreateInput, StatisticsListInput,
    };
    use chatbotx_db::repo::utils::SortItem;
    use chatbotx_db::rusqlite::types::Value;
    use repo11::{OTHER_WS, WS, campaign, exec, put_campaign, read, two_ws_db, write};
    let db = two_ws_db().await;
    let auth = write(&db, |c| {
        fmm::auth_create(
            c,
            &AuthCreateInput {
                id: 1,
                workspace_id: WS,
                auth: serde_json::json!({ "t": 1 }),
                token_expires_at: Some(5),
                facebook_user_id: None,
            },
        )
    })
    .await;
    assert_eq!(auth.status, "active");
    write(&db, |c| fmm::auth_update_status(c, WS, "invalid")).await;
    let updated = write(&db, |c| {
        fmm::auth_update_auth(
            c,
            &AuthUpdateInput {
                id: 1,
                auth: serde_json::json!({ "t": 2 }),
                token_expires_at: None,
                facebook_user_id: Some("u".to_owned()),
            },
        )
    })
    .await
    .expect("updated");
    assert_eq!(
        (updated.status.as_str(), updated.token_expires_at),
        ("active", None)
    );
    assert!(
        read(&db, |c| fmm::auth_find_by_workspace_id(c, WS))
            .await
            .is_some()
    );
    assert!(
        read(&db, |c| fmm::auth_find_by_workspace_id(c, OTHER_WS))
            .await
            .is_none()
    );

    let created = write(&db, |c| {
        fmm::create(
            c,
            &CreateInput {
                id: 10,
                workspace_id: WS,
                name: "Spring sale".to_owned(),
                page_id: "page-1".to_owned(),
                ad_account_id: "act_2".to_owned(),
                currency: "EUR".to_owned(),
                currency_offset: 100,
                budget_type: "daily".to_owned(),
                budget_minor_units: 500,
                content: serde_json::json!({ "v": 1 }),
                target_type: "optIn".to_owned(),
                campaign_id: "meta-10".to_owned(),
                ..CreateInput::default()
            },
        )
    })
    .await;
    assert_eq!(
        (created.status.as_str(), created.day_sent),
        ("scheduled", 0)
    );
    exec(
        &db,
        r#"INSERT INTO "FacebookMarketingMessageAudience" ("id", "workspaceId", "customAudienceId", "adAccountId", "pageId", "name", "rowsUploaded") VALUES (77, 2001, 'ca', 'act_1', 'page-1', 'a', 0)"#,
    )
    .await;
    put_campaign(
        &db,
        11,
        WS,
        "sending",
        "\"createdAt\" = 1, \"audienceId\" = 77",
    )
    .await;
    put_campaign(&db, 12, WS, "sending", "\"createdAt\" = 2").await;
    put_campaign(&db, 13, OTHER_WS, "sending", "").await;
    exec(
        &db,
        r#"UPDATE "FacebookMarketingMessage" SET "createdAt" = 3 WHERE "id" = 10"#,
    )
    .await;
    let all = read(&db, |c| fmm::list_by_workspace_id(c, WS)).await;
    assert_eq!(
        all.iter().map(|r| r.id).collect::<Vec<_>>(),
        vec![10, 12, 11]
    );
    let accounts = read(&db, |c| fmm::list_ad_accounts_by_workspace_id(c, WS)).await;
    assert_eq!(
        accounts
            .iter()
            .map(|a| (a.ad_account_id.as_str(), a.currency.as_str()))
            .collect::<Vec<_>>(),
        vec![("act_2", "EUR"), ("act_1", "USD")]
    );
    let page = read(&db, |c| {
        fmm::list_for_statistics(
            c,
            &StatisticsListInput {
                workspace_id: WS,
                ad_account_id: "act_1".to_owned(),
                search: Some(" C1 ".to_owned()),
                sort: Some(vec![SortItem {
                    id: "name".to_owned(),
                    desc: false,
                }]),
                ..StatisticsListInput::default()
            },
        )
    })
    .await;
    assert_eq!(
        page.data.iter().map(|r| r.id).collect::<Vec<_>>(),
        vec![11, 12]
    );
    assert_eq!((page.total, page.page_count), (2, 1));
    let by_meta = read(&db, |c| {
        fmm::list_for_statistics(
            c,
            &StatisticsListInput {
                workspace_id: WS,
                ad_account_id: "act_1".to_owned(),
                search: Some("meta-12".to_owned()),
                ..StatisticsListInput::default()
            },
        )
    })
    .await;
    assert_eq!(
        by_meta.data.iter().map(|r| r.id).collect::<Vec<_>>(),
        vec![12]
    );
    assert!(
        read(&db, |c| fmm::find_for_workspace(c, 13, WS))
            .await
            .is_none()
    );
    assert_eq!(read(&db, |c| fmm::count_by_audience_id(c, 77, WS)).await, 1);
    assert_eq!(
        read(&db, |c| fmm::find_by_campaign_id(c, "meta-13"))
            .await
            .map(|r| r.id),
        Some(13)
    );
    assert_eq!(
        read(&db, |c| fmm::list_by_status(c, "sending")).await.len(),
        3
    );
    let patch = vec![("status", Value::Text("paused".to_owned()))];
    let p2 = patch.clone();
    assert!(
        write(&db, move |c| fmm::transition(c, 11, &["scheduled"], &p2))
            .await
            .is_none()
    );
    let moved = write(&db, move |c| fmm::transition(c, 11, &["sending"], &patch)).await;
    assert_eq!(moved.map(|r| r.status), Some("paused".to_owned()));
    let frozen = vec![("campaignId", Value::Text("x".to_owned()))];
    let r =
        chatbotx_db::repo::write(&db, move |c| fmm::transition(c, 11, &["paused"], &frozen)).await;
    assert!(repo_common::err_message(r).contains("campaignId"));
    write(&db, |c| fmm::increment_day_sent(c, 11, 3)).await;
    write(&db, |c| fmm::set_first_sent_at(c, 11)).await;
    let first = campaign(&db, 11).await.first_sent_at;
    write(&db, |c| fmm::set_first_sent_at(c, 11)).await;
    let row = campaign(&db, 11).await;
    assert_eq!((row.day_sent, row.first_sent_at), (3, first));
    assert!(first.is_some());
    write(&db, |c| fmm::remove(c, &[11, 13], WS)).await;
    assert!(read(&db, |c| fmm::find_by_id(c, 11)).await.is_none());
    assert!(read(&db, |c| fmm::find_by_id(c, 13)).await.is_some());
}

#[tokio::test]
async fn facebook_marketing_message_recipient_and_audience_functions() {
    use chatbotx_db::repo::facebook_marketing_message_audience::{
        self as aud, CreateInput, ListFilter,
    };
    use chatbotx_db::repo::facebook_marketing_message_recipient::{
        self as rec, FailRemainingInput,
    };
    use chatbotx_db::rusqlite::types::Value;
    use repo11::{OTHER_WS, WS, put_campaign, put_recipient, read, recipient, two_ws_db, write};
    let db = two_ws_db().await;
    put_campaign(&db, 1, WS, "sending", "").await;
    let row = |token: &str| {
        vec![
            ("workspaceId", Value::Integer(WS)),
            ("campaignRowId", Value::Integer(1)),
            ("subscriptionToken", Value::Text(token.to_owned())),
            ("status", Value::Text("pending".to_owned())),
        ]
    };
    let rows = vec![row("a"), row("b"), row("a")];
    assert_eq!(write(&db, move |c| rec::insert_many(c, &rows)).await, 2);
    let due = read(&db, |c| {
        rec::list_due(c, 1, chatbotx_db::repo::now_ms(), 10)
    })
    .await;
    assert_eq!(due.len(), 2);
    let a = read(&db, |c| rec::find_by_campaign_token(c, 1, "a"))
        .await
        .expect("a");
    let patch = vec![("trackingId", Value::Text("trk".to_owned()))];
    write(&db, move |c| rec::update(c, a.id, &patch)).await;
    assert_eq!(
        read(&db, |c| rec::find_by_tracking_id(c, "trk"))
            .await
            .map(|r| r.id),
        Some(a.id)
    );
    put_recipient(&db, 50, 1, "unconfirmed", "\"sentAt\" = 100").await;
    put_recipient(&db, 51, 1, "unconfirmed", "\"sentAt\" = 900").await;
    assert_eq!(
        write(&db, |c| rec::fail_stale_unconfirmed(c, 1, 500, 1000)).await,
        1
    );
    assert_eq!(
        recipient(&db, 50).await.error_source.as_deref(),
        Some("send")
    );
    let input = FailRemainingInput {
        campaign_row_id: 1,
        error_code: Some(131_000),
        error_message: "stopped".to_owned(),
        now: 2000,
        ..FailRemainingInput::default()
    };
    // Default statuses: the two pending rows and unconfirmed 51.
    assert_eq!(write(&db, move |c| rec::fail_remaining(c, &input)).await, 3);
    let outcomes = read(&db, |c| rec::count_outcomes(c, 1)).await;
    assert_eq!(
        (outcomes.failed, outcomes.pending, outcomes.unconfirmed),
        (4, 0, 0)
    );
    let breakdown = read(&db, |c| rec::error_breakdown(c, 1)).await;
    assert_eq!(
        breakdown
            .iter()
            .map(|b| (b.error_code, b.error_message.as_str(), b.count))
            .collect::<Vec<_>>(),
        vec![(Some(131_000), "stopped", 3), (None, "outcomeUnknown", 1)]
    );

    let created = write(&db, |c| {
        aud::create(
            c,
            &CreateInput {
                id: 5,
                workspace_id: WS,
                custom_audience_id: "ca".to_owned(),
                ad_account_id: "act_1".to_owned(),
                page_id: "page-1".to_owned(),
                name: "VIP".to_owned(),
                ..CreateInput::default()
            },
        )
    })
    .await;
    assert_eq!(created.rows_uploaded, 0);
    write(&db, |c| aud::increment_rows_uploaded(c, 5, WS, 7)).await;
    write(&db, |c| aud::increment_rows_uploaded(c, 5, OTHER_WS, 7)).await;
    let ready_only = ListFilter {
        ready_only: true,
        ..ListFilter::default()
    };
    let f2 = ready_only.clone();
    assert!(
        read(&db, move |c| aud::list_by_workspace_id(c, WS, &f2))
            .await
            .is_empty()
    );
    write(&db, |c| aud::mark_ready(c, &[5], WS, 9)).await;
    let listed = read(&db, move |c| aud::list_by_workspace_id(c, WS, &ready_only)).await;
    assert_eq!(
        listed
            .iter()
            .map(|a| (a.rows_uploaded, a.ready_at))
            .collect::<Vec<_>>(),
        vec![(7, Some(9))]
    );
    write(&db, |c| aud::remove(c, 5, WS)).await;
    assert!(
        read(&db, |c| aud::find_for_workspace(c, 5, WS))
            .await
            .is_none()
    );
}

#[tokio::test]
async fn whatsapp_signup_session_create_and_purge() {
    use chatbotx_db::repo::whatsapp_signup_session::{self as wss, CreateInput};
    use repo11::{WS, put_signup_session, read, two_ws_db, write};
    let db = two_ws_db().await;
    let created = write(&db, |c| {
        wss::create_signup_session(
            c,
            &CreateInput {
                user_id: 1001,
                owner_id: 1002,
                workspace_id: Some(WS),
                waba_id: "waba".to_owned(),
                business_id: "biz".to_owned(),
                encrypted_access_token: serde_json::json!({ "iv": "x" }),
                api_version: "v21.0".to_owned(),
                candidate_phone_number_ids: vec!["pn-1".to_owned()],
                now: 1_000,
            },
        )
    })
    .await;
    assert_eq!(
        created.expires_at,
        1_000 + wss::WHATSAPP_SIGNUP_SESSION_TTL_MS
    );
    let id = created.id;
    assert!(
        read(&db, move |c| wss::find_active_signup_session_for_user(
            c, id, 1001, 1_000
        ))
        .await
        .is_some()
    );
    put_signup_session(&db, 2, &["pn-1"], &[], 500).await;
    put_signup_session(&db, 3, &["pn-1"], &[], 900_000).await;
    // Session 2 expired at 500; the new session expires after 700 000; session 3 is live.
    assert_eq!(
        write(&db, |c| wss::purge_finished_signup_sessions(c, 700_000, 10)).await,
        2
    );
    assert!(
        read(&db, |c| wss::find_active_signup_session_for_user(
            c, 3, 1001, 700_000
        ))
        .await
        .is_some()
    );
}

/// Two numbers (in two new workspaces of user 1001) for the integration-whatsapp smoke
/// tests.
async fn two_numbers() -> (common::TestDb, repo11::SeededNumber, repo11::SeededNumber) {
    let db = repo11::two_ws_db().await;
    let a = repo11::seed_number(&db, 1001, None, "waba-a", "pn-a", false, false, None).await;
    let b = repo11::seed_number(&db, 1001, None, "waba-b", "pn-b", false, false, None).await;
    (db, a, b)
}

#[tokio::test]
async fn integration_whatsapp_readers() {
    use chatbotx_db::repo::integration_whatsapp as iw;
    use repo11::{exec, read};
    let (db, a, b) = two_numbers().await;
    let (aid, aws, ainbox) = (a.integration_id, a.workspace_id, a.inbox_id);
    let (bid, bws) = (b.integration_id, b.workspace_id);
    exec(
        &db,
        format!(
            r#"UPDATE "IntegrationWhatsapp" SET "callRecordingEnabled" = 1, "verificationCodeRequestedAt" = 77, "displayPhoneNumber" = '+1 (555) 010' WHERE "id" = {aid};
               UPDATE "Inbox" SET "name" = 'Sales' WHERE "id" = {ainbox};"#
        ),
    )
    .await;
    let ids = vec!["pn-a".to_owned(), "pn-x".to_owned()];
    let connected = read(&db, move |c| iw::find_connected_phone_number_ids(c, &ids)).await;
    assert_eq!(connected.into_iter().collect::<Vec<_>>(), vec!["pn-a"]);
    assert!(
        read(&db, |c| iw::find_connected_phone_number_ids(c, &[]))
            .await
            .is_empty()
    );
    let all = read(&db, iw::find_all_for_token_refresh).await;
    let all_ids: Vec<(i64, String)> = all.into_iter().map(|r| (r.id, r.waba_id)).collect();
    assert!(
        all_ids.contains(&(aid, "waba-a".to_owned()))
            && all_ids.contains(&(bid, "waba-b".to_owned()))
    );
    let by_ws = read(&db, move |c| {
        iw::find_for_token_refresh_by_workspace_ids(c, &[bws])
    })
    .await;
    assert_eq!(
        by_ws
            .iter()
            .map(|r| (r.id, r.workspace_id))
            .collect::<Vec<_>>(),
        vec![(bid, bws)]
    );
    assert!(
        read(&db, |c| iw::find_for_token_refresh_by_workspace_ids(c, &[]))
            .await
            .is_empty()
    );
    // Workspace scoping of the id and inbox lookups.
    assert_eq!(
        read(&db, move |c| iw::find_by_id_for_workspace(c, aid, aws))
            .await
            .map(|r| r.id),
        Some(aid)
    );
    assert!(
        read(&db, move |c| iw::find_by_id_for_workspace(c, aid, bws))
            .await
            .is_none()
    );
    assert_eq!(
        read(&db, move |c| iw::find_by_id_unscoped(c, aid))
            .await
            .map(|r| r.workspace_id),
        Some(aws)
    );
    assert_eq!(
        read(&db, move |c| iw::find_workspace_integration_by_inbox_id(
            c, aws, ainbox
        ))
        .await,
        Some(iw::IntegrationWabaRef {
            id: aid,
            waba_id: "waba-a".to_owned()
        })
    );
    assert!(
        read(&db, move |c| iw::find_workspace_integration_by_inbox_id(
            c, bws, ainbox
        ))
        .await
        .is_none()
    );
    assert!(
        read(&db, move |c| iw::find_auth_by_inbox_id_for_workspace(
            c, aws, ainbox
        ))
        .await
        .is_some()
    );
    assert_eq!(
        read(&db, move |c| iw::find_by_inbox_id_for_workspace(
            c, aws, ainbox
        ))
        .await
        .map(|r| r.id),
        Some(aid)
    );
    assert!(
        read(&db, move |c| iw::is_call_recording_enabled_for_inbox(
            c, aws, ainbox
        ))
        .await
    );
    assert!(
        !read(&db, move |c| iw::is_call_recording_enabled_for_inbox(
            c, bws, ainbox
        ))
        .await
    );
    assert_eq!(
        read(&db, move |c| iw::find_verification_code_requested_at(
            c, aid, aws
        ))
        .await,
        Some(Some(77))
    );
    assert_eq!(
        read(&db, move |c| iw::find_verification_code_requested_at(
            c, bid, bws
        ))
        .await,
        Some(None)
    );
    assert_eq!(
        read(&db, move |c| iw::find_verification_code_requested_at(
            c, aid, bws
        ))
        .await,
        None
    );
    // Phone number lookup, with and without the WABA filter (an empty WABA is no filter).
    for (waba, want) in [
        (None, Some(aid)),
        (Some(""), Some(aid)),
        (Some("waba-a"), Some(aid)),
        (Some("waba-b"), None),
    ] {
        let got = read(&db, move |c| iw::find_by_phone_number_id(c, "pn-a", waba)).await;
        assert_eq!(got.map(|r| r.id), want, "{waba:?}");
    }
    let listed = read(&db, move |c| iw::list_by_workspace_id(c, aws)).await;
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].integration.id, aid);
    assert_eq!(
        listed[0].inbox,
        Some(iw::InboxName {
            id: ainbox,
            name: "Sales".to_owned()
        })
    );
    assert_eq!(
        read(&db, move |c| iw::list_by_display_phone_digits(
            c, aws, "1555010"
        ))
        .await
        .len(),
        1
    );
}

#[tokio::test]
async fn integration_whatsapp_auth_registration_and_routing_writes() {
    use chatbotx_db::repo::Json;
    use chatbotx_db::repo::integration_whatsapp as iw;
    use chatbotx_db::rusqlite::types::Value;
    use repo11::{exec, read, write};
    let (db, a, b) = two_numbers().await;
    let (aid, aws, bws) = (a.integration_id, a.workspace_id, b.workspace_id);
    exec(
        &db,
        format!(
            r#"UPDATE "IntegrationWhatsapp" SET "tokenRefreshError" = 'expired' WHERE "id" = {aid}"#
        ),
    )
    .await;
    let auth = Json(serde_json::json!({ "accessToken": "new" }));
    let auth2 = auth.clone();
    assert_eq!(
        write(&db, move |c| iw::update_auth(c, aid, bws, &auth2)).await,
        None
    );
    let auth2 = auth.clone();
    assert_eq!(
        write(&db, move |c| iw::update_auth(c, aid, aws, &auth2)).await,
        Some("pn-a".to_owned())
    );
    let row = read(&db, move |c| iw::find_by_id_unscoped(c, aid))
        .await
        .unwrap();
    assert_eq!(
        (row.auth.clone(), row.token_refresh_error.clone()),
        (auth.clone(), None)
    );
    let unscoped = Json(serde_json::json!({ "accessToken": "webhook" }));
    let unscoped2 = unscoped.clone();
    write(&db, move |c| iw::update_auth_unscoped(c, aid, &unscoped2)).await;
    write(&db, move |c| iw::mark_history_declined(c, aid)).await;
    let row = read(&db, move |c| iw::find_by_id_unscoped(c, aid))
        .await
        .unwrap();
    assert_eq!(row.auth, unscoped);
    assert!(row.history_declined);
    // Registration: only the two registration keys are accepted.
    let error = serde_json::json!({ "code": 133010, "message": "not registered" });
    let values = vec![
        ("registrationStatus", Value::Text("failed".to_owned())),
        ("registrationError", Value::Text(error.to_string())),
    ];
    assert_eq!(
        write(&db, move |c| iw::update_registration(c, aid, aws, &values)).await,
        Some(Json(error))
    );
    let values = vec![
        ("registrationStatus", Value::Text("registered".to_owned())),
        ("registrationError", Value::Null),
    ];
    assert_eq!(
        write(&db, move |c| iw::update_registration(c, aid, aws, &values)).await,
        None
    );
    let refused = vec![("name", Value::Text("x".to_owned()))];
    assert!(
        chatbotx_db::repo::write(&db, move |c| iw::update_registration(c, aid, aws, &refused))
            .await
            .is_err()
    );
    let replaced = Json(serde_json::json!({ "accessToken": "replaced" }));
    let replaced2 = replaced.clone();
    let row = write(&db, move |c| {
        iw::replace_auth(c, aid, aws, &replaced2, true, 500)
    })
    .await
    .unwrap();
    assert_eq!(
        (row.auth, row.has_capi_scope, row.capi_scope_checked_at),
        (replaced, true, Some(500))
    );
    assert!(
        write(&db, move |c| iw::replace_auth(
            c,
            aid,
            bws,
            &Json::null(),
            false,
            1
        ))
        .await
        .is_none()
    );
    // Routing roles: a newer snapshot wins, an older or equal one is kept out.
    let roles = |at: &str| Json(serde_json::json!({ "roles": [], "checkedAt": at }));
    let (r1, r0, r2) = (
        roles("2026-11-13T10:00:00Z"),
        roles("2026-11-13T09:00:00Z"),
        roles("2026-11-13T11:00:00Z"),
    );
    let r1b = r1.clone();
    assert_eq!(
        write(&db, move |c| iw::update_routing_roles_if_newer(
            c, aid, aws, &r1b
        ))
        .await
        .and_then(|r| r.routing_roles),
        Some(r1.clone())
    );
    let r1b = r1.clone();
    assert!(
        write(&db, move |c| iw::update_routing_roles_if_newer(
            c, aid, aws, &r0
        ))
        .await
        .is_none()
    );
    assert!(
        write(&db, move |c| iw::update_routing_roles_if_newer(
            c, aid, aws, &r1b
        ))
        .await
        .is_none()
    );
    let r2b = r2.clone();
    assert_eq!(
        write(&db, move |c| iw::update_routing_roles_if_newer(
            c, aid, aws, &r2b
        ))
        .await
        .and_then(|r| r.routing_roles),
        Some(r2)
    );
    let no_date = Json(serde_json::json!({ "roles": [] }));
    assert!(
        chatbotx_db::repo::write(&db, move |c| iw::update_routing_roles_if_newer(
            c, aid, aws, &no_date
        ))
        .await
        .is_err()
    );
}

#[tokio::test]
async fn integration_whatsapp_capi_writes() {
    use chatbotx_db::repo::Json;
    use chatbotx_db::repo::integration_whatsapp as iw;
    use repo11::write;
    let (db, a, b) = two_numbers().await;
    let (aid, aws, bws) = (a.integration_id, a.workspace_id, b.workspace_id);
    // CAS on capiScopeCheckedAt: the initial NULL matches NULL; a stale expectation loses
    // and the cache write returns the current row.
    let claimed = write(&db, move |c| {
        iw::claim_capi_scope_cache_refresh(c, aid, aws, 100, None)
    })
    .await;
    assert_eq!(claimed.map(|r| r.capi_scope_checked_at), Some(Some(100)));
    assert!(
        write(&db, move |c| iw::claim_capi_scope_cache_refresh(
            c, aid, aws, 200, None
        ))
        .await
        .is_none()
    );
    let lost = write(&db, move |c| {
        iw::update_capi_scope_cache(c, aid, aws, true, Some(300), Some(1))
    })
    .await
    .unwrap();
    assert_eq!(
        (lost.has_capi_scope, lost.capi_scope_checked_at),
        (false, Some(100))
    );
    let won = write(&db, move |c| {
        iw::update_capi_scope_cache(c, aid, aws, true, None, Some(100))
    })
    .await
    .unwrap();
    assert_eq!(
        (won.has_capi_scope, won.capi_scope_checked_at),
        (true, None)
    );
    assert!(
        write(&db, move |c| iw::update_capi_scope_cache(
            c, aid, bws, true, None, None
        ))
        .await
        .is_none()
    );
    // Dataset: the if-null write keeps an existing id; the plain write overwrites it.
    let ds = |row: Option<chatbotx_db::repo::models::IntegrationWhatsapp>| {
        row.and_then(|r| r.dataset_id)
    };
    assert_eq!(
        ds(write(&db, move |c| iw::update_dataset_id_if_null(
            c, aid, aws, "ds-1"
        ))
        .await),
        Some("ds-1".to_owned())
    );
    assert!(
        write(&db, move |c| iw::update_dataset_id_if_null(
            c, aid, aws, "ds-2"
        ))
        .await
        .is_none()
    );
    assert_eq!(
        ds(write(&db, move |c| iw::update_dataset_id(c, aid, aws, "ds-3")).await),
        Some("ds-3".to_owned())
    );
    assert!(
        write(&db, move |c| iw::update_dataset_id(c, aid, bws, "ds-4"))
            .await
            .is_none()
    );
    let code = write(&db, move |c| {
        iw::update_capi_test_event_code(c, aid, aws, Some("TEST1"))
    })
    .await;
    assert_eq!(
        code.and_then(|r| r.capi_test_event_code),
        Some("TEST1".to_owned())
    );
    let code = write(&db, move |c| {
        iw::update_capi_test_event_code(c, aid, aws, None)
    })
    .await;
    assert_eq!(code.map(|r| r.capi_test_event_code), Some(None));
    let token = Json(serde_json::json!({ "iv": "a", "data": "b" }));
    let t2 = token.clone();
    let row = write(&db, move |c| iw::update_capi_access_token(c, aid, aws, &t2))
        .await
        .unwrap();
    assert_eq!(row.capi_access_token, Some(token.clone()));
    let row = write(&db, move |c| iw::set_capi_disconnected_at(c, aid, aws, 900))
        .await
        .unwrap();
    assert_eq!(
        (row.capi_disconnected_at, row.capi_access_token),
        (Some(900), None)
    );
    let t2 = token.clone();
    let row = write(&db, move |c| {
        iw::connect_custom_capi(c, aid, aws, "ds-c", &t2)
    })
    .await
    .unwrap();
    assert_eq!(
        (
            row.dataset_id.as_deref(),
            row.capi_access_token,
            row.capi_disconnected_at
        ),
        (Some("ds-c"), Some(token), None)
    );
    write(&db, move |c| iw::set_capi_disconnected_at(c, aid, aws, 901)).await;
    let row = write(&db, move |c| iw::clear_capi_disconnected_at(c, aid, aws))
        .await
        .unwrap();
    assert_eq!(row.capi_disconnected_at, None);
    let t3 = Json(serde_json::json!({ "iv": "c" }));
    write(&db, move |c| iw::update_capi_access_token(c, aid, aws, &t3)).await;
    let row = write(&db, move |c| iw::clear_capi_access_token(c, aid, aws))
        .await
        .unwrap();
    assert_eq!(row.capi_access_token, None);
    for f in [iw::clear_capi_disconnected_at, iw::clear_capi_access_token] {
        assert!(write(&db, move |c| f(c, aid, bws)).await.is_none());
    }
}

#[tokio::test]
async fn integration_whatsapp_call_settings_and_verification_slot() {
    use chatbotx_db::repo::integration_whatsapp as iw;
    use chatbotx_db::rusqlite::types::Value;
    use repo11::{read, write};
    let (db, a, b) = two_numbers().await;
    let (aid, aws, ainbox, bws) = (a.integration_id, a.workspace_id, a.inbox_id, b.workspace_id);
    // Verification slot: one claim at a time; a release only clears its own claim.
    assert_eq!(
        write(&db, move |c| iw::claim_verification_code_slot(
            c, aid, aws, 1_000, 500
        ))
        .await,
        Some(1_000)
    );
    assert_eq!(
        write(&db, move |c| iw::claim_verification_code_slot(
            c, aid, aws, 1_100, 900
        ))
        .await,
        None
    );
    assert_eq!(
        write(&db, move |c| iw::claim_verification_code_slot(
            c, aid, bws, 1_100, 2_000
        ))
        .await,
        None
    );
    write(&db, move |c| {
        iw::release_verification_code_slot(c, aid, aws, 999)
    })
    .await;
    assert_eq!(
        read(&db, move |c| iw::find_verification_code_requested_at(
            c, aid, aws
        ))
        .await,
        Some(Some(1_000))
    );
    assert_eq!(
        write(&db, move |c| iw::claim_verification_code_slot(
            c, aid, aws, 2_000, 1_500
        ))
        .await,
        Some(2_000)
    );
    write(&db, move |c| {
        iw::release_verification_code_slot(c, aid, aws, 2_000)
    })
    .await;
    assert_eq!(
        read(&db, move |c| iw::find_verification_code_requested_at(
            c, aid, aws
        ))
        .await,
        Some(None)
    );
    // Call recording, Pix redirect and the call settings patch.
    write(&db, move |c| {
        iw::update_call_recording_enabled(c, aid, bws, true)
    })
    .await;
    assert!(
        !read(&db, move |c| iw::is_call_recording_enabled_for_inbox(
            c, aws, ainbox
        ))
        .await
    );
    let transcribe = || vec![("callTranscriptionEnabled", Value::Integer(1))];
    assert!(
        write(&db, move |c| iw::update_call_settings(
            c,
            aid,
            aws,
            &transcribe(),
            true
        ))
        .await
        .is_none()
    );
    write(&db, move |c| {
        iw::update_call_recording_enabled(c, aid, aws, true)
    })
    .await;
    assert!(
        read(&db, move |c| iw::is_call_recording_enabled_for_inbox(
            c, aws, ainbox
        ))
        .await
    );
    let row = write(&db, move |c| {
        iw::update_call_settings(c, aid, aws, &transcribe(), true)
    })
    .await
    .unwrap();
    assert!(row.call_transcription_enabled);
    let hours = serde_json::json!({ "timezone": "UTC", "weeklyHours": [] });
    let values = vec![
        ("callRecordingRetentionDays", Value::Integer(30)),
        ("callingEnabled", Value::Integer(0)),
        ("inboundCallsEnabled", Value::Integer(1)),
        ("callHours", Value::Text(hours.to_string())),
        ("callRecordingEnabled", Value::Integer(0)),
    ];
    let row = write(&db, move |c| {
        iw::update_call_settings(c, aid, aws, &values, false)
    })
    .await
    .unwrap();
    assert_eq!(
        (
            row.call_recording_retention_days,
            row.calling_enabled,
            row.inbound_calls_enabled,
            row.call_recording_enabled
        ),
        (30, Some(false), true, false)
    );
    assert_eq!(row.call_hours.map(|h| h.0), Some(hours));
    let refused = vec![("pixRedirectEnabled", Value::Integer(1))];
    assert!(
        chatbotx_db::repo::write(&db, move |c| iw::update_call_settings(
            c, aid, aws, &refused, false
        ))
        .await
        .is_err()
    );
    let row = write(&db, move |c| {
        iw::update_pix_redirect_enabled(c, aid, aws, true)
    })
    .await
    .unwrap();
    assert!(row.pix_redirect_enabled);
    assert!(
        write(&db, move |c| iw::update_pix_redirect_enabled(
            c, aid, bws, false
        ))
        .await
        .is_none()
    );
}

#[tokio::test]
async fn contact_inbox_bulk_ai_re_check_and_count() {
    use chatbotx_db::repo::contact_inbox as ci;
    use chatbotx_db::repo::queries::ai_handover_bulk_eligibility::AiHandoverBulkAction;
    use repo11::read;
    use repo12::{Thread, hours_ago, input, input_for, put_bulk_thread, put_conversation, thread};
    let db = common::fresh_db().await;
    let s = seed(&db).await;
    let (ws, inbox) = (s.workspace_id, s.inbox_id);
    let held = Thread {
        state: Some("standby"),
        role: Some("ai_agent"),
        updated_at: Some(hours_ago(2)),
        ..thread()
    };
    // 9101, 9102: held by the AI agent; 9103: owned; 9104: held but without a DM.
    for (ci_id, contact_id, t) in [
        (9101, 8101, held),
        (9102, 8102, held),
        (
            9103,
            8103,
            Thread {
                state: Some("owned"),
                ..thread()
            },
        ),
        (9104, 8104, held),
    ] {
        put_bulk_thread(&db, ci_id, contact_id, ws, inbox, t).await;
    }
    for (conv, contact) in [(7101, 8101), (7102, 8102), (7103, 8103)] {
        put_conversation(&db, conv, ws, contact, None, None, None).await;
    }
    let ids = [9101, 9102, 9103, 9104, 9999];
    let disable = input();
    let enable = input_for(AiHandoverBulkAction::Enable);
    assert_eq!(
        read(&db, move |c| ci::list_still_bulk_ai_eligible(
            c, &disable, ws, &ids
        ))
        .await,
        vec![9101, 9102]
    );
    assert_eq!(
        read(&db, move |c| ci::list_still_bulk_ai_eligible(
            c, &enable, ws, &ids
        ))
        .await,
        vec![9103]
    );
    assert!(
        read(&db, move |c| ci::list_still_bulk_ai_eligible(
            c,
            &disable,
            ws + 1,
            &ids
        ))
        .await
        .is_empty()
    );
    assert_eq!(
        read(&db, move |c| ci::count_bulk_ai_eligible(
            c, &disable, ws, inbox, None
        ))
        .await,
        2
    );
    assert_eq!(
        read(&db, move |c| ci::count_bulk_ai_eligible(
            c,
            &disable,
            ws,
            inbox,
            Some(9101)
        ))
        .await,
        1
    );
    assert_eq!(
        read(&db, move |c| ci::count_bulk_ai_eligible(
            c, &enable, ws, inbox, None
        ))
        .await,
        1
    );
    assert_eq!(
        read(&db, move |c| ci::count_bulk_ai_eligible(
            c,
            &disable,
            ws,
            inbox + 1,
            None
        ))
        .await,
        0
    );
}
