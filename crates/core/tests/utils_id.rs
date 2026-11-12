//! Port of `packages/utils/__tests__/id.test.ts`. Fake timers become an
//! injected `TestClock`; `Math.random` stand-ins are `random` closures; the
//! env argument is an `Env` map.

mod support;

use std::collections::HashSet;
use std::sync::Arc;

use chatbotx_core::utils::id::{
    SNOWFLAKE_PLACE_ID_MAX, SnowflakeGenerator, create_id, resolve_id, resolve_snowflake_place_id,
};
use chatbotx_core::{Clock, TestClock};
use regex::Regex;

const SEQUENCE_SPACE: usize = 1024;

fn frozen_now() -> i64 {
    support::iso_ms("2026-09-24T02:25:38.515Z")
}

fn constant_random(value: f64) -> Box<dyn FnMut() -> f64 + Send> {
    Box::new(move || value)
}

fn generator(
    place_id: f64,
    random: Box<dyn FnMut() -> f64 + Send>,
    clock: &Arc<TestClock>,
) -> SnowflakeGenerator {
    SnowflakeGenerator::new(place_id, random, clock.clone() as Arc<dyn Clock>).unwrap()
}

fn big(id: &str) -> u128 {
    id.parse().unwrap()
}

/// Port of: packages/utils/__tests__/id.test.ts:32 "createId > decodes with resolveId to the generation time under the legacy uuniq layout"
#[test]
fn createid_decodes_with_resolveid_to_the_generation_time_under_the_legacy_uuniq_layout() {
    let before = support::now_ms();
    let id = create_id();
    let after = support::now_ms();

    let resolved = resolve_id(&id).unwrap();
    let decoded_ms = support::iso_ms(&resolved.created_at);

    assert!(Regex::new(r"^\d+$").unwrap().is_match(&id));
    assert!(decoded_ms >= before);
    assert!(decoded_ms <= after);
    assert!(resolved.place_id >= 0);
    assert!(resolved.place_id <= SNOWFLAKE_PLACE_ID_MAX);
}

/// Port of: packages/utils/__tests__/id.test.ts:47 "createId > stays unique and strictly increasing across a burst larger than the per-millisecond sequence space"
#[test]
fn createid_stays_unique_and_strictly_increasing_across_a_burst_larger_than_the_per_millisecond_sequence_space()
 {
    let ids: Vec<String> = (0..3000).map(|_| create_id()).collect();

    assert_eq!(ids.iter().collect::<HashSet<_>>().len(), 3000);
    for i in 1..ids.len() {
        assert!(big(&ids[i]) > big(&ids[i - 1]));
    }
}

/// Port of: packages/utils/__tests__/id.test.ts:61 "createSnowflakeGenerator > encodes the place id in the 4-bit field the legacy decoder reads"
#[test]
fn createsnowflakegenerator_encodes_the_place_id_in_the_4_bit_field_the_legacy_decoder_reads() {
    let clock = Arc::new(TestClock::new(frozen_now()));
    let generator = generator(3.0, constant_random(0.0), &clock);

    let resolved = resolve_id(&generator.generate()).unwrap();

    assert_eq!(resolved.place_id, 3);
    assert_eq!(resolved.sequence, 0);
    assert_eq!(resolved.created_at, support::iso(frozen_now()));
}

/// Port of: packages/utils/__tests__/id.test.ts:76 "createSnowflakeGenerator > two processes with different place ids never collide in the same millisecond"
#[test]
fn createsnowflakegenerator_two_processes_with_different_place_ids_never_collide_in_the_same_millisecond()
 {
    let clock = Arc::new(TestClock::new(frozen_now()));
    let worker_a = generator(3.0, constant_random(0.0), &clock);
    let worker_b = generator(7.0, constant_random(0.0), &clock);

    let ids_a: HashSet<String> = (0..200).map(|_| worker_a.generate()).collect();
    let ids_b: HashSet<String> = (0..200).map(|_| worker_b.generate()).collect();

    assert_eq!(ids_a.intersection(&ids_b).count(), 0);
}

