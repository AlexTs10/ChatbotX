//! Port of `apps/realtime/__tests__/presence-reporter.test.ts` (P2.8 spec
//! 4.6, R173-R183). Same harness: a recording `report`, a counter `now`,
//! `Promise.withResolvers` gates (`support::Deferred`), `flushMicrotasks`
//! (`support::flush_tasks`) and `vi.waitFor` (`support::wait_for`).

mod support;

use std::collections::HashMap;
use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use chatbotx_rt::BoxFuture;
use chatbotx_rt::presence::{PresenceReporter, PresenceReporterOptions, create_presence_reporter};
use support::{Deferred, flush_tasks, strings, wait_for};

const WAIT: Duration = Duration::from_secs(5);

/// An insertion-ordered `Map<string, string[]>`.
#[derive(Default)]
struct Users {
    order: Vec<String>,
    map: HashMap<String, Vec<String>>,
}

impl Users {
    fn of(entries: &[(&str, &[&str])]) -> Arc<Mutex<Users>> {
        let mut users = Users::default();
        for (workspace_id, user_ids) in entries {
            users.set(workspace_id, strings(user_ids));
        }
        Arc::new(Mutex::new(users))
    }

    fn numbered(count: usize) -> Arc<Mutex<Users>> {
        let mut users = Users::default();
        for index in 0..count {
            users.set(&format!("ws_{index}"), strings(&["u"]));
        }
        Arc::new(Mutex::new(users))
    }

    fn set(&mut self, workspace_id: &str, user_ids: Vec<String>) {
        if !self.map.contains_key(workspace_id) {
            self.order.push(workspace_id.to_owned());
        }
        self.map.insert(workspace_id.to_owned(), user_ids);
    }

    fn retain(&mut self, keep: impl Fn(&str) -> bool) {
        self.order.retain(|workspace_id| keep(workspace_id));
        self.map.retain(|workspace_id, _| keep(workspace_id));
    }
}

type ReportFor = Arc<dyn Fn(&str) -> BoxFuture<'static, ()> + Send + Sync>;
type IsAccepted = Arc<dyn Fn(&str) -> bool + Send + Sync>;
type Gates = Arc<Mutex<Vec<(String, Arc<Deferred>)>>>;

struct Harness {
    reporter: PresenceReporter,
    reported: Arc<Mutex<Vec<String>>>,
    reported_user_ids: Arc<Mutex<HashMap<String, Vec<String>>>>,
    report_calls: Arc<AtomicUsize>,
}

impl Harness {
    fn reported(&self) -> Vec<String> {
        self.reported.lock().unwrap().clone()
    }

    fn calls(&self) -> usize {
        self.report_calls.load(Ordering::SeqCst)
    }
}

fn options(
    concurrency: usize,
    users: &Arc<Mutex<Users>>,
    report: chatbotx_rt::presence::ReportFn,
) -> PresenceReporterOptions {
    let connected = Arc::clone(users);
    let user_ids = Arc::clone(users);
    PresenceReporterOptions {
        concurrency,
        get_connected_workspace_ids: Arc::new(move || connected.lock().unwrap().order.clone()),
        get_user_ids: Arc::new(move |workspace_id| {
            user_ids
                .lock()
                .unwrap()
                .map
                .get(workspace_id)
                .cloned()
                .unwrap_or_default()
        }),
        now: None,
        on_error: None,
        report,
    }
}

