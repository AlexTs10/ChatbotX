//! Port of `packages/database/__tests__/tag-channel-repository.test.ts` (P3.3). The
//! mocked insert/select/update/delete chains become real `Tag`, `TagChannel`,
//! `ContactToTag` and `ContactToTagChannel` rows: asserted conflict targets and `where`
//! arguments are checked as rows inserted once, found, changed or left alone.

mod common;
#[macro_use]
#[path = "common/repo.rs"]
mod repo_common;

use chatbotx_db::repo::tag_channel::{
    LabelMappingInput, TagChannelKey, delete_by_id, delete_contact_tags_for_contacts,
    delete_links_for_channel, find_by_tag_and_integration, insert_if_absent, insert_or_fetch,
    link_contact_inbox, list_by_tag, list_contact_inbox_ids_for_channel_page,
    list_contact_tag_channel_rows, list_tagged_contact_ids_page, unlink_contact_inbox,
    update_external_label_id, upsert_by_tag_and_integration, upsert_label_mapping,
};
use chatbotx_db::repo::{self};
use common::{fresh_db, insert_contact_inbox, seed, seed_workspace};
use repo_common::{int, ok, rows, scalar_i64, setup};

const WS: i64 = 2001;
const OTHER: i64 = 2002;
const TAG: i64 = 1;

/// Tag 1 "VIP" of 2001; contact 4001 with contact inboxes 4101, 4102; contacts 4002, 4003.
async fn seeded() -> common::TestDb {
    let db = fresh_db().await;
    let s = seed(&db).await;
    seed_workspace(&db, 1009, OTHER).await;
    setup(&db, move |tx| {
        ins!(tx, "Tag", { "id" => TAG, "name" => "VIP", "workspaceId" => WS })?;
        insert_contact_inbox(tx, s, 4101, "psid-1", None, None)?;
        insert_contact_inbox(tx, s, 4102, "psid-2", None, None)?;
        ins!(tx, "Contact", { "id" => 4002_i64, "workspaceId" => WS })?;
        ins!(tx, "Contact", { "id" => 4003_i64, "workspaceId" => WS })?;
        Ok(())
    })
    .await;
    db
}

fn key(tag_id: i64, integration_id: i64, label: &str) -> TagChannelKey {
    TagChannelKey {
        workspace_id: WS,
        tag_id,
        channel_type: "whatsapp".to_owned(),
        integration_id,
        external_label_id: label.to_owned(),
    }
}

async fn channel_count(db: &common::TestDb) -> Option<i64> {
    scalar_i64(db, "SELECT count(*) FROM \"TagChannel\"", vec![]).await
}

async fn add_channel(db: &common::TestDb, k: TagChannelKey) -> i64 {
    ok(repo::write(db, move |tx| insert_or_fetch(tx, &k)).await)
        .expect("tag channel")
        .id
}

/// Port of: packages/database/__tests__/tag-channel-repository.test.ts:125 "upsertLabelMapping > links the contact-inbox to the tag and tagChannel when both upserts succeed"
#[tokio::test]
async fn upsert_label_mapping_links_the_contact_inbox_to_the_tag_and_tag_channel_when_both_upserts_succeed()
 {
    let db = seeded().await;
    let input = LabelMappingInput {
        workspace_id: WS,
        channel_type: "whatsapp".to_owned(),
        integration_id: 77,
        external_label_id: "label-1".to_owned(),
        label_name: "Hot lead".to_owned(),
        contact_inbox_id: 4101,
        contact_id: 4001,
    };
    let again = input.clone();
    ok(repo::write(&db, move |tx| upsert_label_mapping(tx, &input)).await);
    // A repeat links nothing twice (onConflictDoNothing / DO UPDATE).
    ok(repo::write(&db, move |tx| upsert_label_mapping(tx, &again)).await);
    let tag_id = scalar_i64(
        &db,
        "SELECT \"id\" FROM \"Tag\" WHERE \"name\" = 'Hot lead'",
        vec![],
    )
    .await
    .expect("tag");
    assert_eq!(channel_count(&db).await, Some(1));
    let channel = ok(repo::read(&db, move |c| {
        find_by_tag_and_integration(c, WS, tag_id, "whatsapp", 77)
    })
    .await)
    .expect("channel");
    assert_eq!(channel.external_label_id, "label-1");
    assert_eq!(
        rows(
            &db,
            "SELECT \"contactId\", \"tagId\" FROM \"ContactToTag\"",
            vec![]
        )
        .await,
        vec![vec![int(4001), int(tag_id)]]
    );
    assert_eq!(
        rows(
            &db,
            "SELECT \"tagId\", \"tagChannelId\", \"contactInboxId\" FROM \"ContactToTagChannel\"",
            vec![]
        )
        .await,
        vec![vec![int(tag_id), int(channel.id), int(4101)]]
    );
}

