//! Port of `packages/database/__tests__/integration/ai-handover-bulk-eligibility.test.ts`
//! (P3.2b). Runs on a migrated temp file instead of a real Postgres (ported): the TEMP
//! `"ContactInbox"` table shadows the real one, as in the TS suite, and is dropped after
//! each read instead of a rolled-back transaction.

mod common;
#[path = "common/repo12.rs"]
mod repo12;

use chatbotx_db::repo::queries::ai_handover_bulk_eligibility::{
    AiHandoverBulkAction as Action, BulkEligibilityInput, bulk_eligibility_conditions,
};
use chatbotx_db::repo::sql::Sql;
use repo12::{DAY_MS, HOUR_MS, NOW, REQUESTED_AT, Thread, hours_ago, shadow_eligible, thread};

fn threads() -> Vec<(&'static str, Thread)> {
    let standby_ai = Thread {
        state: Some("standby"),
        role: Some("ai_agent"),
        ..thread()
    };
    vec![
        ("never-observed-active", thread()),
        (
            "never-observed-dormant",
            Thread {
                last_incoming_at: Some(hours_ago(24 * 40)),
                ..thread()
            },
        ),
        (
            "owned",
            Thread {
                state: Some("owned"),
                updated_at: Some(hours_ago(3)),
                ..thread()
            },
        ),
        (
            "idle",
            Thread {
                state: Some("idle"),
                updated_at: Some(hours_ago(30)),
                ..thread()
            },
        ),
        (
            "ai-held",
            Thread {
                updated_at: Some(hours_ago(2)),
                ..standby_ai
            },
        ),
        (
            "ai-held-by-channel-expiry",
            Thread {
                expires_at: Some(NOW + HOUR_MS),
                updated_at: Some(hours_ago(50)),
                last_incoming_at: Some(hours_ago(60)),
                ..standby_ai
            },
        ),
        (
            "ai-standby-past-channel-expiry",
            Thread {
                expires_at: Some(hours_ago(1)),
                updated_at: Some(hours_ago(2)),
                ..standby_ai
            },
        ),
        (
            "ai-standby-idle-after-24h",
            Thread {
                updated_at: Some(hours_ago(30)),
                last_incoming_at: Some(hours_ago(30)),
                ..standby_ai
            },
        ),
        (
            "partner-held",
            Thread {
                role: Some("escalation"),
                updated_at: Some(hours_ago(2)),
                ..standby_ai
            },
        ),
        (
            "ai-held-silent-for-8-days",
            Thread {
                expires_at: Some(NOW + 99 * HOUR_MS),
                updated_at: Some(hours_ago(2)),
                last_incoming_at: Some(hours_ago(8 * 24)),
                ..standby_ai
            },
        ),
        (
            "changed-after-the-request",
            Thread {
                state: Some("owned"),
                updated_at: Some(REQUESTED_AT + 60_000),
                ..thread()
            },
        ),
    ]
}

async fn eligible(action: Action, window_ms: i64) -> Vec<String> {
    let db = common::fresh_db().await;
    let input = BulkEligibilityInput {
        action,
        requested_at: REQUESTED_AT,
        now: NOW,
        last_incoming_since: NOW - window_ms,
    };
    let conditions = Sql::and(bulk_eligibility_conditions(&input).into_iter().map(Some));
    shadow_eligible(&db, threads(), conditions).await
}

#[tokio::test]
async fn bulk_eligibility_conditions_on_postgres_enable_reaches_never_observed_owned_idle_and_lapsed_standby_threads_never_a_held_one()
 {
    assert_eq!(
        eligible(Action::Enable, 30 * DAY_MS).await,
        vec![
            "ai-standby-idle-after-24h",
            "ai-standby-past-channel-expiry",
            "idle",
            "never-observed-active",
            "owned",
        ]
    );
}

#[tokio::test]
async fn bulk_eligibility_conditions_on_postgres_disable_reaches_only_threads_the_ai_agent_holds_right_now_inside_the_7_day_window()
 {
    assert_eq!(
        eligible(Action::Disable, 7 * DAY_MS).await,
        vec!["ai-held", "ai-held-by-channel-expiry"]
    );
}