/// `createHarness`.
fn create_harness(
    concurrency: usize,
    users: &Arc<Mutex<Users>>,
    report_for: Option<ReportFor>,
    is_accepted: Option<IsAccepted>,
) -> Harness {
    let reported = Arc::new(Mutex::new(Vec::new()));
    let reported_user_ids = Arc::new(Mutex::new(HashMap::new()));
    let report_calls = Arc::new(AtomicUsize::new(0));
    let clock = Arc::new(AtomicI64::new(0));
    let report = {
        let reported = Arc::clone(&reported);
        let reported_user_ids = Arc::clone(&reported_user_ids);
        let report_calls = Arc::clone(&report_calls);
        Arc::new(move |workspace_id: String, user_ids: Vec<String>| {
            report_calls.fetch_add(1, Ordering::SeqCst);
            reported.lock().unwrap().push(workspace_id.clone());
            reported_user_ids
                .lock()
                .unwrap()
                .insert(workspace_id.clone(), user_ids);
            let report_for = report_for.clone();
            let is_accepted = is_accepted.clone();
            Box::pin(async move {
                if let Some(report_for) = report_for {
                    report_for(&workspace_id).await;
                }
                Ok(is_accepted.is_none_or(|accepted| accepted(&workspace_id)))
            }) as BoxFuture<'static, Result<bool, chatbotx_rt::BoxError>>
        })
    };
    let mut opts = options(concurrency, users, report);
    opts.now = Some(Arc::new(move || clock.fetch_add(1, Ordering::SeqCst) + 1));
    Harness {
        reporter: create_presence_reporter(opts),
        reported,
        reported_user_ids,
        report_calls,
    }
}

/// A `reportFor` that waits on one shared gate for `gated` (all when `None`).
fn gated(gate: &Arc<Deferred>, only: Option<&'static str>) -> ReportFor {
    let gate = Arc::clone(gate);
    Arc::new(move |workspace_id: &str| {
        let gate = Arc::clone(&gate);
        let wait = only.is_none_or(|only| only == workspace_id);
        Box::pin(async move {
            if wait {
                gate.wait().await;
            }
        })
    })
}

/// Port of: apps/realtime/__tests__/presence-reporter.test.ts:47 "createPresenceReporter > does not let one slow workspace delay the others (no chunk head-of-line blocking)"
#[tokio::test]
async fn createpresencereporter_does_not_let_one_slow_workspace_delay_the_others_no_chunk_head_of_line_blocking()
 {
    let slow = Deferred::new();
    let users = Users::numbered(40);
    let harness = create_harness(4, &users, Some(gated(&slow, Some("ws_0"))), None);

    harness.reporter.sweep();
    wait_for(WAIT, || harness.reported().len() == 40).await;

    let mut distinct = harness.reported();
    distinct.sort();
    distinct.dedup();
    assert_eq!(distinct.len(), 40);
    slow.resolve();
    harness.reporter.stop();
}

/// Port of: apps/realtime/__tests__/presence-reporter.test.ts:67 "createPresenceReporter > never exceeds the configured concurrency"
#[tokio::test]
async fn createpresencereporter_never_exceeds_the_configured_concurrency() {
    let gates: Gates = Arc::new(Mutex::new(Vec::new()));
    let users = Users::numbered(10);
    let active = Arc::new(AtomicUsize::new(0));
    let max_active = Arc::new(AtomicUsize::new(0));
    let report_for: ReportFor = {
        let gates = Arc::clone(&gates);
        let active = Arc::clone(&active);
        let max_active = Arc::clone(&max_active);
        Arc::new(move |workspace_id: &str| {
            let now_active = active.fetch_add(1, Ordering::SeqCst) + 1;
            max_active.fetch_max(now_active, Ordering::SeqCst);
            let gate = Deferred::new();
            gates
                .lock()
                .unwrap()
                .push((workspace_id.to_owned(), Arc::clone(&gate)));
            let active = Arc::clone(&active);
            Box::pin(async move {
                gate.wait().await;
                active.fetch_sub(1, Ordering::SeqCst);
            })
        })
    };
    let harness = create_harness(3, &users, Some(report_for), None);

    harness.reporter.sweep();
    flush_tasks().await;
    assert_eq!(harness.reported().len(), 3);

    for _ in 0..10 {
        let reported = harness.reported();
        let next = {
            let mut gates = gates.lock().unwrap();
            let position = gates
                .iter()
                .position(|(workspace_id, _)| reported.contains(workspace_id));
            position.map(|position| gates.remove(position))
        };
        if let Some((_, gate)) = next {
            gate.resolve();
        }
        flush_tasks().await;
    }

    assert!(max_active.load(Ordering::SeqCst) <= 3);
    assert_eq!(harness.reported().len(), 10);
    harness.reporter.stop();
}

