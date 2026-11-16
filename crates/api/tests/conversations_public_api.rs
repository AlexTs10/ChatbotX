//! Port of `apps/builder/__tests__/conversations-public-api.test.ts`,
//! `list-conversations-query.test.ts`, `build-conversation-where-sla.test.ts`
//! and `src/features/conversations/queries/__tests__/list-conversations.query.test.ts`
//! (P3.6-D3 slice conversations). Adapted: the captured oRPC handlers, the
//! mocked `conversationService` / `findConversation` / message repository and
//! the `buildConversationWhere` object shapes become `handlers::dispatch`
//! calls over real rows; "service X was called with Y" becomes the changed
//! row, and a where-clause shape becomes the rows the list returns.

mod handlers_common;

use chatbotx_api::routes::{PUBLIC_ROUTES, RouteAuth};
use handlers_common::{Fixture, OTHER_WS, WS, call, fixture, scalar};
use serde_json::{Value, json};

const HOUR: i64 = 60 * 60 * 1000;

fn now() -> i64 {
    chatbotx_db::repo::now_ms()
}

async fn exec(f: &Fixture, sql: String) {
    f.state
        .db
        .write(move |tx| {
            tx.execute_batch(&sql)?;
            Ok(())
        })
        .await
        .expect("seed");
}

/// Inboxes 5 telegram, 6 messenger, 7 instagram, 8 whatsapp.
async fn inboxes(f: &Fixture) {
    exec(
        f,
        format!(
            r#"INSERT INTO "Inbox" ("id", "workspaceId", "name", "channel", "sourceId") VALUES
               (5, {WS}, 'Tele', 'telegram', 't-5'), (6, {WS}, 'Mess', 'messenger', 'm-6'),
               (7, {WS}, 'Insta', 'instagram', 'i-7'), (8, {WS}, 'Wa', 'whatsapp', 'w-8');"#
        ),
    )
    .await;
}

/// Contact / contact inbox / DM conversation `id` (contact inbox on
/// `inbox`, `None` = no contact inbox). `contact` / `conv`: extra
/// `(column, SQL literal)` pairs.
async fn seed(
    f: &Fixture,
    id: i64,
    inbox: Option<(i64, &str)>,
    contact: &[(&str, &str)],
    conv: &[(&str, &str)],
) {
    let cols = |extra: &[(&str, &str)]| {
        extra
            .iter()
            .map(|(c, _)| format!(", \"{c}\""))
            .collect::<String>()
    };
    let vals = |extra: &[(&str, &str)]| {
        extra
            .iter()
            .map(|(_, v)| format!(", {v}"))
            .collect::<String>()
    };
    let mut contact: Vec<(&str, &str)> = contact.to_vec();
    let first = format!("'F{id}'");
    if !contact.iter().any(|(c, _)| *c == "firstName") {
        contact.push(("firstName", &first));
    }
    let mut sql = format!(
        r#"INSERT INTO "Contact" ("id", "workspaceId", "lastName"{}) VALUES ({id}, {WS}, 'Filter'{});"#,
        cols(&contact),
        vals(&contact)
    );
    if let Some((inbox, channel)) = inbox {
        sql.push_str(&format!(
            r#"INSERT INTO "ContactInbox" ("id", "originalContactId", "contactId", "inboxId", "channel", "source", "sourceId")
               VALUES ({id}, {id}, {id}, {inbox}, '{channel}', 'direct', 'src-{id}');"#
        ));
    }
    sql.push_str(&format!(
        r#"INSERT INTO "Conversation" ("id", "workspaceId", "contactId"{}) VALUES ({id}, {WS}, {id}{});"#,
        cols(conv),
        vals(conv)
    ));
    exec(f, sql).await;
}

async fn message(f: &Fixture, id: i64, conversation: i64, contact_inbox: i64, kind: &str, at: i64) {
    exec(
        f,
        format!(
            r#"INSERT INTO "Message" ("id", "createdAt", "updatedAt", "conversationId", "contactInboxId", "workspaceId", "text",
                 "messageType", "contentType", "senderType")
               VALUES ({id}, {at}, {at}, {conversation}, {contact_inbox}, {WS}, 'm{id}', '{kind}', 'text', 'api');"#
        ),
    )
    .await;
}

