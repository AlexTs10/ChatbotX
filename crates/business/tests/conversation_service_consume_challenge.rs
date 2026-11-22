//! Port of `packages/business/__tests__/conversation-service-consume-challenge.test.ts`.
//! Adapted: the mocked Drizzle `update().set().where().returning()` chain
//! becomes a real conversation row; "the WHERE clause binds X" becomes "a row
//! that differs in X is not touched".

mod common;

use chatbotx_business::conversation_challenge::{
    AdvanceAttempt, advance_step_challenge_attempt, consume_challenge, restore_challenge_if_absent,
    update_challenge,
};
use chatbotx_db::Db;
use chatbotx_db::rusqlite::params;
use common::{WS, contact, fresh_db, setup, workspace};
use serde_json::{Value, json};

const CONV: i64 = 501;

fn step_challenge() -> Value {
    json!({ "type": "step", "data": {
        "flowId": "1", "nodeId": "n1", "stepId": "step-1", "attempts": 1,
        "lastAttemptAt": "2026-11-09T09:00:00.000Z", "startedAt": "2026-11-09T08:00:00.000Z",
        "challengeId": "challenge-1",
    }})
}

async fn db_with(attributes: Option<Value>) -> common::TestDb {
    let db = fresh_db().await;
    let attributes = attributes.map(|a| a.to_string());
    setup(&db, move |tx| {
        workspace(tx, 1, WS)?;
        contact(tx, 3, WS)?;
        tx.execute(
            r#"INSERT INTO "Conversation" ("id", "workspaceId", "contactId", "additionalAttributes") VALUES (?1, ?2, 3, ?3)"#,
            params![CONV, WS, attributes],
        )?;
        Ok(())
    })
    .await;
    db
}

async fn attributes(db: &Db) -> Value {
    let raw: Option<String> = db
        .read(|conn| {
            Ok(conn.query_row(
                r#"SELECT "additionalAttributes" FROM "Conversation" WHERE "id" = ?1"#,
                [CONV],
                |r| r.get(0),
            )?)
        })
        .await
        .unwrap();
    raw.map(|r| serde_json::from_str(&r).unwrap())
        .unwrap_or(Value::Null)
}

mod consume_challenge_tests {
    use super::*;

    #[tokio::test]
    async fn clears_the_challenge_and_returns_true_when_a_row_is_claimed() {
        let db = db_with(Some(json!({ "challenge": step_challenge(), "other": 1 }))).await;
        assert!(
            consume_challenge(&db, WS, CONV, "step-1", "challenge-1")
                .await
                .unwrap()
        );
        assert_eq!(attributes(&db).await, json!({ "other": 1 }));
    }

    #[tokio::test]
    async fn returns_false_no_claim_when_zero_rows_match() {
        let db = db_with(None).await;
        assert!(
            !consume_challenge(&db, WS, CONV, "step-1", "challenge-1")
                .await
                .unwrap()
        );
    }

    #[tokio::test]
    async fn scopes_the_where_clause_by_id_workspace_id_step_id_and_challenge_id_as_bound_parameters()
     {
        let db = db_with(Some(json!({ "challenge": step_challenge() }))).await;
        assert!(
            !consume_challenge(&db, WS + 1, CONV, "step-1", "challenge-1")
                .await
                .unwrap()
        );
        assert!(
            !consume_challenge(&db, WS, CONV + 1, "step-1", "challenge-1")
                .await
                .unwrap()
        );
        assert!(
            !consume_challenge(&db, WS, CONV, "step-2", "challenge-1")
                .await
                .unwrap()
        );
        // An injection attempt is a value, not SQL.
        assert!(
            !consume_challenge(&db, WS, CONV, "step-1", "x' OR '1'='1")
                .await
                .unwrap()
        );
        assert_eq!(attributes(&db).await["challenge"], step_challenge());
    }

    #[tokio::test]
    async fn does_not_clear_the_challenge_no_matching_row_when_the_challenge_id_differs() {
        let db = db_with(Some(json!({ "challenge": step_challenge() }))).await;
        assert!(
            !consume_challenge(&db, WS, CONV, "step-1", "challenge-2")
                .await
                .unwrap()
        );
        assert_eq!(attributes(&db).await["challenge"], step_challenge());
    }
}

mod restore_challenge_if_absent_tests {
    use super::*;

