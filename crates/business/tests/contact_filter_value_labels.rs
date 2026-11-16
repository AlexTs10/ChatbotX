//! Port of `packages/business/__tests__/contact-filter-value-labels.test.ts`
//! (P3.6-D2 slice filter value labels). Adapted: the mocked resolvers become
//! seeded rows of each entity, so "called with the de-duplicated ids" is
//! checked through the labels that come back; "a failing lookup rejects" uses
//! a dropped table instead of a rejected mock.

mod common;

use std::collections::BTreeMap;

use chatbotx_business::contact::filter_value_labels::{
    FILTER_VALUE_LABEL_TYPES, IdLabel, list_contact_activity_value_options,
    resolve_contact_filter_value_labels,
};
use chatbotx_business::pipeline::access::PipelineAccessScope;
use chatbotx_db::rusqlite::params;
use common::{OTHER_WS, TestDb, WS, contact, contact_inbox, inbox, setup, tag, two_workspaces};
use serde_json::json;

/// `MEMBER_SCOPE`: a plain member (workspace member 401).
const MEMBER_SCOPE: PipelineAccessScope = PipelineAccessScope::Member {
    workspace_member_id: 401,
    is_workspace_admin: false,
    can_view_contact_email_and_phone: true,
};

fn label(id: &str, name: &str) -> IdLabel {
    IdLabel {
        id: id.into(),
        name: name.into(),
    }
}

fn ids(pairs: &[(&str, &[&str])]) -> BTreeMap<String, Vec<String>> {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_owned(), v.iter().map(|s| (*s).to_owned()).collect()))
        .collect()
}

