//! Port of `apps/worker/__tests__/scheduler-tick-interval-env.test.ts`
//! (spec 5.3 "scheduler-tick-interval-env"): the strict parse of
//! `SCHEDULER_TICK_INTERVAL_MS` over an env map (no global env mutation).

use std::collections::BTreeMap;

use chatbotx_jobs::sequence::tick::{
    DEFAULT_SCHEDULER_TICK_INTERVAL_MS, scheduler_tick_interval_from_env,
};

fn env_with(tick_interval: Option<&str>) -> BTreeMap<String, String> {
    let mut env = BTreeMap::new();
    if let Some(value) = tick_interval {
        env.insert("SCHEDULER_TICK_INTERVAL_MS".to_owned(), value.to_owned());
    }
    env
}

fn assert_rejects(value: &str) {
    let result = scheduler_tick_interval_from_env(&env_with(Some(value)));
    assert!(
        result.is_err(),
        "{value:?} must be rejected, got {result:?}"
    );
}

/// Port of: apps/worker/__tests__/scheduler-tick-interval-env.test.ts:31 "SCHEDULER_TICK_INTERVAL_MS > defaults to 5000 ms when unset"
#[test]
fn defaults_to_5000_ms_when_unset() {
    assert_eq!(scheduler_tick_interval_from_env(&env_with(None)), Ok(5000));
    assert_eq!(DEFAULT_SCHEDULER_TICK_INTERVAL_MS, 5000);
}

/// Port of: apps/worker/__tests__/scheduler-tick-interval-env.test.ts:39 "SCHEDULER_TICK_INTERVAL_MS > coerces a numeric string"
#[test]
fn coerces_a_numeric_string() {
    assert_eq!(
        scheduler_tick_interval_from_env(&env_with(Some("2500"))),
        Ok(2500)
    );
}

/// Port of: apps/worker/__tests__/scheduler-tick-interval-env.test.ts:45 "SCHEDULER_TICK_INTERVAL_MS > accepts the bound 100"
#[test]
fn accepts_the_bound_100() {
    assert_eq!(
        scheduler_tick_interval_from_env(&env_with(Some("100"))),
        Ok(100)
    );
}

/// Port of: apps/worker/__tests__/scheduler-tick-interval-env.test.ts:45 "SCHEDULER_TICK_INTERVAL_MS > accepts the bound 10000"
#[test]
fn accepts_the_bound_10000() {
    assert_eq!(
        scheduler_tick_interval_from_env(&env_with(Some("10000"))),
        Ok(10_000)
    );
}

/// Port of: apps/worker/__tests__/scheduler-tick-interval-env.test.ts:51 "SCHEDULER_TICK_INTERVAL_MS > rejects 99"
#[test]
fn rejects_99() {
    assert_rejects("99");
}

/// Port of: apps/worker/__tests__/scheduler-tick-interval-env.test.ts:51 "SCHEDULER_TICK_INTERVAL_MS > rejects 0"
#[test]
fn rejects_0() {
    assert_rejects("0");
}

/// Port of: apps/worker/__tests__/scheduler-tick-interval-env.test.ts:51 "SCHEDULER_TICK_INTERVAL_MS > rejects -500"
#[test]
fn rejects_minus_500() {
    assert_rejects("-500");
}

/// Port of: apps/worker/__tests__/scheduler-tick-interval-env.test.ts:51 "SCHEDULER_TICK_INTERVAL_MS > rejects 10001"
#[test]
fn rejects_10001() {
    assert_rejects("10001");
}

/// Port of: apps/worker/__tests__/scheduler-tick-interval-env.test.ts:51 "SCHEDULER_TICK_INTERVAL_MS > rejects abc"
#[test]
fn rejects_abc() {
    assert_rejects("abc");
}

/// Port of: apps/worker/__tests__/scheduler-tick-interval-env.test.ts:51 "SCHEDULER_TICK_INTERVAL_MS > rejects 1000.5"
#[test]
fn rejects_1000_5() {
    assert_rejects("1000.5");
}