/// Port of: packages/utils/__tests__/id.test.ts:95 "createSnowflakeGenerator > two processes sharing a place id are separated by their per-millisecond sequence offset"
#[test]
fn createsnowflakegenerator_two_processes_sharing_a_place_id_are_separated_by_their_per_millisecond_sequence_offset()
 {
    let clock = Arc::new(TestClock::new(frozen_now()));
    let legacy_like = generator(0.0, constant_random(0.0), &clock);
    let upgraded = generator(0.0, constant_random(0.5), &clock);

    let legacy_ids: HashSet<String> = (0..100).map(|_| legacy_like.generate()).collect();
    let upgraded_ids: Vec<String> = (0..100).map(|_| upgraded.generate()).collect();

    assert!(upgraded_ids.iter().all(|id| !legacy_ids.contains(id)));
    assert_eq!(resolve_id(&upgraded_ids[0]).unwrap().sequence, 512);
}

/// Port of: packages/utils/__tests__/id.test.ts:116 "createSnowflakeGenerator > starts each new millisecond at the sequence offset drawn from random"
#[test]
fn createsnowflakegenerator_starts_each_new_millisecond_at_the_sequence_offset_drawn_from_random() {
    let clock = Arc::new(TestClock::new(frozen_now()));
    let mut draws = vec![0.25, 0.75].into_iter();
    let generator = generator(1.0, Box::new(move || draws.next().unwrap_or(0.0)), &clock);

    let first = resolve_id(&generator.generate()).unwrap().sequence;
    clock.advance(1);
    let second = resolve_id(&generator.generate()).unwrap().sequence;

    assert_eq!(first, 256);
    assert_eq!(second, 768);
}

/// Port of: packages/utils/__tests__/id.test.ts:133 "createSnowflakeGenerator > never wraps the sequence inside a millisecond, so ids stay strictly increasing"
#[test]
fn createsnowflakegenerator_never_wraps_the_sequence_inside_a_millisecond_so_ids_stay_strictly_increasing()
 {
    let clock = Arc::new(TestClock::new(frozen_now()));
    // Offset 1013 leaves room for 11 ids in the first slot.
    let generator = generator(0.0, constant_random(0.99), &clock);

    let ids: Vec<String> = (0..30).map(|_| generator.generate()).collect();

    for i in 1..ids.len() {
        assert!(big(&ids[i]) > big(&ids[i - 1]));
    }
    assert_eq!(resolve_id(&ids[10]).unwrap().sequence, 1023);
    assert_eq!(
        resolve_id(&ids[10]).unwrap().created_at,
        support::iso(frozen_now())
    );
    assert_eq!(resolve_id(&ids[11]).unwrap().sequence, 1013);
    assert_eq!(
        support::iso_ms(&resolve_id(&ids[11]).unwrap().created_at),
        frozen_now() + 1
    );
}

/// Port of: packages/utils/__tests__/id.test.ts:155 "createSnowflakeGenerator > exhausting the sequence in one millisecond advances a logical clock instead of blocking"
#[test]
fn createsnowflakegenerator_exhausting_the_sequence_in_one_millisecond_advances_a_logical_clock_instead_of_blocking()
 {
    let clock = Arc::new(TestClock::new(frozen_now()));
    let generator = generator(0.0, constant_random(0.0), &clock);

    for _ in 0..SEQUENCE_SPACE {
        generator.generate();
    }
    // Wall clock is frozen; a busy-wait here would never return.
    let overflow = resolve_id(&generator.generate()).unwrap();

    assert_eq!(support::iso_ms(&overflow.created_at), frozen_now() + 1);
    assert_eq!(overflow.sequence, 0);
}

