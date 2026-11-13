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
