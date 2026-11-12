//! Port of `packages/utils/__tests__/concurrency.test.ts`. Promises become
//! futures on a current-thread tokio runtime; fake timers become
//! `tokio::time::pause` (auto-advance) or real short sleeps as in the
//! original; `AbortController` is the crate's own.

use std::cell::{Cell, RefCell};
use std::time::Duration;

use chatbotx_core::utils::concurrency::{
    AbortController, ConcurrencyLimiter, LimitError, map_with_concurrency, with_timeout,
};
use tokio::sync::oneshot;

async fn sleep_ms(ms: u64) {
    tokio::time::sleep(Duration::from_millis(ms)).await;
}

/// Port of: packages/utils/__tests__/concurrency.test.ts:9 "mapWithConcurrency > respects the concurrency cap"
#[tokio::test]
async fn mapwithconcurrency_respects_the_concurrency_cap() {
    let active = Cell::new(0usize);
    let max_active = Cell::new(0usize);
    let items: Vec<usize> = (0..10).collect();

    map_with_concurrency(items, 3, |item, _| {
        let active = &active;
        let max_active = &max_active;
        async move {
            active.set(active.get() + 1);
            max_active.set(max_active.get().max(active.get()));
            sleep_ms(5).await;
            active.set(active.get() - 1);
            Ok::<_, String>(item)
        }
    })
    .await;

    assert!(max_active.get() <= 3);
    assert!(max_active.get() > 1);
}

/// Port of: packages/utils/__tests__/concurrency.test.ts:26 "mapWithConcurrency > preserves per-item results in input order"
#[tokio::test]
async fn mapwithconcurrency_preserves_per_item_results_in_input_order() {
    let results = map_with_concurrency(vec![3u64, 1, 2], 2, |item, _| async move {
        sleep_ms(item).await;
        Ok::<_, String>(item * 10)
    })
    .await;

    assert_eq!(results, vec![Ok(30), Ok(10), Ok(20)]);
}

/// Port of: packages/utils/__tests__/concurrency.test.ts:41 "mapWithConcurrency > isolates per-item failures instead of aborting the others"
#[tokio::test]
async fn mapwithconcurrency_isolates_per_item_failures_instead_of_aborting_the_others() {
    let results = map_with_concurrency(vec!["ok-1", "fail", "ok-2"], 2, |item, _| async move {
        if item == "fail" {
            Err("boom".to_owned())
        } else {
            Ok(item)
        }
    })
    .await;

    assert_eq!(results[0], Ok("ok-1"));
    assert!(results[1].is_err());
    assert_eq!(results[2], Ok("ok-2"));
}

/// Port of: packages/utils/__tests__/concurrency.test.ts:56 "mapWithConcurrency > returns [] for an empty input without calling fn"
#[tokio::test]
async fn mapwithconcurrency_returns_for_an_empty_input_without_calling_fn() {
    let called = Cell::new(false);

    let results = map_with_concurrency(Vec::<u32>::new(), 5, |item, _| {
        called.set(true);
        async move { Ok::<_, String>(item) }
    })
    .await;

    assert!(results.is_empty());
    assert!(!called.get());
}

/// Port of: packages/utils/__tests__/concurrency.test.ts:68 "mapWithConcurrency > never runs more concurrently than the item count"
#[tokio::test]
async fn mapwithconcurrency_never_runs_more_concurrently_than_the_item_count() {
    let active = Cell::new(0usize);
    let max_active = Cell::new(0usize);

    map_with_concurrency(vec![1, 2], 10, |item, _| {
        let active = &active;
        let max_active = &max_active;
        async move {
            active.set(active.get() + 1);
            max_active.set(max_active.get().max(active.get()));
            sleep_ms(5).await;
            active.set(active.get() - 1);
            Ok::<_, String>(item)
        }
    })
    .await;

    assert_eq!(max_active.get(), 2);
}

/// Port of: packages/utils/__tests__/concurrency.test.ts:89 "withTimeout > resolves with the value and leaves no timer behind when the work is fast"
#[tokio::test(start_paused = true)]
async fn withtimeout_resolves_with_the_value_and_leaves_no_timer_behind_when_the_work_is_fast() {
    // `vi.getTimerCount()` has no tokio counterpart: `tokio::time::timeout`
    // drops its timer with the future.
    assert_eq!(with_timeout(async { "ok" }, 250).await, Ok("ok"));
}

/// Port of: packages/utils/__tests__/concurrency.test.ts:96 "withTimeout > rejects once the deadline passes while the work is still pending"
#[tokio::test(start_paused = true)]
async fn withtimeout_rejects_once_the_deadline_passes_while_the_work_is_still_pending() {
    let outcome = with_timeout(std::future::pending::<()>(), 250).await;

    assert_eq!(outcome.unwrap_err().to_string(), "Timed out after 250ms");
}

/// Port of: packages/utils/__tests__/concurrency.test.ts:109 "withTimeout > passes the work's own rejection through"
#[tokio::test]
async fn withtimeout_passes_the_work_s_own_rejection_through() {
    let outcome = with_timeout(async { Err::<(), _>("db down".to_owned()) }, 250).await;

    assert_eq!(outcome, Ok(Err("db down".to_owned())));
}