/// The workspaces, a member user 1101 ("Alice", member 401) and one row of
/// each simple entity.
async fn seeded() -> TestDb {
    let db = two_workspaces().await;
    setup(&db, |tx| {
        tag(tx, 11, WS, "VIP")?;
        tag(tx, 12, OTHER_WS, "Foreign")?;
        tx.execute(r#"INSERT INTO "User" ("id", "email", "name") VALUES (1101, 'alice@example.test', 'Alice')"#, [])?;
        tx.execute(r#"INSERT INTO "User" ("id", "email") VALUES (1102, 'bob@example.test')"#, [])?;
        tx.execute(
            r#"INSERT INTO "WorkspaceMember" ("id", "workspaceId", "userId", "role") VALUES (401, ?1, 1101, 'agent'), (402, ?1, 1102, 'agent')"#,
            params![WS],
        )?;
        tx.execute(r#"INSERT INTO "InboxTeam" ("id", "workspaceId", "name") VALUES (21, ?1, 'Support')"#, params![WS])?;
        tx.execute(r#"INSERT INTO "Sequence" ("id", "workspaceId", "name") VALUES (31, ?1, 'Drip'), (32, ?1, 'Drip 2')"#, params![WS])?;
        tx.execute(
            r#"INSERT INTO "Broadcast" ("id", "name", "workspaceId", "status", "schedulesType", "schedulesAt", "subaction", "channel", "deletedAt")
               VALUES (41, 'Promo', ?1, 'sent', 'now', 0, 'message', 'messenger', NULL),
                      (42, 'Gone', ?1, 'sent', 'now', 0, 'message', 'messenger', 1)"#,
            params![WS],
        )?;
        tx.execute(
            r#"INSERT INTO "EmailTopic" ("id", "workspaceId", "name", "createdAt") VALUES (51, ?1, 'Newsletter', 1), (52, ?1, 'News flash', 2), (53, ?1, 'Billing', 3)"#,
            params![WS],
        )?;
        tx.execute(
            r#"INSERT INTO "AutomatedResponse" ("id", "workspaceId", "keywords", "status", "text", "type", "createdAt")
               VALUES (61, ?1, '["hi","hello"]', 1, 't', 'inbound', 1), (62, ?1, '["bye"]', 1, 't', 'inbound', 2),
                      (63, ?1, '[]', 1, 't', 'inbound', 3), (64, ?1, '["out"]', 1, 't', 'outbound', 4)"#,
            params![WS],
        )?;
        tx.execute(
            r#"INSERT INTO "Pipeline" ("id", "workspaceId", "name", "type", "referenceName", "settings") VALUES (71, ?1, 'Sales', 'custom', 'sales', '{}'), (72, ?1, 'Hidden', 'custom', 'hidden', '{}')"#,
            params![WS],
        )?;
        tx.execute(
            r#"INSERT INTO "PipelineStage" ("id", "workspaceId", "pipelineId", "name", "position") VALUES (81, ?1, 71, 'Lead', 0), (82, ?1, 72, 'Secret', 0)"#,
            params![WS],
        )?;
        // Member 401 may view pipeline 71 only.
        tx.execute(
            r#"INSERT INTO "PipelineMember" ("id", "workspaceId", "pipelineId", "workspaceMemberId", "canView") VALUES (91, ?1, 71, 401, 1), (92, ?1, 72, 401, 0)"#,
            params![WS],
        )?;
        Ok(())
    })
    .await;
    db
}

#[tokio::test]
async fn routes_each_id_type_to_its_resolver_within_the_workspace() {
    let db = seeded().await;
    let labels = resolve_contact_filter_value_labels(
        &db,
        WS,
        ids(&[
            ("tags", &["11", "12"]),
            ("members", &["1101", "1102"]),
            ("inboxTeams", &["21"]),
        ]),
        MEMBER_SCOPE,
    )
    .await
    .unwrap();
    // Tag 12 belongs to another workspace.
    assert_eq!(labels["tags"], vec![label("11", "VIP")]);
    assert_eq!(
        labels["members"],
        vec![label("1101", "Alice"), label("1102", "--")]
    );
    assert_eq!(labels["inboxTeams"], vec![label("21", "Support")]);
}

#[tokio::test]
async fn returns_an_empty_list_for_every_type_that_was_not_asked_for() {
    let db = seeded().await;
    let labels = resolve_contact_filter_value_labels(&db, WS, BTreeMap::new(), MEMBER_SCOPE)
        .await
        .unwrap();
    let keys: Vec<&str> = labels.keys().map(String::as_str).collect();
    let mut expected: Vec<&str> = FILTER_VALUE_LABEL_TYPES.to_vec();
    expected.sort_unstable();
    assert_eq!(keys, expected);
    assert!(labels.values().all(Vec::is_empty));
}

#[tokio::test]
async fn routes_pipeline_and_stage_ids_to_their_own_resolvers() {
    let db = seeded().await;
    let labels = resolve_contact_filter_value_labels(
        &db,
        WS,
        ids(&[
            ("pipelines", &["71", "71", "72"]),
            ("pipelineStages", &["81", "82", "999"]),
        ]),
        MEMBER_SCOPE,
    )
    .await
    .unwrap();
    // Pipeline names are ACL data: the caller's scope hides pipeline 72.
    assert_eq!(labels["pipelines"], vec![label("71", "Sales")]);
    assert_eq!(labels["pipelineStages"], vec![label("81", "Lead")]);
    let all = resolve_contact_filter_value_labels(
        &db,
        WS,
        ids(&[("pipelines", &["71", "72"])]),
        PipelineAccessScope::WorkspaceToken,
    )
    .await
    .unwrap();
    assert_eq!(
        all["pipelines"],
        vec![label("71", "Sales"), label("72", "Hidden")]
    );
}

#[tokio::test]
async fn de_duplicates_ids_before_looking_them_up() {
    let db = seeded().await;
    let labels = resolve_contact_filter_value_labels(
        &db,
        WS,
        ids(&[("sequences", &["31", "31", "32"])]),
        MEMBER_SCOPE,
    )
    .await
    .unwrap();
    assert_eq!(
        labels["sequences"],
        vec![label("31", "Drip"), label("32", "Drip 2")]
    );
}

#[tokio::test]
async fn omits_ids_that_no_longer_exist_so_the_caller_can_flag_them_unknown() {
    let db = seeded().await;
    let labels = resolve_contact_filter_value_labels(
        &db,
        WS,
        ids(&[("broadcasts", &["41", "42", "999"])]),
        MEMBER_SCOPE,
    )
    .await
    .unwrap();
    assert_eq!(labels["broadcasts"], vec![label("41", "Promo")]);
}

#[tokio::test]
async fn a_failing_lookup_rejects_instead_of_reporting_every_id_as_deleted() {
    let db = seeded().await;
    setup(&db, |tx| {
        tx.execute_batch(r#"DROP TABLE "InboxTeamMember"; DROP TABLE "InboxTeam";"#)?;
        Ok(())
    })
    .await;
    let out =
        resolve_contact_filter_value_labels(&db, WS, ids(&[("inboxTeams", &["21"])]), MEMBER_SCOPE)
            .await;
    assert!(out.is_err());
}

#[tokio::test]
async fn routes_flows_nodes_keywords_whatsapp_flows_and_email_topics_to_their_resolvers() {
    let db = seeded().await;
    setup(&db, |tx| {
        tx.execute(r#"INSERT INTO "Flow" ("id", "name", "workspaceId") VALUES (7, 'Flow A', ?1)"#, params![WS])?;
        tx.execute(
            r#"INSERT INTO "FlowVersion" ("id", "workspaceId", "flowId", "nodes", "edges", "isDraft", "isLatest", "startNodeId") VALUES (700, ?1, 7, ?2, '[]', 0, 1, 9)"#,
            params![WS, json!([{ "id": "9", "data": { "name": "Welcome" } }, { "id": "10", "data": { "name": " " } }]).to_string()],
        )?;
        tx.execute(r#"UPDATE "Flow" SET "currentVersionId" = 700 WHERE "id" = 7"#, [])?;
        contact(tx, 1, WS)?;
        inbox(tx, 801, WS, "messenger")?;
        contact_inbox(tx, 811, 1, 801, "messenger", "src-1")?;
        tx.execute(
            r#"INSERT INTO "ContactReferralEvent" ("id", "workspaceId", "contactId", "contactInboxId", "channel", "type", "occurredAt", "adId", "dedupKey", "metadata")
               VALUES (1, ?1, 1, 811, 'messenger', 'ad', 10, '120', 'k1', '{"adTitle":"Summer sale"}')"#,
            params![WS],
        )?;
        Ok(())
    })
    .await;
    let labels = resolve_contact_filter_value_labels(
        &db,
        WS,
        ids(&[
            ("flows", &["7"]),
            ("flowNodes", &["7:9", "7:9", "7:10", "7:404", "9"]),
            ("keywords", &["61", "63", "64"]),
            ("whatsappFlows", &["5"]),
            ("emailTopics", &["51"]),
            ("ads", &["120", "121"]),
        ]),
        MEMBER_SCOPE,
    )
    .await
    .unwrap();
    assert_eq!(labels["flows"], vec![label("7", "Flow A")]);
    assert_eq!(
        labels["flowNodes"],
        vec![
            label("7:9", "Flow A · Welcome"),
            label("7:10", "Flow A · 10")
        ]
    );
    // An outbound rule is not a contact keyword; a rule with no keywords is `#id`.
    assert_eq!(
        labels["keywords"],
        vec![label("61", "hi, hello"), label("63", "#63")]
    );
    assert!(labels["whatsappFlows"].is_empty());
    assert_eq!(labels["emailTopics"], vec![label("51", "Newsletter")]);
    assert_eq!(
        labels["ads"],
        vec![label("120", "Summer sale (120)"), label("121", "121")]
    );
}

// --- listContactActivityValueOptions -----------------------------------

fn options(
    out: Vec<chatbotx_business::contact::filter_value_labels::ContactActivityValueOption>,
) -> Vec<(String, String)> {
    out.into_iter().map(|o| (o.value, o.label)).collect()
}

fn pairs(list: &[(&str, &str)]) -> Vec<(String, String)> {
    list.iter()
        .map(|(v, l)| ((*v).to_owned(), (*l).to_owned()))
        .collect()
}

#[tokio::test]
async fn keywords_lists_contact_rules_by_their_keywords_searched_on_the_server() {
    let db = seeded().await;
    let out = list_contact_activity_value_options(&db, WS, "keywords", Some("  hi "))
        .await
        .unwrap();
    assert_eq!(options(out), pairs(&[("61", "hi, hello")]));
}

#[tokio::test]
async fn keywords_no_search_means_no_keyword_filter() {
    let db = seeded().await;
    let out = list_contact_activity_value_options(&db, WS, "keywords", None)
        .await
        .unwrap();
    // Newest first; outbound rules are not listed.
    assert_eq!(
        options(out),
        pairs(&[("63", "#63"), ("62", "bye"), ("61", "hi, hello")])
    );
}

#[tokio::test]
async fn whatsapp_flows_bounded_name_filtered_and_workspace_scoped() {
    // Adapted: no WhatsApp integration is seeded, so the workspace scope
    // leaves nothing to list.
    let db = seeded().await;
    let out = list_contact_activity_value_options(&db, WS, "whatsappFlows", Some("lead"))
        .await
        .unwrap();
    assert!(out.is_empty());
}

#[tokio::test]
async fn email_topics_bounded_and_name_filtered() {
    let db = seeded().await;
    let out = list_contact_activity_value_options(&db, WS, "emailTopics", Some("news"))
        .await
        .unwrap();
    assert_eq!(
        options(out),
        pairs(&[("52", "News flash"), ("51", "Newsletter")])
    );
}

#[tokio::test]
async fn ads_the_recent_ads_of_the_workspace_labelled_with_their_headline() {
    let db = seeded().await;
    setup(&db, |tx| {
        contact(tx, 1, WS)?;
        inbox(tx, 801, WS, "messenger")?;
        contact_inbox(tx, 811, 1, 801, "messenger", "src-1")?;
        tx.execute(
            r#"INSERT INTO "ContactReferralEvent" ("id", "workspaceId", "contactId", "contactInboxId", "channel", "type", "occurredAt", "adId", "dedupKey", "metadata")
               VALUES (1, ?1, 1, 811, 'messenger', 'ad', 20, '120', 'k1', '{"adTitle":"Summer sale"}'),
                      (2, ?1, 1, 811, 'messenger', 'ad', 10, '121', 'k2', NULL),
                      (3, ?1, 1, 811, 'messenger', 'ad', 5, '122', 'k3', '{"adTitle":"Winter"}')"#,
            params![WS],
        )?;
        Ok(())
    })
    .await;
    let out = list_contact_activity_value_options(&db, WS, "ads", Some(" summer "))
        .await
        .unwrap();
    assert_eq!(options(out), pairs(&[("120", "Summer sale (120)")]));
    let out = list_contact_activity_value_options(&db, WS, "ads", None)
        .await
        .unwrap();
    assert_eq!(
        options(out),
        pairs(&[
            ("120", "Summer sale (120)"),
            ("121", "121"),
            ("122", "Winter (122)")
        ])
    );
}

#[tokio::test]
async fn a_failing_lookup_rejects_instead_of_looking_like_an_empty_list() {
    let db = seeded().await;
    setup(&db, |tx| {
        tx.execute_batch(r#"DROP TABLE "EmailTopic";"#)?;
        Ok(())
    })
    .await;
    assert!(
        list_contact_activity_value_options(&db, WS, "emailTopics", None)
            .await
            .is_err()
    );
}