fn query(params: &[(&str, &str)]) -> String {
    url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs(params)
        .finish()
}

async fn list(f: &Fixture, params: &[(&str, &str)]) -> Value {
    let q = query(params);
    let (status, body) = call(f, "conversations.list", WS, &[], Some(&q), None).await;
    assert_eq!(status, 200, "{body}");
    body
}

async fn ids(f: &Fixture, params: &[(&str, &str)]) -> Vec<String> {
    list(f, params).await["data"]
        .as_array()
        .expect("data")
        .iter()
        .map(|c| c["id"].as_str().expect("id").to_owned())
        .collect()
}

fn s(ids: &[i64]) -> Vec<String> {
    ids.iter().map(i64::to_string).collect()
}

async fn action(f: &Fixture, route: &str, id: &str) -> (u16, Value) {
    call(f, route, WS, &[("id", id)], None, Some(json!({}))).await
}

async fn column(f: &Fixture, sql: &'static str) -> Option<Value> {
    scalar(f, sql, 10).await
}

// ─── conversations-public-api.test.ts ───────────────────────────────────

#[test]
fn registers_the_conversations_public_router_under_the_inbox_scope() {
    let routes: Vec<_> = PUBLIC_ROUTES
        .iter()
        .filter(|r| r.id.starts_with("conversations."))
        .collect();
    assert_eq!(routes.len(), 12);
    for route in routes {
        assert!(
            matches!(route.auth, RouteAuth::WorkspaceToken(scopes) if scopes == ["inbox"]),
            "{}",
            route.id
        );
    }
}

#[tokio::test]
async fn get_v1_conversations_id_delegates_to_findconversation() {
    let f = fixture().await;
    inboxes(&f).await;
    seed(&f, 10, Some((5, "telegram")), &[], &[]).await;
    let (status, body) = call(&f, "conversations.get", WS, &[("id", "10")], None, None).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["data"]["id"], json!("10"));
    assert_eq!(body["data"]["workspaceId"], json!(WS.to_string()));
    // The token's workspace scopes the lookup.
    let (status, _) = call(
        &f,
        "conversations.get",
        OTHER_WS,
        &[("id", "10")],
        None,
        None,
    )
    .await;
    assert_eq!(status, 404);
}

#[tokio::test]
async fn post_assign_resolves_the_conversation_and_delegates_to_assignone_without_an_actor() {
    let f = fixture().await;
    inboxes(&f).await;
    seed(&f, 10, Some((5, "telegram")), &[], &[]).await;
    exec(
        &f,
        format!(
            r#"INSERT INTO "WorkspaceMember" ("id", "workspaceId", "userId", "role") VALUES (1, {WS}, 1001, 'owner');"#
        ),
    )
    .await;
    let (status, body) = call(
        &f,
        "conversations.assign",
        WS,
        &[("id", "10")],
        None,
        Some(json!({ "assignedId": "u_1001" })),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body, json!({ "success": true }));
    assert_eq!(
        column(
            &f,
            r#"SELECT "assignedUserId" FROM "Conversation" WHERE "id" = ?1"#
        )
        .await,
        Some(json!(1001))
    );
    assert!(
        column(
            &f,
            r#"SELECT "assignedUserAt" IS NOT NULL FROM "Conversation" WHERE "id" = ?1"#
        )
        .await
            == Some(json!(1))
    );
}

#[tokio::test]
async fn post_archive_delegates_to_archivebyids_for_a_single_id_without_an_actor() {
    let f = fixture().await;
    inboxes(&f).await;
    seed(&f, 10, Some((5, "telegram")), &[], &[]).await;
    let (status, body) = action(&f, "conversations.archive", "10").await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body, json!({ "success": true }));
    assert_eq!(
        column(
            &f,
            r#"SELECT "archivedAt" IS NOT NULL FROM "Conversation" WHERE "id" = ?1"#
        )
        .await,
        Some(json!(1))
    );
}

