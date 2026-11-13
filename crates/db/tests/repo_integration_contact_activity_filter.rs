//! Port of `packages/database/__tests__/integration/contact-activity-filter.test.ts`
//! (P3.2b slice C2). Real-SQL semantics on a migrated SQLite temp file: "own" = workspace 1
//! (contacts 101-105 = the TS `contacts[0..4]`), "foreign" = workspace 2 (contact 201).
//! The fixture rows are written with the ported `contact_activity` repository, as the TS.

mod common;

use chatbotx_db::Db;
use chatbotx_db::repo::contact::find_ids_matching_filter;
use chatbotx_db::repo::contact_activity::{
    self as ca, ContactActivityRecord, ContactKeywordMessageRecord,
};
use chatbotx_db::repo::queries::contact_filter::{FilterCriteria, apply_contact_filter};
use chatbotx_db::repo::utils::normalize_for_match;
use chatbotx_db::repo::{self, parse_iso};
use chatbotx_db::rusqlite::{self, params};
use serde_json::{Value as Json, json};

const WS: i64 = 1;
const FOREIGN_WS: i64 = 2;
const CONTACTS: [i64; 5] = [101, 102, 103, 104, 105];
const FOREIGN_CONTACT: i64 = 201;
const FLOW_1: i64 = 1001;
const FLOW_2: i64 = 1002;
const NODE_A: i64 = 2001;
const TOPIC_1: i64 = 3001;
const KEYWORD_1: i64 = 4001;
const KEYWORD_2: i64 = 4002;
/// `2026-06-01T00:00:00.000Z` (no condition here reads `now`).
const NOW: i64 = 1_780_272_000_000;

fn t(iso: &str) -> i64 {
    parse_iso(iso).expect("iso")
}

fn activity(
    contact_id: i64,
    activity_type: &str,
    parent_reference_id: i64,
    reference_id: i64,
    reference_type: &str,
) -> ContactActivityRecord {
    ContactActivityRecord {
        activity_type: activity_type.to_owned(),
        contact_id,
        occurred_at: t("2026-01-01T00:00:00Z"),
        parent_reference_id,
        reference_id,
        reference_type: reference_type.to_owned(),
    }
}

fn flow(contact_id: i64, reference_id: i64) -> ContactActivityRecord {
    activity(contact_id, "flowExecuted", 0, reference_id, "flow")
}

/// The TS `beforeEach`.
async fn fixture() -> common::TestDb {
    let db = common::fresh_db().await;
    common::seed_workspace(&db, 1, WS).await;
    common::seed_workspace(&db, 2, FOREIGN_WS).await;
    db.write(|tx| {
        for id in CONTACTS {
            tx.execute(
                r#"INSERT INTO "Contact" ("id", "workspaceId") VALUES (?1, ?2)"#,
                params![id, WS],
            )?;
        }
        tx.execute(
            r#"INSERT INTO "Contact" ("id", "workspaceId") VALUES (?1, ?2)"#,
            params![FOREIGN_CONTACT, FOREIGN_WS],
        )?;
        Ok(())
    })
    .await
    .expect("contacts");
    let [c0, c1, c2, c3, _] = CONTACTS;
    let records = vec![
        flow(c0, FLOW_1),
        flow(c1, FLOW_1),
        flow(c2, FLOW_2),
        activity(c0, "flowNodeExecuted", FLOW_1, NODE_A, "flowNode"),
        activity(c1, "flowNodeExecuted", FLOW_2, NODE_A, "flowNode"),
        activity(
            c0,
            "automatedResponseMatched",
            0,
            KEYWORD_1,
            "automatedResponse",
        ),
        activity(
            c1,
            "automatedResponseMatched",
            0,
            KEYWORD_2,
            "automatedResponse",
        ),
        activity(c3, "messageSent", 0, -1, "emailTopic"),
        activity(c2, "messageSent", 0, TOPIC_1, "emailTopic"),
    ];
    repo::write(&db, move |tx| {
        ca::upsert_activities(tx, WS, &records)?;
        // The same flow id in another workspace must never match this workspace.
        ca::upsert_activities(tx, FOREIGN_WS, &[flow(FOREIGN_CONTACT, FLOW_1)])?;
        for (contact_id, text) in [(c0, "Xin chào các bạn"), (c1, "Tôi muốn mua HÀNG")] {
            ca::insert_keyword_message(
                tx,
                WS,
                &ContactKeywordMessageRecord {
                    automated_response_id: KEYWORD_1,
                    contact_id,
                    message_id: contact_id * 10 + 9,
                    normalized_text: normalize_for_match(text),
                    occurred_at: t("2026-01-01T00:00:00Z"),
                    text: text.to_owned(),
                },
            )?;
        }
        ca::upsert_last_delivered(
            tx,
            WS,
            &[
                (c0, t("2026-05-19T10:00:00Z")),
                (c1, t("2026-05-20T10:00:00Z")),
            ],
        )?;
        Ok(())
    })
    .await
    .expect("activity fixture");
    db
}

