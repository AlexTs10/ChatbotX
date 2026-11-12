//! Port of `packages/utils/__tests__/image-optimize-settings.test.ts`. The
//! logger mock is a `tracing` recorder; stubbed env vars are an env map; the
//! module-level encode slots are an `EncodeSlots` per test.

mod support;

use std::cell::Cell;

use chatbotx_core::env::EmptyEnv;
use chatbotx_core::utils::image_optimize_settings::{
    EncodeSlots, is_stored_image_optimization_enabled,
};
use chatbotx_core::utils::live_media::is_live_media_lazy_channel;
use support::RecordingLog;
use tokio::sync::oneshot;
use tracing::Level;

/// Port of: packages/utils/__tests__/image-optimize-settings.test.ts:20 "isStoredImageOptimizationEnabled > is on by default and off when switched off"
#[test]
fn isstoredimageoptimizationenabled_is_on_by_default_and_off_when_switched_off() {
    assert!(is_stored_image_optimization_enabled(&EmptyEnv));
    assert!(!is_stored_image_optimization_enabled(&[(
        "STORED_IMAGE_OPTIMIZATION",
        "off"
    )]));
}

/// Port of: packages/utils/__tests__/image-optimize-settings.test.ts:26 "isStoredImageOptimizationEnabled > treats a misconfigured value as off and says so, instead of throwing"
#[test]
fn isstoredimageoptimizationenabled_treats_a_misconfigured_value_as_off_and_says_so_instead_of_throwing()
 {
    let log = RecordingLog::install();
    assert!(!is_stored_image_optimization_enabled(&[(
        "STORED_IMAGE_OPTIMIZATION",
        "true"
    )]));
    let errors = log.at(Level::ERROR);
    assert_eq!(errors.len(), 1);
    assert!(errors[0].field("err").is_some());
    assert!(!errors[0].message.is_empty());
}

/// Port of: packages/utils/__tests__/image-optimize-settings.test.ts:36 "isStoredImageOptimizationEnabled > never breaks unrelated settings read on the webhook path"
#[test]
fn isstoredimageoptimizationenabled_never_breaks_unrelated_settings_read_on_the_webhook_path() {
    assert!(is_live_media_lazy_channel(
        &[("STORED_IMAGE_OPTIMIZATION", "true")],
        "messenger"
    ));
}

/// Port of: packages/utils/__tests__/image-optimize-settings.test.ts:51 "runInEncodeSlot > never runs more than the limit at once, and a waiting caller runs once a slot frees"
#[tokio::test]
async fn runinencodeslot_never_runs_more_than_the_limit_at_once_and_a_waiting_caller_runs_once_a_slot_frees()
 {
    let slots = EncodeSlots::new();
    let (release_first, first_gate) = oneshot::channel::<()>();
    let running = Cell::new(0usize);
    let peak = Cell::new(0usize);
    let enter = || {
        running.set(running.get() + 1);
        peak.set(peak.get().max(running.get()));
    };
    let leave = || running.set(running.get() - 1);

    let holder = slots.run_in_encode_slot(1, false, || async {
        enter();
        let _ = first_gate.await;
        leave();
        Ok::<_, String>("done")
    });
    let waiter = slots.run_in_encode_slot(1, true, || async {
        enter();
        leave();
        Ok::<_, String>("done")
    });
    let refused = slots
        .run_in_encode_slot(1, false, || async {
            enter();
            leave();
            Ok::<_, String>("done")
        })
        .await;
    release_first.send(()).unwrap();

    assert_eq!(refused, Ok(None));
    let (holder, waiter) = tokio::join!(holder, waiter);
    assert_eq!(holder, Ok(Some("done")));
    assert_eq!(waiter, Ok(Some("done")));
    assert_eq!(peak.get(), 1);
}

/// Port of: packages/utils/__tests__/image-optimize-settings.test.ts:83 "runInEncodeSlot > bounds how many callers may wait, so a backlog cannot pile up in memory"
#[tokio::test]
async fn runinencodeslot_bounds_how_many_callers_may_wait_so_a_backlog_cannot_pile_up_in_memory() {
    let slots = EncodeSlots::new();
    let (release, gate) = oneshot::channel::<()>();
    let holder = slots.run_in_encode_slot(1, false, || async {
        let _ = gate.await;
        Ok::<_, String>("held")
    });
    let waiters: Vec<_> = (0..4)
        .map(|_| slots.run_in_encode_slot(1, true, || async { Ok::<_, String>("ok") }))
        .collect();

    assert_eq!(
        slots
            .run_in_encode_slot(1, true, || async { Ok::<_, String>("late") })
            .await,
        Ok(None)
    );

    release.send(()).unwrap();
    let (held, results) = tokio::join!(holder, futures::future::join_all(waiters));
    assert_eq!(held, Ok(Some("held")));
    for result in results {
        assert_eq!(result, Ok(Some("ok")));
    }
}

/// Port of: packages/utils/__tests__/image-optimize-settings.test.ts:106 "runInEncodeSlot > frees its slot when the work throws"
#[tokio::test]
async fn runinencodeslot_frees_its_slot_when_the_work_throws() {
    let slots = EncodeSlots::new();
    let failed = slots
        .run_in_encode_slot(1, false, || async {
            Err::<&str, _>("encode failed".to_owned())
        })
        .await;
    assert_eq!(failed, Err("encode failed".to_owned()));
    assert_eq!(
        slots
            .run_in_encode_slot(1, false, || async { Ok::<_, String>("ok") })
            .await,
        Ok(Some("ok"))
    );
}