#[tokio::test]
async fn post_unarchive_delegates_to_unarchivebyids_for_a_single_id() {
    let f = fixture().await;
    inboxes(&f).await;
    seed(
        &f,
        10,
        Some((5, "telegram")),
        &[],
        &[("archivedAt", "1000")],
    )
    .await;
    let (status, body) = action(&f, "conversations.unarchive", "10").await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body, json!({ "success": true }));
    assert_eq!(
        column(
            &f,
            r#"SELECT "archivedAt" FROM "Conversation" WHERE "id" = ?1"#
        )
        .await,
        Some(Value::Null)
    );
}

#[tokio::test]
async fn post_read_delegates_to_updatereadstatus() {
    let f = fixture().await;
    inboxes(&f).await;
    seed(&f, 10, Some((5, "telegram")), &[], &[]).await;
    let before = now();
    let (status, body) = action(&f, "conversations.read", "10").await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body, json!({ "success": true }));
    let read = column(
        &f,
        r#"SELECT "agentLastReadAt" FROM "Conversation" WHERE "id" = ?1"#,
    )
    .await
    .and_then(|v| v.as_i64())
    .expect("agentLastReadAt");
    assert!(read >= before);
}

#[tokio::test]
async fn post_unread_delegates_to_markunread() {
    let f = fixture().await;
    inboxes(&f).await;
    seed(
        &f,
        10,
        Some((5, "telegram")),
        &[],
        &[("agentLastReadAt", "5000")],
    )
    .await;
    let (status, body) = action(&f, "conversations.unread", "10").await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body, json!({ "success": true }));
    assert_eq!(
        column(
            &f,
            r#"SELECT "agentLastReadAt" FROM "Conversation" WHERE "id" = ?1"#
        )
        .await,
        Some(Value::Null)
    );
}

#[tokio::test]
async fn post_follow_delegates_to_setfollowed_without_an_actor() {
    let f = fixture().await;
    inboxes(&f).await;
    seed(&f, 10, Some((5, "telegram")), &[], &[]).await;
    let (status, body) = action(&f, "conversations.follow", "10").await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body, json!({ "success": true }));
    assert_eq!(
        column(
            &f,
            r#"SELECT "followed" FROM "Conversation" WHERE "id" = ?1"#
        )
        .await,
        Some(json!(1))
    );
}

#[tokio::test]
async fn post_unfollow_delegates_to_setfollowed() {
    let f = fixture().await;
    inboxes(&f).await;
    seed(&f, 10, Some((5, "telegram")), &[], &[("followed", "1")]).await;
    let (status, body) = action(&f, "conversations.unfollow", "10").await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body, json!({ "success": true }));
    assert_eq!(
        column(
            &f,
            r#"SELECT "followed" FROM "Conversation" WHERE "id" = ?1"#
        )
        .await,
        Some(json!(0))
    );
}

#[tokio::test]
async fn post_enable_bot_delegates_to_setbotenabledbyids_without_an_actor() {
    let f = fixture().await;
    inboxes(&f).await;
    seed(
        &f,
        10,
        Some((5, "telegram")),
        &[],
        &[("botEnabled", "0"), ("botResumeAt", "99999")],
    )
    .await;
    let (status, body) = action(&f, "conversations.enableBot", "10").await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body, json!({ "success": true }));
    assert_eq!(
        column(
            &f,
            r#"SELECT json_array("botEnabled", "botResumeAt") FROM "Conversation" WHERE "id" = ?1"#
        )
        .await,
        Some(json!("[1,null]"))
    );
}

#[tokio::test]
async fn post_disable_bot_delegates_to_setbotenabledbyids_without_an_actor() {
    let f = fixture().await;
    inboxes(&f).await;
    seed(&f, 10, Some((5, "telegram")), &[], &[]).await;
    let before = now();
    let (status, body) = action(&f, "conversations.disableBot", "10").await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body, json!({ "success": true }));
    assert_eq!(
        column(
            &f,
            r#"SELECT "botEnabled" FROM "Conversation" WHERE "id" = ?1"#
        )
        .await,
        Some(json!(0))
    );
    let resume = column(
        &f,
        r#"SELECT "botResumeAt" FROM "Conversation" WHERE "id" = ?1"#,
    )
    .await
    .and_then(|v| v.as_i64())
    .expect("botResumeAt");
    assert!(resume >= before + 24 * HOUR);
}