/// Port of: packages/utils/__tests__/concurrency.test.ts:115 "withTimeout > handles a rejection that arrives after the deadline"
#[tokio::test(start_paused = true)]
async fn withtimeout_handles_a_rejection_that_arrives_after_the_deadline() {
    let (reject_late, late) = oneshot::channel::<String>();
    let outcome = with_timeout(late, 100).await;

    assert_eq!(outcome.unwrap_err().to_string(), "Timed out after 100ms");
    // The work was dropped at the deadline: a late rejection goes nowhere.
    assert!(reject_late.send("too late".to_owned()).is_err());
    sleep_ms(1).await;
}

/// Port of: packages/utils/__tests__/concurrency.test.ts:136 "createConcurrencyLimiter > caps concurrent work across every caller"
#[tokio::test]
async fn createconcurrencylimiter_caps_concurrent_work_across_every_caller() {
    let run = ConcurrencyLimiter::new(2);
    let active = Cell::new(0usize);
    let max_active = Cell::new(0usize);
    let work = || async {
        active.set(active.get() + 1);
        max_active.set(max_active.get().max(active.get()));
        sleep_ms(1).await;
        active.set(active.get() - 1);
        Ok::<_, String>(())
    };

    let runs: Vec<_> = (0..6).map(|_| run.run(work, None)).collect();
    futures::future::join_all(runs).await;

    assert_eq!(max_active.get(), 2);
}

/// Port of: packages/utils/__tests__/concurrency.test.ts:152 "createConcurrencyLimiter > starts work at once, in the caller's turn, when a slot is free"
#[tokio::test]
async fn createconcurrencylimiter_starts_work_at_once_in_the_caller_s_turn_when_a_slot_is_free() {
    let run = ConcurrencyLimiter::new(1);
    let order = RefCell::new(Vec::new());

    let done = run.run(
        || {
            order.borrow_mut().push("work");
            async { Ok::<_, String>(()) }
        },
        None,
    );
    order.borrow_mut().push("caller");
    done.await.unwrap();

    assert_eq!(*order.borrow(), vec!["work", "caller"]);
}

/// Port of: packages/utils/__tests__/concurrency.test.ts:166 "createConcurrencyLimiter > starts waiting work in order"
#[tokio::test]
async fn createconcurrencylimiter_starts_waiting_work_in_order() {
    let run = ConcurrencyLimiter::new(1);
    let order = RefCell::new(Vec::new());

    let runs: Vec<_> = [1, 2, 3]
        .into_iter()
        .map(|item| {
            let order = &order;
            run.run(
                move || async move {
                    order.borrow_mut().push(item);
                    sleep_ms(1).await;
                    Ok::<_, String>(())
                },
                None,
            )
        })
        .collect();
    futures::future::join_all(runs).await;

    assert_eq!(*order.borrow(), vec![1, 2, 3]);
}

/// Port of: packages/utils/__tests__/concurrency.test.ts:182 "createConcurrencyLimiter > frees the slot when work throws"
#[tokio::test]
async fn createconcurrencylimiter_frees_the_slot_when_work_throws() {
    let run = ConcurrencyLimiter::new(1);

    let failed = run
        .run(|| async { Err::<(), _>("boom".to_owned()) }, None)
        .await;
    assert_eq!(failed.unwrap_err().to_string(), "boom");
    assert_eq!(
        run.run(|| async { Ok::<_, String>("next") }, None).await,
        Ok("next")
    );
}

/// Port of: packages/utils/__tests__/concurrency.test.ts:191 "createConcurrencyLimiter > a caller that gives up while waiting leaves the queue without a slot"
#[tokio::test]
async fn createconcurrencylimiter_a_caller_that_gives_up_while_waiting_leaves_the_queue_without_a_slot()
 {
    let run = ConcurrencyLimiter::new(1);
    let (release_first, first_gate) = oneshot::channel::<()>();
    let first = run.run(
        move || async move {
            let _ = first_gate.await;
            Ok::<_, String>(())
        },
        None,
    );
    let controller = AbortController::new();
    let gave_up = run.run(
        || async { Ok::<_, String>("never") },
        Some(controller.signal()),
    );

    controller.abort("deadline");

    let gave_up = gave_up.await;
    assert_eq!(gave_up.clone().unwrap_err().to_string(), "deadline");
    assert!(matches!(gave_up, Err(LimitError::Aborted(_))));
    release_first.send(()).unwrap();
    first.await.unwrap();
    assert_eq!(
        run.run(|| async { Ok::<_, String>("next") }, None).await,
        Ok("next")
    );
}

/// Port of: packages/utils/__tests__/concurrency.test.ts:208 "createConcurrencyLimiter > a signal already aborted rejects at once when the cap is reached"
#[tokio::test]
async fn createconcurrencylimiter_a_signal_already_aborted_rejects_at_once_when_the_cap_is_reached()
{
    let run = ConcurrencyLimiter::new(1);
    let (release_first, first_gate) = oneshot::channel::<()>();
    let first = run.run(
        move || async move {
            let _ = first_gate.await;
            Ok::<_, String>(())
        },
        None,
    );
    let controller = AbortController::new();
    controller.abort("late");

    let result = run
        .run(
            || async { Ok::<_, String>("never") },
            Some(controller.signal()),
        )
        .await;
    assert_eq!(result.unwrap_err().to_string(), "late");
    release_first.send(()).unwrap();
    first.await.unwrap();
}
