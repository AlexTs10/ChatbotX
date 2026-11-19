//! The Graph usage throttle of the coexist pulls: the
//! `X-Business-Use-Case-Usage` reading of the Messenger sync
//! (`integration-messenger/apis/usage.ts`, `concurrencyForUsage`), the
//! `X-App-Usage` reading of the Instagram sync (`usage-throttle.ts`,
//! `resolveUsageThrottle`), the shared pause, and `pLimit` + `Promise.all`.
use std::future::Future;
use std::pin::Pin;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::task::Poll;
use std::time::Duration;

use serde_json::Value;
use tokio::time::Instant;

/// One usage reading (percent figures; `regain_secs` only from BUC).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(crate) struct Usage {
    pub call_count: f64,
    pub total_cputime: f64,
    pub total_time: f64,
    pub regain_secs: f64,
}

impl Usage {
    fn peak(&self) -> f64 {
        self.call_count.max(self.total_cputime).max(self.total_time)
    }
}

fn header<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(n, _)| n.eq_ignore_ascii_case(name))
        .map(|(_, v)| v.as_str())
}

fn num(entry: &Value, key: &str) -> f64 {
    entry[key].as_f64().unwrap_or(0.0)
}

/// `parseBucHeader`: the first entry of the first business bucket.
///
/// ponytail: serde_json orders the buckets by key, JS by insertion; Meta
/// sends one business id per Page token.
pub(crate) fn parse_buc(headers: &[(String, String)]) -> Option<Usage> {
    let parsed: Value = serde_json::from_str(header(headers, "x-business-use-case-usage")?).ok()?;
    let entry = parsed.as_object()?.values().next()?.as_array()?.first()?;
    Some(Usage {
        call_count: num(entry, "call_count"),
        total_cputime: num(entry, "total_cputime"),
        total_time: num(entry, "total_time"),
        regain_secs: num(entry, "estimated_time_to_regain_access"),
    })
}

/// `parseAppUsageHeader` + `toAppUsageSignal` (`meta-app-usage`).
pub(crate) fn parse_app_usage(headers: &[(String, String)]) -> Option<Usage> {
    let parsed: Value = serde_json::from_str(header(headers, "x-app-usage")?).ok()?;
    parsed.is_object().then(|| Usage {
        call_count: num(&parsed, "call_count"),
        total_cputime: num(&parsed, "total_cputime"),
        total_time: num(&parsed, "total_time"),
        regain_secs: 0.0,
    })
}

/// `concurrencyForUsage`: 0 = exhausted.
pub(crate) fn messenger_concurrency(usage: Option<&Usage>) -> usize {
    let Some(usage) = usage else { return 5 };
    if usage.regain_secs > 0.0 {
        return 0;
    }
    match usage.peak() {
        p if p < 50.0 => 5,
        p if p < 75.0 => 3,
        p if p < 90.0 => 1,
        _ => 0,
    }
}

/// `resolveUsageThrottle` for an app-usage reading: `(concurrency, pause)`.
pub(crate) fn instagram_decision(
    usage: Option<&Usage>,
    default: usize,
    max_pause: Duration,
) -> (usize, Duration) {
    // `DEFAULT_EXHAUSTED_PAUSE_MS`.
    let exhausted = Duration::from_secs(30).min(max_pause);
    let Some(usage) = usage else {
        return (default, Duration::ZERO);
    };
    match usage.peak() {
        p if p < 50.0 => (default, Duration::ZERO),
        p if p < 75.0 => (default.min(3), Duration::ZERO),
        p if p < 90.0 => (1, Duration::ZERO),
        _ => (0, exhausted),
    }
}

/// The adaptive concurrency and the pause shared by a run's Graph calls.
pub(crate) struct Throttle {
    concurrency: AtomicUsize,
    pause_until: Mutex<Option<Instant>>,
}

impl Throttle {
    pub(crate) fn new(concurrency: usize) -> Self {
        Self {
            concurrency: AtomicUsize::new(concurrency),
            pause_until: Mutex::new(None),
        }
    }

    pub(crate) fn concurrency(&self) -> usize {
        self.concurrency.load(Ordering::Relaxed)
    }