    #[tokio::test]
    async fn returns_true_when_the_conditional_write_claims_a_row() {
        let db = db_with(Some(json!({ "other": 1 }))).await;
        assert!(
            restore_challenge_if_absent(&db, WS, CONV, step_challenge())
                .await
                .unwrap()
        );
        assert_eq!(
            attributes(&db).await,
            json!({ "other": 1, "challenge": step_challenge() })
        );
        let db = db_with(None).await;
        assert!(
            restore_challenge_if_absent(&db, WS, CONV, step_challenge())
                .await
                .unwrap()
        );
    }

    #[tokio::test]
    async fn returns_false_when_a_challenge_already_exists_predicate_excludes_the_row() {
        let newer =
            json!({ "type": "step", "data": { "stepId": "step-1", "challengeId": "newer" } });
        let db = db_with(Some(json!({ "challenge": newer }))).await;
        assert!(
            !restore_challenge_if_absent(&db, WS, CONV, step_challenge())
                .await
                .unwrap()
        );
        assert_eq!(attributes(&db).await["challenge"], newer);
    }

    #[tokio::test]
    async fn guards_the_write_with_an_absence_predicate_and_binds_scoping_values_as_parameters() {
        let db = db_with(None).await;
        assert!(
            !restore_challenge_if_absent(&db, WS + 1, CONV, step_challenge())
                .await
                .unwrap()
        );
        assert!(
            !restore_challenge_if_absent(&db, WS, CONV + 1, step_challenge())
                .await
                .unwrap()
        );
        assert_eq!(attributes(&db).await, Value::Null);
    }
}

mod advance_step_challenge_attempt_tests {
    use super::*;

    const LAST_ATTEMPT_AT: i64 = 1_794_218_400_000; // 2026-11-09T10:00:00.000Z

    fn props() -> AdvanceAttempt {
        AdvanceAttempt {
            workspace_id: WS,
            conversation_id: CONV,
            step_id: "step-1".into(),
            challenge_id: "challenge-1".into(),
            from_attempts: 1,
            to_attempts: 2,
            last_attempt_at: LAST_ATTEMPT_AT,
        }
    }

    #[tokio::test]
    async fn returns_true_for_the_one_caller_whose_compare_and_set_claims_the_row() {
        let db = db_with(Some(json!({ "challenge": step_challenge() }))).await;
        assert!(advance_step_challenge_attempt(&db, props()).await.unwrap());
    }

    #[tokio::test]
    async fn returns_false_never_throws_when_a_concurrent_reply_already_advanced_the_attempt() {
        let db = db_with(Some(json!({ "challenge": step_challenge() }))).await;
        assert!(advance_step_challenge_attempt(&db, props()).await.unwrap());
        assert!(!advance_step_challenge_attempt(&db, props()).await.unwrap());
    }

    #[tokio::test]
    async fn only_matches_a_step_challenge_of_this_step_and_challenge_cycle_at_the_expected_attempt_all_bound_as_parameters()
     {
        let db = db_with(Some(json!({ "challenge": step_challenge() }))).await;
        for other in [
            AdvanceAttempt {
                step_id: "step-2".into(),
                ..props()
            },
            AdvanceAttempt {
                challenge_id: "challenge-2".into(),
                ..props()
            },
            AdvanceAttempt {
                from_attempts: 2,
                ..props()
            },
            AdvanceAttempt {
                workspace_id: WS + 1,
                ..props()
            },
        ] {
            assert!(!advance_step_challenge_attempt(&db, other).await.unwrap());
        }
        let mut quick = step_challenge();
        quick["type"] = json!("quickReply");
        update_challenge(&db, WS, CONV, Some(quick)).await.unwrap();
        assert!(!advance_step_challenge_attempt(&db, props()).await.unwrap());
    }

    #[tokio::test]
    async fn moves_attempts_and_last_attempt_at_but_leaves_started_at_the_deadline_origin_untouched()
     {
        let db = db_with(Some(json!({ "challenge": step_challenge() }))).await;
        advance_step_challenge_attempt(&db, props()).await.unwrap();
        let data = attributes(&db).await["challenge"]["data"].clone();
        assert_eq!(data["attempts"], json!(2));
        assert_eq!(data["lastAttemptAt"], json!("2026-11-09T10:00:00.000Z"));
        assert_eq!(data["startedAt"], json!("2026-11-09T08:00:00.000Z"));
    }
}