/// Port of: packages/database/__tests__/tag-channel-repository.test.ts:213 "insertIfAbsent > targets the (tagId, channelType, integrationId) conflict key"
#[tokio::test]
async fn insert_if_absent_targets_the_tag_id_channel_type_integration_id_conflict_key() {
    let db = seeded().await;
    ok(repo::write(&db, |tx| insert_if_absent(tx, &key(TAG, 77, "label-1"))).await);
    // Same key, another label id: no second row and the first label stays.
    ok(repo::write(&db, |tx| insert_if_absent(tx, &key(TAG, 77, "label-2"))).await);
    assert_eq!(channel_count(&db).await, Some(1));
    let row = ok(repo::read(&db, |c| {
        find_by_tag_and_integration(c, WS, TAG, "whatsapp", 77)
    })
    .await)
    .expect("row");
    assert_eq!(row.external_label_id, "label-1");
}

/// Port of: packages/database/__tests__/tag-channel-repository.test.ts:232 "insertOrFetch > returns the inserted row when the insert wins"
#[tokio::test]
async fn insert_or_fetch_returns_the_inserted_row_when_the_insert_wins() {
    let db = seeded().await;
    let row = ok(repo::write(&db, |tx| insert_or_fetch(tx, &key(TAG, 77, "label-1"))).await)
        .expect("row");
    assert_eq!(
        (
            row.tag_id,
            row.integration_id,
            row.external_label_id.as_str()
        ),
        (TAG, 77, "label-1")
    );
}

/// Port of: packages/database/__tests__/tag-channel-repository.test.ts:247 "insertOrFetch > falls back to a refetch when the insert conflicts"
#[tokio::test]
async fn insert_or_fetch_falls_back_to_a_refetch_when_the_insert_conflicts() {
    let db = seeded().await;
    let first = add_channel(&db, key(TAG, 77, "label-1")).await;
    let again = ok(repo::write(&db, |tx| insert_or_fetch(tx, &key(TAG, 77, "label-2"))).await)
        .expect("row");
    assert_eq!(again.id, first);
    assert_eq!(again.external_label_id, "label-1");
}

/// Port of: packages/database/__tests__/tag-channel-repository.test.ts:265 "deleteLinksForChannel / deleteContactTagsForContacts > no-ops without querying when the id list is empty"
#[tokio::test]
async fn delete_links_for_channel_delete_contact_tags_for_contacts_no_ops_without_querying_when_the_id_list_is_empty()
 {
    let db = seeded().await;
    let ch = add_channel(&db, key(TAG, 77, "label-1")).await;
    ok(repo::write(&db, move |tx| link_contact_inbox(tx, TAG, ch, 4101)).await);
    setup(
        &db,
        |tx| ins!(tx, "ContactToTag", { "contactId" => 4001_i64, "tagId" => TAG }),
    )
    .await;
    ok(repo::write(&db, move |tx| delete_links_for_channel(tx, ch, &[])).await);
    ok(repo::write(&db, |tx| delete_contact_tags_for_contacts(tx, TAG, &[])).await);
    assert_eq!(
        scalar_i64(&db, "SELECT count(*) FROM \"ContactToTagChannel\"", vec![]).await,
        Some(1)
    );
    assert_eq!(
        scalar_i64(&db, "SELECT count(*) FROM \"ContactToTag\"", vec![]).await,
        Some(1)
    );
}