fn criteria(conditions: Vec<Json>, operator: &str) -> FilterCriteria {
    serde_json::from_value(json!({ "operator": operator, "conditions": conditions }))
        .expect("criteria")
}

/// `matching(conditions, operator)`: the own-workspace contacts the filter matches.
async fn matching_op(db: &Db, conditions: Vec<Json>, operator: &str) -> Vec<i64> {
    let q = apply_contact_filter(&criteria(conditions, operator), Some(WS), NOW)
        .expect("Expected the filter to render a predicate");
    let sql = format!(
        "SELECT \"Contact\".\"id\" FROM \"Contact\" WHERE \"Contact\".\"workspaceId\" = {WS} AND ({}) ORDER BY 1",
        q.text
    );
    db.read(move |c| {
        let mut stmt = c.prepare(&sql)?;
        let rows = stmt
            .query_map(rusqlite::params_from_iter(q.params.iter()), |r| r.get(0))?
            .collect::<rusqlite::Result<Vec<i64>>>()?;
        Ok(rows)
    })
    .await
    .expect("matching")
}

async fn matching(db: &Db, conditions: Vec<Json>) -> Vec<i64> {
    matching_op(db, conditions, "and").await
}

/// `ids(...indexes)`.
fn ids(indexes: &[usize]) -> Vec<i64> {
    let mut out: Vec<i64> = indexes.iter().map(|i| CONTACTS[*i]).collect();
    out.sort_unstable();
    out
}

fn c(field: &str, operator: &str, value: Json) -> Json {
    json!({ "field": field, "operator": operator, "value": value })
}

fn bare(field: &str, operator: &str) -> Json {
    json!({ "field": field, "operator": operator })
}

fn s(id: i64) -> String {
    id.to_string()
}

// ts:258 ported
#[tokio::test]
async fn is_matches_any_listed_flow_or_and_never_another_workspace_s_activity() {
    let db = fixture().await;
    assert_eq!(
        matching(&db, vec![c("executedFlow", "in", json!([s(FLOW_1)]))]).await,
        ids(&[0, 1])
    );
    assert_eq!(
        matching(
            &db,
            vec![c("executedFlow", "in", json!([s(FLOW_1), s(FLOW_2)]))]
        )
        .await,
        ids(&[0, 1, 2])
    );
}

// ts:271 ported
#[tokio::test]
async fn is_not_is_the_exact_complement_and_includes_contacts_with_no_history() {
    let db = fixture().await;
    let positive = matching(&db, vec![c("executedFlow", "in", json!([s(FLOW_1)]))]).await;
    let negative = matching(&db, vec![c("executedFlow", "notIn", json!([s(FLOW_1)]))]).await;
    assert_eq!(negative, ids(&[2, 3, 4]));
    let mut all = [positive, negative].concat();
    all.sort_unstable();
    assert_eq!(all, ids(&[0, 1, 2, 3, 4]));
}

// ts:282 ported
#[tokio::test]
async fn has_no_value_has_a_value_test_for_any_activity_of_the_type() {
    let db = fixture().await;
    assert_eq!(
        matching(&db, vec![bare("executedFlow", "isEmpty")]).await,
        ids(&[3, 4])
    );
    assert_eq!(
        matching(&db, vec![bare("executedFlow", "isNotEmpty")]).await,
        ids(&[0, 1, 2])
    );
}

// ts:291 ported
#[tokio::test]
async fn a_step_is_scoped_by_its_flow_the_same_node_id_in_another_flow_is_a_different_step() {
    let db = fixture().await;
    let step = |flow: i64| json!([format!("{flow}:{NODE_A}")]);
    assert_eq!(
        matching(&db, vec![c("executedStep", "in", step(FLOW_1))]).await,
        ids(&[0])
    );
    assert_eq!(
        matching(&db, vec![c("executedStep", "in", step(FLOW_2))]).await,
        ids(&[1])
    );
    assert_eq!(
        matching(&db, vec![c("executedStep", "notIn", step(FLOW_1))]).await,
        ids(&[1, 2, 3, 4])
    );
}