// ─── list-conversations-query.test.ts ───────────────────────────────────

/// Adapted: a comment-thread conversation and the DM share contact inbox 20
/// (its `lastMessageAt` is the newer DM time); each preview still shows its
/// own last message, so the window is anchored on each `lastActivityAt`.
#[tokio::test]
async fn anchors_each_conversation_s_sincetime_on_its_own_lastactivityat_not_a_shared_contactinbox_anchor()
 {
    let f = fixture().await;
    inboxes(&f).await;
    let older = now() - 160 * 24 * HOUR;
    let newer = now() - HOUR;
    seed(
        &f,
        20,
        Some((6, "messenger")),
        &[],
        &[("lastActivityAt", &newer.to_string())],
    )
    .await;
    exec(
        &f,
        format!(
            r#"UPDATE "ContactInbox" SET "lastMessageAt" = {newer} WHERE "id" = 20;
               INSERT INTO "Conversation" ("id", "workspaceId", "contactId", "sourceId", "lastActivityAt") VALUES (21, {WS}, 20, 'comment-1', {older});"#
        ),
    )
    .await;
    message(&f, 900, 21, 20, "incoming", older).await;
    message(&f, 901, 20, 20, "outgoing", newer).await;
    let body = list(&f, &[]).await;
    let data = body["data"].as_array().expect("data");
    assert_eq!(data.len(), 2);
    assert_eq!(data[0]["id"], json!("20"));
    assert_eq!(data[0]["messages"][0]["id"], json!("901"));
    assert_eq!(data[1]["id"], json!("21"));
    assert_eq!(data[1]["messages"][0]["id"], json!("900"));
}

#[tokio::test]
async fn findconversation_anchors_sincetime_on_the_conversation_s_own_lastactivityat() {
    let f = fixture().await;
    inboxes(&f).await;
    let at = now() - 200 * 24 * HOUR;
    seed(
        &f,
        10,
        Some((6, "messenger")),
        &[],
        &[("lastActivityAt", &at.to_string())],
    )
    .await;
    exec(
        &f,
        format!(
            r#"UPDATE "ContactInbox" SET "lastMessageAt" = {} WHERE "id" = 10;"#,
            now()
        ),
    )
    .await;
    message(&f, 900, 10, 10, "incoming", at).await;
    let (status, body) = call(&f, "conversations.get", WS, &[("id", "10")], None, None).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["data"]["messages"][0]["id"], json!("900"));
}

#[tokio::test]
async fn listconversations_requests_attachmentcountonly_instead_of_full_attachment_rows() {
    let f = fixture().await;
    inboxes(&f).await;
    let at = now() - HOUR;
    seed(
        &f,
        10,
        Some((5, "telegram")),
        &[],
        &[("lastActivityAt", &at.to_string())],
    )
    .await;
    message(&f, 900, 10, 10, "incoming", at).await;
    let body = list(&f, &[]).await;
    let preview = &body["data"][0]["messages"][0];
    assert_eq!(preview["attachmentCount"], json!(0));
    assert!(preview.get("attachments").is_none(), "{preview}");
}

#[tokio::test]
async fn findconversation_requests_attachmentcountonly_instead_of_full_attachment_rows() {
    let f = fixture().await;
    inboxes(&f).await;
    let at = now() - HOUR;
    seed(
        &f,
        10,
        Some((5, "telegram")),
        &[],
        &[("lastActivityAt", &at.to_string())],
    )
    .await;
    message(&f, 900, 10, 10, "incoming", at).await;
    let (_, body) = call(&f, "conversations.get", WS, &[("id", "10")], None, None).await;
    let preview = &body["data"]["messages"][0];
    assert_eq!(preview["attachmentCount"], json!(0));
    assert!(preview.get("attachments").is_none(), "{preview}");
}