/// Port of: apps/realtime/__tests__/presence-reporter.test.ts:106 "createPresenceReporter > reports the stalest workspace first on a sweep"
#[tokio::test]
async fn createpresencereporter_reports_the_stalest_workspace_first_on_a_sweep() {
    let users = Users::of(&[("ws_a", &["u"]), ("ws_b", &["u"]), ("ws_c", &["u"])]);
    let harness = create_harness(1, &users, None, None);

    harness.reporter.report_now("ws_c");
    harness.reporter.report_now("ws_a");
    wait_for(WAIT, || harness.reported().len() == 2).await;
    // Let the second report finish before the sweep (`vi.waitFor` polls).
    flush_tasks().await;
    harness.reported.lock().unwrap().clear();

    harness.reporter.sweep();
    wait_for(WAIT, || harness.reported().len() == 3).await;

    let reported = harness.reported();
    assert_ne!(reported.last().map(String::as_str), Some("ws_b"));
    assert_eq!(reported[0], "ws_b");
    harness.reporter.stop();
}

/// Port of: apps/realtime/__tests__/presence-reporter.test.ts:130 "createPresenceReporter > serves reportNow ahead of an already queued sweep"
#[tokio::test]
async fn createpresencereporter_serves_reportnow_ahead_of_an_already_queued_sweep() {
    let gate = Deferred::new();
    let users = Users::numbered(5);
    let harness = create_harness(1, &users, Some(gated(&gate, Some("ws_0"))), None);

    harness.reporter.sweep();
    flush_tasks().await;
    harness.reporter.report_now("ws_4");
    gate.resolve();
    wait_for(WAIT, || harness.reported().len() == 5).await;

    let reported = harness.reported();
    assert_eq!(reported[1], "ws_4");
    assert_eq!(reported.iter().filter(|ws| *ws == "ws_4").count(), 1);
    harness.reporter.stop();
}

/// Port of: apps/realtime/__tests__/presence-reporter.test.ts:155 "createPresenceReporter > does not start a second report for a workspace already in flight, and re-runs a reportNow requested meanwhile"
#[tokio::test]
async fn createpresencereporter_does_not_start_a_second_report_for_a_workspace_already_in_flight_and_re_runs_a_reportnow_requested_meanwhile()
 {
    let gate = Deferred::new();
    let users = Users::of(&[("ws_1", &["u_1"])]);
    let harness = create_harness(4, &users, Some(gated(&gate, None)), None);

    harness.reporter.sweep();
    flush_tasks().await;
    harness.reporter.sweep();
    flush_tasks().await;
    assert_eq!(harness.calls(), 1);

    users.lock().unwrap().set("ws_1", strings(&["u_1", "u_2"]));
    harness.reporter.report_now("ws_1");
    gate.resolve();
    wait_for(WAIT, || harness.calls() == 2).await;

    assert_eq!(
        harness
            .reported_user_ids
            .lock()
            .unwrap()
            .get("ws_1")
            .cloned(),
        Some(strings(&["u_1", "u_2"]))
    );
    harness.reporter.stop();
}

/// Port of: apps/realtime/__tests__/presence-reporter.test.ts:179 "createPresenceReporter > skips a workspace that has no connected users any more and prunes its bookkeeping"
#[tokio::test]
async fn createpresencereporter_skips_a_workspace_that_has_no_connected_users_any_more_and_prunes_its_bookkeeping()
 {
    let users = Users::of(&[("ws_gone", &[])]);
    let harness = create_harness(2, &users, None, None);

    harness.reporter.report_now("ws_gone");
    flush_tasks().await;

    assert_eq!(harness.calls(), 0);
    harness.reporter.stop();
}

