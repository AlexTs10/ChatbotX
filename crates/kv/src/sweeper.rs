//! The two sweepers (spec 2.3, 2.9, 2.12): one task for the volatile stores,
//! one for `kv_entry`. They only free space; every read already treats an
//! expired entry as absent.
//!
//! A sweeper sleeps until `max(earliest expiry, last sweep + min interval)`.
//! With no entry that can expire it has no timer and waits only on its
//! [`SweepSignal`]. A write notifies the signal only when its expiry would
//! move the planned wake-up earlier, so a busy store does not wake it.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::time::Duration;

use tokio::sync::Notify;

use crate::BoxFuture;
use crate::clock::Clock;

/// Volatile stores: at most one sweep per 60 s (spec 2.3).
pub(crate) const VOLATILE_MIN_INTERVAL_MS: i64 = 60_000;
/// `kv_entry`: at most one sweep per 300 s (spec 2.9).
pub(crate) const DURABLE_MIN_INTERVAL_MS: i64 = 300_000;

/// `planned_ms` value while no sweeper listens: writes never notify.
const NO_SWEEPER: i64 = i64::MIN;

/// Shared between the stores of one sweeper and the sweeper task.
pub(crate) struct SweepSignal {
    notify: Notify,
    /// The planned wake-up (unix ms); `i64::MAX` = no timer.
    planned_ms: AtomicI64,
    /// The earliest allowed sweep (last sweep + min interval).
    floor_ms: AtomicI64,
    closed: AtomicBool,
}

impl SweepSignal {
    pub(crate) fn new() -> Arc<SweepSignal> {
        Arc::new(SweepSignal {
            notify: Notify::new(),
            planned_ms: AtomicI64::new(NO_SWEEPER),
            floor_ms: AtomicI64::new(i64::MIN),
            closed: AtomicBool::new(false),
        })
    }

    /// Called after a write that stores an entry expiring at `expires_at_ms`.
    pub(crate) fn note_expiry(&self, expires_at_ms: i64) {
        let planned = self.planned_ms.load(Ordering::SeqCst);
        if planned == NO_SWEEPER {
            return;
        }
        let floor = self.floor_ms.load(Ordering::SeqCst);
        if expires_at_ms.max(floor) < planned {
            self.notify.notify_one();
        }
    }

    /// Stops the sweeper at its next wake-up (the `Kv` handle is gone).
    pub(crate) fn close(&self) {
        self.closed.store(true, Ordering::SeqCst);
        self.notify.notify_one();
    }

    fn is_closed(&self) -> bool {
        self.closed.load(Ordering::SeqCst)
    }

    #[cfg(test)]
    pub(crate) fn planned_ms(&self) -> i64 {
        self.planned_ms.load(Ordering::SeqCst)
    }
}

/// The stores one sweeper cleans.
pub(crate) trait SweepTarget: Send + Sync + 'static {
    /// The earliest stored expiry (`Some(None)` = nothing expires);
    /// `None` when the stores are gone and the sweeper must stop.
    fn next_expiry(&self) -> BoxFuture<'_, Option<Option<i64>>>;

    /// Removes expired entries; `false` when the stores are gone.
    fn sweep(&self) -> BoxFuture<'_, bool>;
}

/// The wake-up rule: `max(earliest, floor)`; no timer without an expiry.
pub(crate) fn plan_wake(earliest: Option<i64>, floor: i64) -> Option<i64> {
    earliest.map(|e| e.max(floor))
}