/// Port of: packages/database/__tests__/tag-channel-repository.test.ts:278 "deleteLinksForChannel / deleteContactTagsForContacts > deletes scoped to the given ids when non-empty"
#[tokio::test]
async fn delete_links_for_channel_delete_contact_tags_for_contacts_deletes_scoped_to_the_given_ids_when_non_empty()
 {
    let db = seeded().await;
    let ch = add_channel(&db, key(TAG, 77, "label-1")).await;
    ok(repo::write(&db, move |tx| link_contact_inbox(tx, TAG, ch, 4101)).await);
    ok(repo::write(&db, move |tx| link_contact_inbox(tx, TAG, ch, 4102)).await);
    setup(&db, |tx| {
        ins!(tx, "ContactToTag", { "contactId" => 4001_i64, "tagId" => TAG })?;
        ins!(tx, "ContactToTag", { "contactId" => 4002_i64, "tagId" => TAG })
    })
    .await;
    ok(repo::write(&db, move |tx| delete_links_for_channel(tx, ch, &[4101])).await);
    ok(repo::write(&db, |tx| delete_contact_tags_for_contacts(tx, TAG, &[4001])).await);
    assert_eq!(
        rows(
            &db,
            "SELECT \"contactInboxId\" FROM \"ContactToTagChannel\"",
            vec![]
        )
        .await,
        vec![vec![int(4102)]]
    );
    assert_eq!(
        rows(&db, "SELECT \"contactId\" FROM \"ContactToTag\"", vec![]).await,
        vec![vec![int(4002)]]
    );
}

/// Port of: packages/database/__tests__/tag-channel-repository.test.ts:296 "listContactInboxIdsForChannelPage > pages by contactInboxId ascending, keyed off tagChannelId"
#[tokio::test]
async fn list_contact_inbox_ids_for_channel_page_pages_by_contact_inbox_id_ascending_keyed_off_tag_channel_id()
 {
    let db = seeded().await;
    let ch = add_channel(&db, key(TAG, 77, "label-1")).await;
    let other = add_channel(&db, key(TAG, 78, "label-x")).await;
    ok(repo::write(&db, move |tx| {
        link_contact_inbox(tx, TAG, ch, 4102)?;
        link_contact_inbox(tx, TAG, ch, 4101)?;
        link_contact_inbox(tx, TAG, other, 4101)
    })
    .await);
    assert_eq!(
        ok(
            repo::read(&db, move |c| list_contact_inbox_ids_for_channel_page(
                c, ch, None, 10
            ))
            .await
        ),
        vec![4101, 4102]
    );
    assert_eq!(
        ok(
            repo::read(&db, move |c| list_contact_inbox_ids_for_channel_page(
                c, ch, None, 1
            ))
            .await
        ),
        vec![4101]
    );
}

/// Port of: packages/database/__tests__/tag-channel-repository.test.ts:312 "listContactInboxIdsForChannelPage > adds the afterContactInboxId gt-filter when a cursor is passed"
#[tokio::test]
async fn list_contact_inbox_ids_for_channel_page_adds_the_after_contact_inbox_id_gt_filter_when_a_cursor_is_passed()
 {
    let db = seeded().await;
    let ch = add_channel(&db, key(TAG, 77, "label-1")).await;
    ok(repo::write(&db, move |tx| {
        link_contact_inbox(tx, TAG, ch, 4101)?;
        link_contact_inbox(tx, TAG, ch, 4102)
    })
    .await);
    assert_eq!(
        ok(
            repo::read(&db, move |c| list_contact_inbox_ids_for_channel_page(
                c,
                ch,
                Some(4101),
                10
            ))
            .await
        ),
        vec![4102]
    );
}