/// Port of: apps/realtime/__tests__/presence-reporter.test.ts:193 "createPresenceReporter > drains a large backlog of disconnected workspaces without recursing, keeping every slot usable"
#[tokio::test]
async fn createpresencereporter_drains_a_large_backlog_of_disconnected_workspaces_without_recursing_keeping_every_slot_usable()
 {
    let gate = Deferred::new();
    let backlog_size = 20_000;
    let users = Users::numbered(backlog_size);
    let on_error_calls = Arc::new(AtomicUsize::new(0));
    let reported = Arc::new(Mutex::new(Vec::<String>::new()));
    let report: chatbotx_rt::presence::ReportFn = {
        let reported = Arc::clone(&reported);
        let gate = Arc::clone(&gate);
        Arc::new(move |workspace_id: String, _user_ids: Vec<String>| {
            reported.lock().unwrap().push(workspace_id.clone());
            let gate = Arc::clone(&gate);
            Box::pin(async move {
                if workspace_id == "ws_0" {
                    gate.wait().await;
                }
                Ok(true)
            })
        })
    };
    let mut opts = options(1, &users, report);
    let errors = Arc::clone(&on_error_calls);
    opts.on_error = Some(Arc::new(move |_error, _workspace_id| {
        errors.fetch_add(1, Ordering::SeqCst);
    }));
    let reporter = create_presence_reporter(opts);

    reporter.sweep();
    flush_tasks().await;
    users
        .lock()
        .unwrap()
        .retain(|workspace_id| workspace_id == "ws_0");
    users.lock().unwrap().set("ws_new", strings(&["u"]));
    gate.resolve();
    reporter.report_now("ws_new");

    wait_for(WAIT, || {
        reported.lock().unwrap().iter().any(|ws| ws == "ws_new")
    })
    .await;
    assert_eq!(on_error_calls.load(Ordering::SeqCst), 0);
    reporter.stop();
}

/// Port of: apps/realtime/__tests__/presence-reporter.test.ts:228 "createPresenceReporter > does not treat a rejected report as fresh: that workspace stays the stalest"
#[tokio::test]
async fn createpresencereporter_does_not_treat_a_rejected_report_as_fresh_that_workspace_stays_the_stalest()
 {
    let users = Users::of(&[("ws_a", &["u"]), ("ws_b", &["u"])]);
    let is_a_accepted = Arc::new(std::sync::atomic::AtomicBool::new(true));
    let is_accepted: IsAccepted = {
        let is_a_accepted = Arc::clone(&is_a_accepted);
        Arc::new(move |workspace_id: &str| {
            workspace_id != "ws_a" || is_a_accepted.load(Ordering::SeqCst)
        })
    };
    let harness = create_harness(1, &users, None, Some(is_accepted));

    harness.reporter.report_now("ws_a");
    harness.reporter.report_now("ws_b");
    wait_for(WAIT, || harness.reported().len() == 2).await;
    // b renews again, then a's renewal is rejected.
    harness.reporter.report_now("ws_b");
    wait_for(WAIT, || harness.reported().len() == 3).await;
    is_a_accepted.store(false, Ordering::SeqCst);
    harness.reporter.report_now("ws_a");
    wait_for(WAIT, || harness.reported().len() == 4).await;
    // Let the rejected report of ws_a finish before the sweep.
    flush_tasks().await;
    harness.reported.lock().unwrap().clear();

    harness.reporter.sweep();
    wait_for(WAIT, || harness.reported().len() == 2).await;

    // a last renewed before b did, so its lease expires first.
    assert_eq!(harness.reported()[0], "ws_a");
    harness.reporter.stop();
}

