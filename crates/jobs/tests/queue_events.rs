//! Port of `packages/worker-config/__tests__/queue-events.test.ts` (spec 5.1). TS-only (one `QueueEvents`
//! per queue on Redis) → proposal: one process-wide `CompletionRegistry`.

use std::sync::{Arc, Mutex};

use chatbotx_jobs::{CompletionRegistry, JobEvent, JobWaiter, QueueName, WaitError};
use serde_json::json;

/// Both tests use the process-wide registry; run them one at a time.
static GLOBAL: Mutex<()> = Mutex::new(());

/// Port of: packages/worker-config/__tests__/queue-events.test.ts:27 "shared queue events > returns the same instance for the same queue"
#[test]
fn returns_the_same_instance_for_the_same_queue() {
    let _serial = GLOBAL.lock().unwrap_or_else(|e| e.into_inner());

    let first = CompletionRegistry::global();
    let second = CompletionRegistry::global();

    assert!(Arc::ptr_eq(&first, &second));
}

/// Port of: packages/worker-config/__tests__/queue-events.test.ts:35 "shared queue events > closes every instance and creates a fresh one afterwards"
#[tokio::test]
async fn closes_every_instance_and_creates_a_fresh_one_afterwards() {
    let _serial = GLOBAL.lock().unwrap_or_else(|e| e.into_inner());
    let chat = CompletionRegistry::global();

    let pending = chat.wait_until_finished(QueueName::Chat, "job-1", 60_000);
    let (result, ()) = tokio::join!(pending, CompletionRegistry::close_global());

    assert_eq!(result, Err(WaitError::Shutdown));
    assert!(chat.is_closed());
    let fresh = CompletionRegistry::global();
    assert!(!Arc::ptr_eq(&fresh, &chat));

    let wait = fresh.wait_until_finished(QueueName::Chat, "job-2", 60_000);
    let publish = async {
        fresh.publish(JobEvent::Completed {
            queue: QueueName::Chat,
            job_id: "job-2".to_owned(),
            return_value: json!("done"),
        });
    };
    let (result, ()) = tokio::join!(wait, publish);
    assert_eq!(result, Ok(json!("done")));
}