#[tokio::test]
async fn propagates_attachmentcount_from_the_repository_into_the_response() {
    let f = fixture().await;
    inboxes(&f).await;
    let at = now() - HOUR;
    seed(
        &f,
        10,
        Some((5, "telegram")),
        &[],
        &[("lastActivityAt", &at.to_string())],
    )
    .await;
    message(&f, 900, 10, 10, "incoming", at).await;
    exec(
        &f,
        format!(
            r#"INSERT INTO "Attachment" ("id", "workspaceId", "conversationId", "messageId", "messageCreatedAt", "fileType", "mimeType", "originPath")
               VALUES (1, {WS}, 10, 900, {at}, 'image', 'image/png', 'a.png'), (2, {WS}, 10, 900, {at}, 'image', 'image/png', 'b.png');"#
        ),
    )
    .await;
    let body = list(&f, &[]).await;
    assert_eq!(body["data"][0]["messages"][0]["attachmentCount"], json!(2));
}

#[tokio::test]
async fn returns_a_proxy_avatar_url_for_a_contact_whose_newest_capable_inbox_is_pending() {
    let f = fixture().await;
    inboxes(&f).await;
    seed(&f, 10, Some((6, "messenger")), &[], &[]).await;
    exec(
        &f,
        r#"UPDATE "ContactInbox" SET "lastMessageAt" = 1000 WHERE "id" = 10;
           INSERT INTO "ContactInbox" ("id", "originalContactId", "contactId", "inboxId", "channel", "source", "sourceId", "lastMessageAt")
           VALUES (11, 10, 10, 7, 'instagram', 'direct', 'ig-10', 2000);"#
            .to_owned(),
    )
    .await;
    let body = list(&f, &[]).await;
    let avatar = body["data"][0]["contact"]["avatar"]
        .as_str()
        .expect("avatar");
    assert!(avatar.starts_with("http://app/media/avatar/"), "{avatar}");
}

#[tokio::test]
async fn keeps_a_mirrored_contact_avatar_s_raw_storage_key_unchanged() {
    let f = fixture().await;
    inboxes(&f).await;
    seed(
        &f,
        10,
        Some((6, "messenger")),
        &[("avatar", "'avatars/contact-1.png'")],
        &[],
    )
    .await;
    let body = list(&f, &[]).await;
    assert_eq!(
        body["data"][0]["contact"]["avatar"],
        json!("avatars/contact-1.png")
    );
}

/// Adapted: no `channel` filter (a contact without a contact inbox matches
/// no channel); the TS case mocks the list.
#[tokio::test]
async fn returns_null_for_a_whatsapp_contact_without_an_avatar_or_contact_inbox() {
    let f = fixture().await;
    inboxes(&f).await;
    seed(&f, 10, None, &[], &[]).await;
    let body = list(&f, &[]).await;
    assert_eq!(body["data"][0]["contact"]["avatar"], Value::Null);
}

// ─── build-conversation-where-sla.test.ts ───────────────────────────────

/// 10 breached, 11 breaching, 12 open window, 13 no window; 10 and 11 are
/// unread, 12 and 13 followed.
async fn sla_rows(f: &Fixture) {
    inboxes(f).await;
    let past = (now() - HOUR).to_string();
    let future = (now() + HOUR).to_string();
    let read = now().to_string();
    let activity = (now() - 2 * HOUR).to_string();
    let act = ("lastActivityAt", activity.as_str());
    seed(
        f,
        10,
        Some((5, "telegram")),
        &[],
        &[act, ("responseDueAt", &past)],
    )
    .await;
    seed(
        f,
        11,
        Some((5, "telegram")),
        &[],
        &[
            act,
            ("responseSlaBreachingAt", &past),
            ("responseDueAt", &future),
        ],
    )
    .await;
    seed(
        f,
        12,
        Some((5, "telegram")),
        &[],
        &[
            act,
            ("agentLastReadAt", &read),
            ("followed", "1"),
            ("responseDueAt", &future),
        ],
    )
    .await;
    seed(
        f,
        13,
        Some((5, "telegram")),
        &[],
        &[act, ("agentLastReadAt", &read), ("followed", "1")],
    )
    .await;
}