/// Port of: apps/realtime/__tests__/presence-reporter.test.ts:260 "createPresenceReporter > re-sorts a backlog still queued when the next sweep starts, so a failed workspace is retried first"
#[tokio::test]
async fn createpresencereporter_re_sorts_a_backlog_still_queued_when_the_next_sweep_starts_so_a_failed_workspace_is_retried_first()
 {
    let gates: Arc<Mutex<HashMap<String, Arc<Deferred>>>> = Arc::new(Mutex::new(HashMap::new()));
    let users = Users::of(&[
        ("ws_a", &["u"]),
        ("ws_b", &["u"]),
        ("ws_c", &["u"]),
        ("ws_d", &["u"]),
    ]);
    let report_for: ReportFor = {
        let gates = Arc::clone(&gates);
        Arc::new(move |workspace_id: &str| {
            let gate = Deferred::new();
            gates
                .lock()
                .unwrap()
                .insert(workspace_id.to_owned(), Arc::clone(&gate));
            Box::pin(async move { gate.wait().await })
        })
    };
    let is_accepted: IsAccepted = Arc::new(|workspace_id: &str| workspace_id != "ws_a");
    let harness = create_harness(1, &users, Some(report_for), Some(is_accepted));
    let gate_of = |workspace_id: &str| gates.lock().unwrap().get(workspace_id).cloned();

    harness.reporter.sweep();
    wait_for(WAIT, || {
        harness.reported() == strings(&["ws_a"]) && gate_of("ws_a").is_some()
    })
    .await;
    // ws_a's renewal is rejected, so it stays the stalest; ws_b starts next.
    gate_of("ws_a").unwrap().resolve();
    wait_for(WAIT, || {
        harness.reported() == strings(&["ws_a", "ws_b"]) && gate_of("ws_b").is_some()
    })
    .await;
    // ws_c and ws_d are still queued from the first sweep.
    harness.reporter.sweep();
    gate_of("ws_b").unwrap().resolve();
    wait_for(WAIT, || harness.reported().len() == 3).await;

    assert_eq!(harness.reported()[2], "ws_a");
    harness.reporter.stop();
}

/// Port of: apps/realtime/__tests__/presence-reporter.test.ts:290 "createPresenceReporter > keeps draining after a report rejects"
#[tokio::test]
async fn createpresencereporter_keeps_draining_after_a_report_rejects() {
    let users = Users::of(&[("ws_bad", &["u"]), ("ws_ok", &["u"])]);
    let report_calls = Arc::new(AtomicUsize::new(0));
    let on_error_calls = Arc::new(AtomicUsize::new(0));
    let report: chatbotx_rt::presence::ReportFn = {
        let report_calls = Arc::clone(&report_calls);
        Arc::new(move |workspace_id: String, _user_ids: Vec<String>| {
            report_calls.fetch_add(1, Ordering::SeqCst);
            Box::pin(async move {
                if workspace_id == "ws_bad" {
                    Err(Box::new(support::ReportFailed) as chatbotx_rt::BoxError)
                } else {
                    Ok(true)
                }
            })
        })
    };
    let mut opts = options(1, &users, report);
    let errors = Arc::clone(&on_error_calls);
    opts.on_error = Some(Arc::new(move |_error, _workspace_id| {
        errors.fetch_add(1, Ordering::SeqCst);
    }));
    let reporter = create_presence_reporter(opts);

    reporter.sweep();
    wait_for(WAIT, || report_calls.load(Ordering::SeqCst) == 2).await;
    flush_tasks().await;

    assert_eq!(on_error_calls.load(Ordering::SeqCst), 1);
    reporter.stop();
}

/// Port of: apps/realtime/__tests__/presence-reporter.test.ts:316 "createPresenceReporter > stops starting new reports after stop()"
#[tokio::test]
async fn createpresencereporter_stops_starting_new_reports_after_stop() {
    let users = Users::of(&[("ws_1", &["u"])]);
    let harness = create_harness(1, &users, None, None);

    harness.reporter.stop();
    harness.reporter.sweep();
    harness.reporter.report_now("ws_1");
    flush_tasks().await;

    assert_eq!(harness.calls(), 0);
}