/// Port of: packages/database/__tests__/tag-channel-repository.test.ts:334 "deleteById > deletes the tagChannel by its own id"
#[tokio::test]
async fn delete_by_id_deletes_the_tag_channel_by_its_own_id() {
    let db = seeded().await;
    let a = add_channel(&db, key(TAG, 77, "label-1")).await;
    add_channel(&db, key(TAG, 78, "label-2")).await;
    ok(repo::write(&db, move |tx| delete_by_id(tx, a)).await);
    assert_eq!(channel_count(&db).await, Some(1));
    assert_eq!(
        ok(repo::read(&db, |c| find_by_tag_and_integration(
            c, WS, TAG, "whatsapp", 77
        ))
        .await),
        None
    );
}

/// Port of: packages/database/__tests__/tag-channel-repository.test.ts:346 "findByTagAndIntegration > scopes the lookup by tagId, workspaceId, channelType, and integrationId"
#[tokio::test]
async fn find_by_tag_and_integration_scopes_the_lookup_by_tag_id_workspace_id_channel_type_and_integration_id()
 {
    let db = seeded().await;
    let a = add_channel(&db, key(TAG, 77, "label-1")).await;
    assert_eq!(
        ok(repo::read(&db, |c| find_by_tag_and_integration(
            c, WS, TAG, "whatsapp", 77
        ))
        .await)
        .map(|r| r.id),
        Some(a)
    );
    assert_eq!(
        ok(repo::read(&db, |c| find_by_tag_and_integration(
            c, OTHER, TAG, "whatsapp", 77
        ))
        .await),
        None
    );
    assert_eq!(
        ok(repo::read(&db, |c| find_by_tag_and_integration(
            c,
            WS,
            TAG,
            "messenger",
            77
        ))
        .await),
        None
    );
    assert_eq!(
        ok(repo::read(&db, |c| find_by_tag_and_integration(
            c, WS, TAG, "whatsapp", 78
        ))
        .await),
        None
    );
}

/// Port of: packages/database/__tests__/tag-channel-repository.test.ts:369 "updateExternalLabelId > updates only the row matching the given id"
#[tokio::test]
async fn update_external_label_id_updates_only_the_row_matching_the_given_id() {
    let db = seeded().await;
    let a = add_channel(&db, key(TAG, 77, "label-1")).await;
    add_channel(&db, key(TAG, 78, "label-2")).await;
    ok(repo::write(&db, move |tx| update_external_label_id(tx, a, "label-9")).await);
    let labels = rows(
        &db,
        "SELECT \"externalLabelId\" FROM \"TagChannel\" ORDER BY \"integrationId\"",
        vec![],
    )
    .await;
    assert_eq!(
        labels,
        vec![
            vec![repo_common::text("label-9")],
            vec![repo_common::text("label-2")]
        ]
    );
}

/// Port of: packages/database/__tests__/tag-channel-repository.test.ts:385 "upsertByTagAndIntegration > targets the (tagId, channelType, integrationId) conflict key and returns the row"
#[tokio::test]
async fn upsert_by_tag_and_integration_targets_the_tag_id_channel_type_integration_id_conflict_key_and_returns_the_row()
 {
    let db = seeded().await;
    let first = ok(repo::write(&db, |tx| {
        upsert_by_tag_and_integration(tx, &key(TAG, 77, "label-1"))
    })
    .await)
    .expect("row");
    let second = ok(repo::write(&db, |tx| {
        upsert_by_tag_and_integration(tx, &key(TAG, 77, "label-2"))
    })
    .await)
    .expect("row");
    assert_eq!(second.id, first.id);
    assert_eq!(second.external_label_id, "label-2");
    assert_eq!(channel_count(&db).await, Some(1));
}