#[tokio::test]
async fn slabreached_deadline_already_passed() {
    let f = fixture().await;
    sla_rows(&f).await;
    assert_eq!(ids(&f, &[("tags", r#"["slaBreached"]"#)]).await, s(&[10]));
}

#[tokio::test]
async fn slabreaching_past_the_breaching_instant_but_not_yet_due() {
    let f = fixture().await;
    sla_rows(&f).await;
    assert_eq!(ids(&f, &[("tags", r#"["slaBreaching"]"#)]).await, s(&[11]));
}

#[tokio::test]
async fn both_bounds_use_the_same_instant_so_the_two_states_never_overlap() {
    let f = fixture().await;
    sla_rows(&f).await;
    assert_eq!(
        ids(&f, &[("tags", r#"["slaBreaching","slaBreached"]"#)]).await,
        Vec::<String>::new()
    );
}

#[tokio::test]
async fn composes_with_the_unread_predicate_instead_of_replacing_it() {
    let f = fixture().await;
    sla_rows(&f).await;
    assert_eq!(ids(&f, &[("tags", r#"["unread"]"#)]).await, s(&[11, 10]));
    assert_eq!(
        ids(&f, &[("tags", r#"["unread","slaBreached"]"#)]).await,
        s(&[10])
    );
}

#[tokio::test]
async fn no_sla_tag_adds_no_sla_clause() {
    let f = fixture().await;
    sla_rows(&f).await;
    assert_eq!(ids(&f, &[("tags", r#"["followUp"]"#)]).await, s(&[13, 12]));
}

// ─── list-conversations.query.test.ts (buildConversationWhere) ──────────

/// 10 telegram "Zeta" female, 11 messenger "Alpha" male, 12 telegram
/// blocked female, 13 messenger "Zeta" male with an email.
async fn filter_rows(f: &Fixture) {
    inboxes(f).await;
    seed(
        f,
        10,
        Some((5, "telegram")),
        &[("firstName", "'Zeta'"), ("gender", "'female'")],
        &[],
    )
    .await;
    seed(
        f,
        11,
        Some((6, "messenger")),
        &[("firstName", "'Alpha'"), ("gender", "'male'")],
        &[],
    )
    .await;
    seed(
        f,
        12,
        Some((5, "telegram")),
        &[("gender", "'female'"), ("blockedAt", "1000")],
        &[],
    )
    .await;
    seed(
        f,
        13,
        Some((6, "messenger")),
        &[
            ("firstName", "'Zeta'"),
            ("gender", "'male'"),
            ("email", "'zeta@example.test'"),
        ],
        &[],
    )
    .await;
}

fn criteria(operator: &str, conditions: &[Value]) -> String {
    json!({ "operator": operator, "conditions": conditions }).to_string()
}

fn cond(field: &str, value: &str) -> Value {
    json!({ "field": field, "operator": "eq", "value": value })
}

#[tokio::test]
async fn does_not_restrict_by_contactinboxes_when_channel_is_the_omnichannel_sentinel() {
    let f = fixture().await;
    filter_rows(&f).await;
    assert_eq!(
        ids(&f, &[("channel", "omnichannel")]).await,
        s(&[13, 11, 10])
    );
}

#[tokio::test]
async fn restricts_by_contactinboxes_when_a_real_channel_is_selected() {
    let f = fixture().await;
    filter_rows(&f).await;
    assert_eq!(ids(&f, &[("channel", "messenger")]).await, s(&[13, 11]));
}

#[tokio::test]
async fn composes_keyword_and_contact_filter_or_branches_without_overwriting_either() {
    let f = fixture().await;
    filter_rows(&f).await;
    let filter = criteria("or", &[cond("gender", "female"), cond("gender", "other")]);
    assert_eq!(
        ids(&f, &[("keyword", "zeta"), ("contactFilter", &filter)]).await,
        s(&[10])
    );
}

#[tokio::test]
async fn drops_the_hidden_blocked_contact_default_when_the_contact_filter_targets_blocked() {
    let f = fixture().await;
    filter_rows(&f).await;
    let filter = criteria("and", &[cond("blocked", "true")]);
    assert_eq!(ids(&f, &[("contactFilter", &filter)]).await, s(&[12]));
}

#[tokio::test]
async fn keeps_hiding_blocked_contacts_when_the_contact_filter_does_not_mention_blocked() {
    let f = fixture().await;
    filter_rows(&f).await;
    let filter = criteria("and", &[cond("gender", "female")]);
    assert_eq!(ids(&f, &[("contactFilter", &filter)]).await, s(&[10]));
}

#[tokio::test]
async fn lifts_the_hidden_default_for_an_or_filter_too_so_the_inbox_matches_the_contacts_page() {
    let f = fixture().await;
    filter_rows(&f).await;
    let filter = criteria("or", &[cond("blocked", "true"), cond("gender", "male")]);
    assert_eq!(
        ids(&f, &[("contactFilter", &filter)]).await,
        s(&[13, 12, 11])
    );
}

#[tokio::test]
async fn lifts_the_hidden_default_for_a_single_condition_or_filter_on_blocked() {
    let f = fixture().await;
    filter_rows(&f).await;
    let filter = criteria("or", &[cond("blocked", "true")]);
    assert_eq!(ids(&f, &[("contactFilter", &filter)]).await, s(&[12]));
}

/// Adapted: a workspace token sees email / phone, so an email keyword matches.
#[tokio::test]
async fn defaults_to_including_email_phone_when_no_scope_is_provided() {
    let f = fixture().await;
    filter_rows(&f).await;
    assert_eq!(ids(&f, &[("keyword", "zeta@example.test")]).await, s(&[13]));
}

/// 10 unread (never read, activity), 11 unread (activity after the read),
/// 12 read, 13 unread and followed.
async fn unread_rows(f: &Fixture) {
    inboxes(f).await;
    let t = |h: i64| (now() - h * HOUR).to_string();
    seed(
        f,
        10,
        Some((5, "telegram")),
        &[],
        &[("lastActivityAt", &t(1))],
    )
    .await;
    seed(
        f,
        11,
        Some((5, "telegram")),
        &[],
        &[("lastActivityAt", &t(2)), ("agentLastReadAt", &t(3))],
    )
    .await;
    seed(
        f,
        12,
        Some((5, "telegram")),
        &[],
        &[("lastActivityAt", &t(3)), ("agentLastReadAt", &t(1))],
    )
    .await;
    seed(
        f,
        13,
        Some((5, "telegram")),
        &[],
        &[("lastActivityAt", &t(4)), ("followed", "1")],
    )
    .await;
}

#[tokio::test]
async fn adds_the_unread_predicate_as_a_top_level_and_condition() {
    let f = fixture().await;
    unread_rows(&f).await;
    assert_eq!(
        ids(&f, &[("tags", r#"["unread"]"#)]).await,
        s(&[10, 11, 13])
    );
}

#[tokio::test]
async fn preserves_a_pre_existing_top_level_and_condition() {
    let f = fixture().await;
    unread_rows(&f).await;
    assert_eq!(
        ids(&f, &[("tags", r#"["unread","followUp"]"#)]).await,
        s(&[13])
    );
}

#[tokio::test]
async fn keeps_cursor_or_separate_from_the_unread_predicate() {
    let f = fixture().await;
    unread_rows(&f).await;
    let first = list(&f, &[("tags", r#"["unread"]"#), ("perPage", "1")]).await;
    assert_eq!(first["data"][0]["id"], json!("10"));
    let cursor = first["nextCursor"].as_str().expect("nextCursor").to_owned();
    let second = list(
        &f,
        &[
            ("tags", r#"["unread"]"#),
            ("perPage", "2"),
            ("cursor", &cursor),
        ],
    )
    .await;
    let got: Vec<&str> = second["data"]
        .as_array()
        .expect("data")
        .iter()
        .map(|c| c["id"].as_str().expect("id"))
        .collect();
    // The read conversation 12 stays out after the cursor.
    assert_eq!(got, vec!["11", "13"]);
    assert_eq!(second["nextCursor"], Value::Null);
    assert!(second["prevCursor"].is_string());
}