// ts:321 ported
#[tokio::test]
async fn keywords_received_follow_the_same_set_semantics() {
    let db = fixture().await;
    assert_eq!(
        matching(
            &db,
            vec![c(
                "keywordsReceived",
                "in",
                json!([s(KEYWORD_1), s(KEYWORD_2)])
            )]
        )
        .await,
        ids(&[0, 1])
    );
    assert_eq!(
        matching(
            &db,
            vec![c("keywordsReceived", "notIn", json!([s(KEYWORD_1)]))]
        )
        .await,
        ids(&[1, 2, 3, 4])
    );
}

// ts:338 ported
#[tokio::test]
async fn email_the_fixed_no_topic_bucket_is_separate_from_a_real_topic() {
    let db = fixture().await;
    assert_eq!(
        matching(&db, vec![c("emailSent", "in", json!(["noTopic"]))]).await,
        ids(&[3])
    );
    assert_eq!(
        matching(&db, vec![c("emailSent", "in", json!([s(TOPIC_1)]))]).await,
        ids(&[2])
    );
    assert_eq!(
        matching(
            &db,
            vec![c("emailSent", "in", json!(["noTopic", s(TOPIC_1)]))]
        )
        .await,
        ids(&[2, 3])
    );
    // The other phases have no rows at all.
    assert_eq!(
        matching(&db, vec![bare("emailOpened", "isNotEmpty")]).await,
        Vec::<i64>::new()
    );
}

// ts:360 ported
#[tokio::test]
async fn conditions_combine_with_and_or() {
    let db = fixture().await;
    let flow = c("executedFlow", "in", json!([s(FLOW_1)]));
    let keyword = c("keywordsReceived", "in", json!([s(KEYWORD_2)]));
    assert_eq!(
        matching_op(&db, vec![flow.clone(), keyword.clone()], "and").await,
        ids(&[1])
    );
    assert_eq!(
        matching_op(&db, vec![flow, keyword], "or").await,
        ids(&[0, 1])
    );
}

// ts:371 ported
#[tokio::test]
async fn a_malformed_condition_matches_nobody_instead_of_everyone() {
    let db = fixture().await;
    assert_eq!(
        matching(&db, vec![c("executedFlow", "in", json!(["not-an-id"]))]).await,
        Vec::<i64>::new()
    );
}

/// ts:380 "%s %j is accent- and case-insensitive".
async fn sent_message_case(operator: &str, value: &str, expected: &[usize]) {
    let db = fixture().await;
    assert_eq!(
        matching(&db, vec![c("sentMessage", operator, json!(value))]).await,
        ids(expected)
    );
}

// ts:380 ported
#[tokio::test]
async fn sent_message_eq_xin_chao_cac_ban_is_accent_and_case_insensitive() {
    sent_message_case("eq", "xin chao cac ban", &[0]).await;
}

// ts:380 ported
#[tokio::test]
async fn sent_message_eq_xin_chao_cac_ban_mixed_case_is_accent_and_case_insensitive() {
    sent_message_case("eq", "Xin Chào Các Bạn", &[0]).await;
}

// ts:380 ported
#[tokio::test]
async fn sent_message_contains_mua_hang_is_accent_and_case_insensitive() {
    sent_message_case("contains", "mua hang", &[1]).await;
}

// ts:380 ported
#[tokio::test]
async fn sent_message_contains_mua_hang_upper_is_accent_and_case_insensitive() {
    sent_message_case("contains", "MUA HÀNG", &[1]).await;
}

// ts:380 ported
#[tokio::test]
async fn sent_message_starts_with_toi_muon_is_accent_and_case_insensitive() {
    sent_message_case("startsWith", "toi muon", &[1]).await;
}

// ts:380 ported
#[tokio::test]
async fn sent_message_ends_with_cac_ban_is_accent_and_case_insensitive() {
    sent_message_case("endsWith", "cac ban", &[0]).await;
}

// ts:380 ported
#[tokio::test]
async fn sent_message_contains_khong_co_is_accent_and_case_insensitive() {
    sent_message_case("contains", "khong co", &[]).await;
}

// ts:394 ported
#[tokio::test]
async fn sent_message_is_not_doesn_t_contain_are_exact_complements_incl_contacts_with_no_message() {
    let db = fixture().await;
    assert_eq!(
        matching(&db, vec![c("sentMessage", "ne", json!("xin chao cac ban"))]).await,
        ids(&[1, 2, 3, 4])
    );
    assert_eq!(
        matching(&db, vec![c("sentMessage", "notContains", json!("hang"))]).await,
        ids(&[0, 2, 3, 4])
    );
}

// ts:407 ported
#[tokio::test]
async fn sent_message_has_no_value_has_a_value() {
    let db = fixture().await;
    assert_eq!(
        matching(&db, vec![bare("sentMessage", "isEmpty")]).await,
        ids(&[2, 3, 4])
    );
    assert_eq!(
        matching(&db, vec![bare("sentMessage", "isNotEmpty")]).await,
        ids(&[0, 1])
    );
}