    /// Sets the concurrency; `true` when it changed.
    pub(crate) fn set_concurrency(&self, next: usize) -> bool {
        self.concurrency.swap(next, Ordering::Relaxed) != next
    }

    /// `pauseUntil = max(pauseUntil, now + pause)`.
    pub(crate) fn pause_for(&self, pause: Duration) {
        let until = Instant::now() + pause;
        let mut guard = self.pause_until.lock().unwrap_or_else(|e| e.into_inner());
        if guard.is_none_or(|current| current < until) {
            *guard = Some(until);
        }
    }

    /// `respectPause`.
    pub(crate) async fn respect_pause(&self) {
        let until = *self.pause_until.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(until) = until.filter(|u| *u > Instant::now()) {
            tokio::time::sleep_until(until).await;
        }
    }
}

/// `pLimit(limit)` over `Promise.all`: at most `limit` of the futures run at
/// once; the outputs keep the input order.
pub(crate) async fn run_limited<F: Future>(futures: Vec<F>, limit: usize) -> Vec<F::Output> {
    let limit = limit.max(1);
    let mut pending: Vec<Option<Pin<Box<F>>>> =
        futures.into_iter().map(|f| Some(Box::pin(f))).collect();
    let mut out: Vec<Option<F::Output>> = pending.iter().map(|_| None).collect();
    let mut running: Vec<usize> = Vec::new();
    let mut next = 0;
    std::future::poll_fn(|cx| {
        loop {
            while running.len() < limit && next < pending.len() {
                running.push(next);
                next += 1;
            }
            let before = running.len();
            running.retain(|&i| {
                let Some(future) = pending[i].as_mut() else {
                    return false;
                };
                match future.as_mut().poll(cx) {
                    Poll::Ready(value) => {
                        out[i] = Some(value);
                        pending[i] = None;
                        false
                    }
                    Poll::Pending => true,
                }
            });
            if running.is_empty() && next >= pending.len() {
                return Poll::Ready(());
            }
            if running.len() == before {
                return Poll::Pending;
            }
        }
    })
    .await;
    out.into_iter().flatten().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers(name: &str, value: &str) -> Vec<(String, String)> {
        vec![(name.to_owned(), value.to_owned())]
    }

    #[test]
    fn readings_map_to_the_original_tiers() {
        let buc = parse_buc(&headers(
            "X-Business-Use-Case-Usage",
            r#"{"page-1":[{"call_count":60,"total_cputime":10,"total_time":5,"estimated_time_to_regain_access":0}]}"#,
        ))
        .unwrap();
        assert_eq!(messenger_concurrency(Some(&buc)), 3);
        assert_eq!(messenger_concurrency(None), 5);
        let exhausted = Usage {
            regain_secs: 12.0,
            ..Usage::default()
        };
        assert_eq!(messenger_concurrency(Some(&exhausted)), 0);
        assert_eq!(
            parse_buc(&headers("x-business-use-case-usage", "not json")),
            None
        );

        let app = parse_app_usage(&headers("x-app-usage", r#"{"call_count":80}"#)).unwrap();
        let max = Duration::from_secs(45);
        assert_eq!(instagram_decision(Some(&app), 3, max), (1, Duration::ZERO));
        let full = Usage {
            total_time: 95.0,
            ..Usage::default()
        };
        assert_eq!(
            instagram_decision(Some(&full), 3, max),
            (0, Duration::from_secs(30))
        );
        assert_eq!(instagram_decision(None, 3, max), (3, Duration::ZERO));
    }

    #[tokio::test]
    async fn run_limited_keeps_the_order_and_the_limit() {
        let live = AtomicUsize::new(0);
        let peak = AtomicUsize::new(0);
        let futures = (0..7)
            .map(|i| {
                let (live, peak) = (&live, &peak);
                async move {
                    let now = live.fetch_add(1, Ordering::SeqCst) + 1;
                    peak.fetch_max(now, Ordering::SeqCst);
                    tokio::time::sleep(Duration::from_millis(5 * (7 - i))).await;
                    live.fetch_sub(1, Ordering::SeqCst);
                    i
                }
            })
            .collect();
        assert_eq!(run_limited(futures, 3).await, vec![0, 1, 2, 3, 4, 5, 6]);
        assert_eq!(peak.load(Ordering::SeqCst), 3);
    }
}
