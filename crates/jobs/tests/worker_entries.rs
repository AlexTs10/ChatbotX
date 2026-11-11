//! Port of `apps/worker/__tests__/worker-entries.test.ts` (spec 5.3
//! "worker-entries"). The originals check that every `src/*/worker*.ts`
//! module is a tsdown entry and has a dev script. With one Rust binary the
//! intent is: every worker spec is started by the process, and every queue
//! with a consumer has a spec (dev runs the same binary).

use chatbotx_jobs::QueueName;
use chatbotx_jobs::runtime::{WorkerMode, started_queues, worker_specs};

/// Port of: apps/worker/__tests__/worker-entries.test.ts:20 "worker processes > every worker module is a tsdown entry"
#[test]
fn every_worker_module_is_a_tsdown_entry() {
    let started = started_queues(WorkerMode::Standalone);
    let specs = worker_specs();
    assert!(!specs.is_empty());

    let missing: Vec<QueueName> = specs
        .iter()
        .map(|spec| spec.queue)
        .filter(|queue| !started.contains(queue))
        .collect();

    assert_eq!(missing, Vec::<QueueName>::new());
}

/// Port of: apps/worker/__tests__/worker-entries.test.ts:31 "worker processes > every worker module has a dev script, so `pnpm dev` starts it"
#[test]
fn every_worker_module_has_a_dev_script_so_pnpm_dev_starts_it() {
    let specs = worker_specs();

    // `broadcast` has no consumer (dead name) and `quota` is consumed by the
    // private enterprise worker (spec 1.2).
    let missing: Vec<QueueName> = QueueName::ALL
        .into_iter()
        .filter(|q| !matches!(q, QueueName::Broadcast | QueueName::Quota))
        .filter(|q| !specs.iter().any(|spec| spec.queue == *q))
        .collect();

    assert_eq!(missing, Vec::<QueueName>::new());
}