/// Port of: packages/database/__tests__/tag-channel-repository.test.ts:406 "listByTag > scopes by tagId and workspaceId with no optional filters"
#[tokio::test]
async fn list_by_tag_scopes_by_tag_id_and_workspace_id_with_no_optional_filters() {
    let db = seeded().await;
    add_channel(&db, key(TAG, 77, "label-1")).await;
    add_channel(
        &db,
        TagChannelKey {
            channel_type: "messenger".to_owned(),
            ..key(TAG, 78, "label-2")
        },
    )
    .await;
    let mut ids: Vec<i64> = ok(repo::read(&db, |c| list_by_tag(c, WS, TAG, None, None)).await)
        .iter()
        .map(|r| r.integration_id)
        .collect();
    ids.sort_unstable();
    assert_eq!(ids, vec![77, 78]);
    assert!(ok(repo::read(&db, |c| list_by_tag(c, OTHER, TAG, None, None)).await).is_empty());
}

/// Port of: packages/database/__tests__/tag-channel-repository.test.ts:425 "listByTag > adds channelType/integrationId filters only when provided"
#[tokio::test]
async fn list_by_tag_adds_channel_type_integration_id_filters_only_when_provided() {
    let db = seeded().await;
    add_channel(&db, key(TAG, 77, "label-1")).await;
    add_channel(&db, key(TAG, 79, "label-3")).await;
    add_channel(
        &db,
        TagChannelKey {
            channel_type: "messenger".to_owned(),
            ..key(TAG, 78, "label-2")
        },
    )
    .await;
    let by_type = ok(repo::read(&db, |c| list_by_tag(c, WS, TAG, Some("whatsapp"), None)).await);
    assert_eq!(by_type.len(), 2);
    let by_both =
        ok(repo::read(&db, |c| list_by_tag(c, WS, TAG, Some("whatsapp"), Some(79))).await);
    assert_eq!(
        by_both
            .iter()
            .map(|r| r.external_label_id.as_str())
            .collect::<Vec<_>>(),
        vec!["label-3"]
    );
}

/// Port of: packages/database/__tests__/tag-channel-repository.test.ts:453 "linkContactInbox > inserts the ContactToTagChannel row with onConflictDoNothing"
#[tokio::test]
async fn link_contact_inbox_inserts_the_contact_to_tag_channel_row_with_on_conflict_do_nothing() {
    let db = seeded().await;
    let ch = add_channel(&db, key(TAG, 77, "label-1")).await;
    ok(repo::write(&db, move |tx| link_contact_inbox(tx, TAG, ch, 4101)).await);
    ok(repo::write(&db, move |tx| link_contact_inbox(tx, TAG, ch, 4101)).await);
    assert_eq!(
        rows(
            &db,
            "SELECT \"tagId\", \"tagChannelId\", \"contactInboxId\" FROM \"ContactToTagChannel\"",
            vec![]
        )
        .await,
        vec![vec![int(TAG), int(ch), int(4101)]]
    );
}

/// Port of: packages/database/__tests__/tag-channel-repository.test.ts:474 "unlinkContactInbox > deletes scoped by tagChannelId AND contactInboxId"
#[tokio::test]
async fn unlink_contact_inbox_deletes_scoped_by_tag_channel_id_and_contact_inbox_id() {
    let db = seeded().await;
    let ch = add_channel(&db, key(TAG, 77, "label-1")).await;
    let other = add_channel(&db, key(TAG, 78, "label-2")).await;
    ok(repo::write(&db, move |tx| {
        link_contact_inbox(tx, TAG, ch, 4101)?;
        link_contact_inbox(tx, TAG, ch, 4102)?;
        link_contact_inbox(tx, TAG, other, 4101)
    })
    .await);
    ok(repo::write(&db, move |tx| unlink_contact_inbox(tx, ch, 4101)).await);
    assert_eq!(
        rows(&db, "SELECT \"tagChannelId\", \"contactInboxId\" FROM \"ContactToTagChannel\" ORDER BY 1, 2", vec![]).await,
        vec![vec![int(ch), int(4102)], vec![int(other), int(4101)]]
    );
}