/// The sweeper loop. Ends when the target is gone or the signal is closed.
pub(crate) async fn run_sweeper(
    target: Arc<dyn SweepTarget>,
    signal: Arc<SweepSignal>,
    clock: Arc<dyn Clock>,
    min_interval_ms: i64,
) {
    let mut last_sweep = clock.now_ms();
    loop {
        if signal.is_closed() {
            return;
        }
        let floor = last_sweep.saturating_add(min_interval_ms);
        signal.floor_ms.store(floor, Ordering::SeqCst);
        // While planning, every write with an expiry notifies (no lost wake-up).
        signal.planned_ms.store(i64::MAX, Ordering::SeqCst);
        let Some(earliest) = target.next_expiry().await else {
            return;
        };
        let wake = plan_wake(earliest, floor);
        signal
            .planned_ms
            .store(wake.unwrap_or(i64::MAX), Ordering::SeqCst);
        match wake {
            None => signal.notify.notified().await,
            Some(wake) => {
                let delay = wake.saturating_sub(clock.now_ms());
                if delay > 0 {
                    let sleep = tokio::time::sleep(Duration::from_millis(delay.unsigned_abs()));
                    tokio::select! {
                        () = sleep => {}
                        () = signal.notify.notified() => continue,
                    }
                }
                if signal.is_closed() || !target.sweep().await {
                    return;
                }
                last_sweep = clock.now_ms();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;
    use std::sync::atomic::AtomicUsize;

    use super::*;
    use crate::mem::MemKv;

    /// Unix ms that follow the (paused) tokio clock.
    struct TokioClock {
        base: i64,
        start: tokio::time::Instant,
    }

    impl Clock for TokioClock {
        fn now_ms(&self) -> i64 {
            self.base + i64::try_from(self.start.elapsed().as_millis()).unwrap_or(i64::MAX)
        }
    }

    struct MemTarget {
        mem: Arc<MemKv>,
        sweeps: AtomicUsize,
        sweep_times: Mutex<Vec<i64>>,
        clock: Arc<dyn Clock>,
    }

    impl SweepTarget for MemTarget {
        fn next_expiry(&self) -> BoxFuture<'_, Option<Option<i64>>> {
            Box::pin(async move { Some(self.mem.next_expiry()) })
        }

        fn sweep(&self) -> BoxFuture<'_, bool> {
            Box::pin(async move {
                self.sweeps.fetch_add(1, Ordering::SeqCst);
                self.sweep_times.lock().unwrap().push(self.clock.now_ms());
                self.mem.sweep_expired();
                true
            })
        }
    }

    fn setup() -> (Arc<MemTarget>, Arc<SweepSignal>, Arc<dyn Clock>) {
        let clock: Arc<dyn Clock> = Arc::new(TokioClock {
            base: 1_000_000,
            start: tokio::time::Instant::now(),
        });
        let signal = SweepSignal::new();
        let mem = Arc::new(MemKv::build(
            Arc::clone(&clock),
            crate::MemKvConfig::default(),
            Arc::clone(&signal),
        ));
        let target = Arc::new(MemTarget {
            mem,
            sweeps: AtomicUsize::new(0),
            sweep_times: Mutex::new(Vec::new()),
            clock: Arc::clone(&clock),
        });
        (target, signal, clock)
    }

    #[test]
    fn plan_wake_never_sweeps_before_the_floor_and_has_no_timer_when_empty() {
        assert_eq!(plan_wake(None, 60_000), None);
        assert_eq!(plan_wake(Some(10), 60_000), Some(60_000));
        assert_eq!(plan_wake(Some(90_000), 60_000), Some(90_000));
    }

    #[test]
    fn writes_do_not_notify_without_a_sweeper_or_when_the_plan_is_earlier() {
        let signal = SweepSignal::new();
        signal.note_expiry(5); // no sweeper: no permit stored
        signal.floor_ms.store(60_000, Ordering::SeqCst);
        signal.planned_ms.store(60_000, Ordering::SeqCst);
        // An expiry before the floor cannot move the wake-up earlier.
        signal.note_expiry(10);
        signal.planned_ms.store(120_000, Ordering::SeqCst);
        // An expiry after the plan cannot either.
        signal.note_expiry(200_000);
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .start_paused(true)
            .build()
            .unwrap();
        rt.block_on(async {
            let notified =
                tokio::time::timeout(Duration::from_millis(1), signal.notify.notified()).await;
            assert!(notified.is_err(), "no permit may be stored");
            // An expiry between floor and plan moves it: notify.
            signal.note_expiry(90_000);
            tokio::time::timeout(Duration::from_millis(1), signal.notify.notified())
                .await
                .expect("permit stored");
        });
    }

    #[tokio::test(start_paused = true)]
    async fn an_idle_sweeper_has_no_timer_and_a_busy_one_sweeps_at_most_once_per_interval() {
        let (target, signal, clock) = setup();
        let task = tokio::spawn(run_sweeper(
            Arc::clone(&target) as Arc<dyn SweepTarget>,
            Arc::clone(&signal),
            Arc::clone(&clock),
            VOLATILE_MIN_INTERVAL_MS,
        ));

        // Empty store: one hour passes with no sweep and no timer.
        tokio::time::sleep(Duration::from_secs(3600)).await;
        assert_eq!(target.sweeps.load(Ordering::SeqCst), 0);
        assert_eq!(signal.planned_ms(), i64::MAX);

        // A 1 s entry wakes the idle sweeper once; it plans the sweep at the
        // expiry (the 60 s floor, counted from the sweeper start, is past).
        let started = clock.now_ms();
        target.mem.put("short", &1, Some(1)).unwrap();
        tokio::time::sleep(Duration::from_millis(10)).await;
        assert_eq!(signal.planned_ms(), started + 1000);
        tokio::time::sleep(Duration::from_secs(2)).await;
        assert_eq!(target.sweeps.load(Ordering::SeqCst), 1);
        assert_eq!(target.mem.stored_len(), 0);

        // Many short entries: sweeps stay >= 60 s apart.
        for i in 0..100 {
            target.mem.put(&format!("k{i}"), &i, Some(1)).unwrap();
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
        tokio::time::sleep(Duration::from_secs(120)).await;
        let times = target.sweep_times.lock().unwrap().clone();
        assert!(times.len() >= 2, "{times:?}");
        for pair in times.windows(2) {
            assert!(pair[1] - pair[0] >= VOLATILE_MIN_INTERVAL_MS, "{times:?}");
        }
        assert_eq!(target.mem.stored_len(), 0);
        // Idle again: no timer.
        tokio::time::sleep(Duration::from_millis(10)).await;
        assert_eq!(signal.planned_ms(), i64::MAX);

        signal.close();
        task.await.unwrap();
    }
}