/// Port of: packages/utils/__tests__/id.test.ts:173 "createSnowflakeGenerator > keeps ids strictly increasing when the wall clock steps backwards"
#[test]
fn createsnowflakegenerator_keeps_ids_strictly_increasing_when_the_wall_clock_steps_backwards() {
    let clock = Arc::new(TestClock::new(frozen_now()));
    let generator = generator(0.0, constant_random(0.0), &clock);

    let before_rollback = generator.generate();
    clock.set(frozen_now() - 5000);
    let after_rollback = generator.generate();

    assert!(big(&after_rollback) > big(&before_rollback));
    assert_eq!(
        resolve_id(&after_rollback).unwrap().created_at,
        support::iso(frozen_now())
    );
}

/// Port of: packages/utils/__tests__/id.test.ts:189 "createSnowflakeGenerator > rejects a place id outside the 4-bit field"
#[test]
fn createsnowflakegenerator_rejects_a_place_id_outside_the_4_bit_field() {
    let clock: Arc<dyn Clock> = Arc::new(TestClock::new(0));
    let place_id_error = Regex::new("(?i)place id").unwrap();
    for place_id in [(SNOWFLAKE_PLACE_ID_MAX + 1) as f64, -1.0, 1.5] {
        let err =
            SnowflakeGenerator::new(place_id, constant_random(0.0), clock.clone()).unwrap_err();
        assert!(place_id_error.is_match(&err.to_string()), "{err}");
    }
}

/// Port of: packages/utils/__tests__/id.test.ts:203 "resolveSnowflakePlaceId > uses SNOWFLAKE_PLACE_ID when it is a valid integer in range"
#[test]
fn resolvesnowflakeplaceid_uses_snowflake_place_id_when_it_is_a_valid_integer_in_range() {
    let mut random = || 0.0;
    assert_eq!(
        resolve_snowflake_place_id(&[("SNOWFLAKE_PLACE_ID", "9")], &mut random),
        9
    );
    assert_eq!(
        resolve_snowflake_place_id(&[("SNOWFLAKE_PLACE_ID", "0")], &mut random),
        0
    );
    assert_eq!(
        resolve_snowflake_place_id(&[("SNOWFLAKE_PLACE_ID", "15")], &mut random),
        15
    );
}

/// Port of: packages/utils/__tests__/id.test.ts:209 "resolveSnowflakePlaceId > never draws the legacy place id 0 when falling back to random, so a rolling deploy cannot overlap old processes"
#[test]
fn resolvesnowflakeplaceid_never_draws_the_legacy_place_id_0_when_falling_back_to_random_so_a_rolling_deploy_cannot_overlap_old_processes()
 {
    let mut drawn = HashSet::new();
    let mut step = 0.0f64;
    while step < 1.0 {
        let value = step;
        drawn.insert(resolve_snowflake_place_id(
            &chatbotx_core::env::EmptyEnv,
            &mut || value,
        ));
        step += 1.0 / 64.0;
    }
    drawn.insert(resolve_snowflake_place_id(
        &chatbotx_core::env::EmptyEnv,
        &mut || 0.999_999,
    ));

    assert!(!drawn.contains(&0));
    assert_eq!(*drawn.iter().min().unwrap(), 1);
    assert_eq!(*drawn.iter().max().unwrap(), SNOWFLAKE_PLACE_ID_MAX);
    assert_eq!(drawn.len() as i64, SNOWFLAKE_PLACE_ID_MAX);
}

/// Port of: packages/utils/__tests__/id.test.ts:222 "resolveSnowflakePlaceId > ignores a missing or invalid env value and falls back to random"
#[test]
fn resolvesnowflakeplaceid_ignores_a_missing_or_invalid_env_value_and_falls_back_to_random() {
    let envs: [&[(&str, &str)]; 6] = [
        &[],
        &[("SNOWFLAKE_PLACE_ID", "")],
        &[("SNOWFLAKE_PLACE_ID", "abc")],
        &[("SNOWFLAKE_PLACE_ID", "16")],
        &[("SNOWFLAKE_PLACE_ID", "-1")],
        &[("SNOWFLAKE_PLACE_ID", "1.5")],
    ];
    for env in envs {
        assert_eq!(resolve_snowflake_place_id(&env, &mut || 0.5), 8);
    }
}