// ts:416 ported
#[tokio::test]
async fn sent_message_treats_like_wildcards_in_the_value_literally() {
    let db = fixture().await;
    assert_eq!(
        matching(&db, vec![c("sentMessage", "contains", json!("%"))]).await,
        Vec::<i64>::new()
    );
    assert_eq!(
        matching(&db, vec![c("sentMessage", "contains", json!("_"))]).await,
        Vec::<i64>::new()
    );
}

// ts:431 ported
#[tokio::test]
async fn last_delivered_compares_on_the_stored_timestamp() {
    let db = fixture().await;
    assert_eq!(
        matching(&db, vec![c("lastDelivered", "gt", json!("2026-05-19"))]).await,
        ids(&[1])
    );
    assert_eq!(
        matching(&db, vec![c("lastDelivered", "eq", json!("2026-05-19"))]).await,
        ids(&[0])
    );
    assert_eq!(
        matching(
            &db,
            vec![c(
                "lastDelivered",
                "isBetween",
                json!(["2026-05-19", "2026-05-20"])
            )]
        )
        .await,
        ids(&[0, 1])
    );
}

// ts:453 ported
#[tokio::test]
async fn last_delivered_is_not_not_between_include_contacts_that_were_never_delivered_to() {
    let db = fixture().await;
    assert_eq!(
        matching(&db, vec![c("lastDelivered", "ne", json!("2026-05-19"))]).await,
        ids(&[1, 2, 3, 4])
    );
    assert_eq!(
        matching(
            &db,
            vec![c(
                "lastDelivered",
                "notBetween",
                json!(["2026-05-19", "2026-05-19"])
            )]
        )
        .await,
        ids(&[1, 2, 3, 4])
    );
}

// ts:470 ported
#[tokio::test]
async fn last_delivered_has_no_value_has_a_value() {
    let db = fixture().await;
    assert_eq!(
        matching(&db, vec![bare("lastDelivered", "isEmpty")]).await,
        ids(&[2, 3, 4])
    );
    assert_eq!(
        matching(&db, vec![bare("lastDelivered", "isNotEmpty")]).await,
        ids(&[0, 1])
    );
}

/// `contactRepository.findIdsMatchingFilter`, sorted.
async fn find_ids(db: &Db, contact_ids: Vec<i64>, conditions: Vec<Json>) -> Vec<i64> {
    let filter = criteria(conditions, "and");
    let mut found = repo::read(db, move |conn| {
        find_ids_matching_filter(conn, WS, &contact_ids, &filter, NOW)
    })
    .await
    .expect("findIdsMatchingFilter");
    found.sort_unstable();
    found
}

// ts:483 ported
#[tokio::test]
async fn find_ids_matching_filter_returns_only_the_page_s_contacts_that_match_never_another_workspace_s()
 {
    let db = fixture().await;
    let mut page = CONTACTS.to_vec();
    page.push(FOREIGN_CONTACT);
    assert_eq!(
        find_ids(&db, page, vec![c("executedFlow", "in", json!([s(FLOW_1)]))]).await,
        ids(&[0, 1])
    );
}

// ts:498 ported
#[tokio::test]
async fn find_ids_matching_filter_evaluates_negatives_not_exists_over_the_page_including_contacts_with_no_history()
 {
    let db = fixture().await;
    assert_eq!(
        find_ids(
            &db,
            CONTACTS.to_vec(),
            vec![c("executedFlow", "notIn", json!([s(FLOW_1)]))]
        )
        .await,
        ids(&[2, 3, 4])
    );
}

// ts:513 ported
#[tokio::test]
async fn find_ids_matching_filter_ignores_contacts_outside_the_supplied_page() {
    let db = fixture().await;
    assert_eq!(
        find_ids(
            &db,
            vec![CONTACTS[1]],
            vec![c("executedFlow", "in", json!([s(FLOW_1)]))]
        )
        .await,
        ids(&[1])
    );
}

// ts:528 ported
#[tokio::test]
async fn find_ids_matching_filter_fails_closed_for_an_unusable_filter_value_and_answers_empty_for_an_empty_page()
 {
    let db = fixture().await;
    assert_eq!(
        find_ids(
            &db,
            CONTACTS.to_vec(),
            vec![c("executedFlow", "in", json!(["not-an-id"]))]
        )
        .await,
        Vec::<i64>::new()
    );
    assert_eq!(
        find_ids(
            &db,
            vec![],
            vec![c("executedFlow", "in", json!([s(FLOW_1)]))]
        )
        .await,
        Vec::<i64>::new()
    );
}