/// Port of: packages/database/__tests__/tag-channel-repository.test.ts:494 "listContactTagChannelRows > inner-joins TagChannel + ContactInbox, scoped by tagId and contactId"
#[tokio::test]
async fn list_contact_tag_channel_rows_inner_joins_tag_channel_contact_inbox_scoped_by_tag_id_and_contact_id()
 {
    let db = seeded().await;
    let ch = add_channel(&db, key(TAG, 77, "label-1")).await;
    setup(&db, |tx| {
        ins!(tx, "Tag", { "id" => 2_i64, "name" => "Other", "workspaceId" => WS })?;
        ins!(tx, "ContactInbox", { "id" => 4201_i64, "originalContactId" => 4002_i64, "contactId" => 4002_i64, "inboxId" => 3001_i64, "channel" => "messenger", "source" => "messenger", "sourceId" => "psid-3" })
    })
    .await;
    let other_tag = add_channel(&db, key(2, 77, "label-o")).await;
    ok(repo::write(&db, move |tx| {
        link_contact_inbox(tx, TAG, ch, 4101)?;
        link_contact_inbox(tx, TAG, ch, 4201)?;
        link_contact_inbox(tx, 2, other_tag, 4102)
    })
    .await);
    let rows = ok(repo::read(&db, |c| list_contact_tag_channel_rows(c, TAG, 4001)).await);
    assert_eq!(rows.len(), 1);
    let r = &rows[0];
    assert_eq!(
        (
            r.tag_channel_id,
            r.contact_inbox_id,
            r.channel_type.as_str(),
            r.integration_id,
            r.external_label_id.as_str(),
            r.source_id.as_str()
        ),
        (ch, 4101, "whatsapp", 77, "label-1", "psid-1")
    );
}

/// Port of: packages/database/__tests__/tag-channel-repository.test.ts:529 "listTaggedContactIdsPage > pages by contactId ascending, keyed off tagId"
#[tokio::test]
async fn list_tagged_contact_ids_page_pages_by_contact_id_ascending_keyed_off_tag_id() {
    let db = seeded().await;
    setup(&db, |tx| {
        ins!(tx, "Tag", { "id" => 2_i64, "name" => "Other", "workspaceId" => WS })?;
        ins!(tx, "ContactToTag", { "contactId" => 4003_i64, "tagId" => TAG })?;
        ins!(tx, "ContactToTag", { "contactId" => 4001_i64, "tagId" => TAG })?;
        ins!(tx, "ContactToTag", { "contactId" => 4002_i64, "tagId" => 2_i64 })
    })
    .await;
    assert_eq!(
        ok(repo::read(&db, |c| list_tagged_contact_ids_page(c, TAG, None, 10)).await),
        vec![4001, 4003]
    );
    assert_eq!(
        ok(repo::read(&db, |c| list_tagged_contact_ids_page(c, TAG, None, 1)).await),
        vec![4001]
    );
}

/// Port of: packages/database/__tests__/tag-channel-repository.test.ts:546 "listTaggedContactIdsPage > adds the afterContactId gt-filter when a cursor is passed"
#[tokio::test]
async fn list_tagged_contact_ids_page_adds_the_after_contact_id_gt_filter_when_a_cursor_is_passed()
{
    let db = seeded().await;
    setup(&db, |tx| {
        ins!(tx, "ContactToTag", { "contactId" => 4003_i64, "tagId" => TAG })?;
        ins!(tx, "ContactToTag", { "contactId" => 4001_i64, "tagId" => TAG })
    })
    .await;
    assert_eq!(
        ok(repo::read(&db, |c| list_tagged_contact_ids_page(
            c,
            TAG,
            Some(4001),
            10
        ))
        .await),
        vec![4003]
    );
}
