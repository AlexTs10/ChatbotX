#![allow(clippy::assertions_on_constants)] // lint only, added by the orchestrator (DECISIONS P2.6 gate); no test logic changed
//! Port of `packages/worker-config/__tests__/template-queue.test.ts` (spec 5.1). The bullmq `Queue` mock
//! (adds per queue name + a BullMQ-like job map) is `FakeTemplateQueue`.

#[path = "support/worker_config.rs"]
mod worker_config;

use chatbotx_jobs::queues::default_jobs::DefaultJobAction;
use chatbotx_jobs::queues::template::{
    FanOutTemplateUpdateData, InstallTemplateData, SyncTemplateInstallationData,
    TEMPLATE_PRIORITY_BACKGROUND, TEMPLATE_PRIORITY_INTERACTIVE, TemplateJobAction, TemplateWork,
    UninstallTemplateData, enqueue_fan_out_template_update, enqueue_install_template,
    enqueue_sync_template_installation, enqueue_uninstall_template,
};
use chatbotx_jobs::{JobState, JobsError, QueueGroup, QueueName, Retention, queue_group};
use serde_json::json;
use worker_config::{ChangeBehavior, FakeTemplateQueue};

fn install(installation_id: &str) -> InstallTemplateData {
    InstallTemplateData {
        installation_id: installation_id.to_owned(),
        workspace_id: "w1".to_owned(),
    }
}

fn sync(sync_id: &str) -> SyncTemplateInstallationData {
    SyncTemplateInstallationData {
        sync_id: sync_id.to_owned(),
        workspace_id: "w1".to_owned(),
    }
}

fn uninstall(attempt: u32) -> UninstallTemplateData {
    UninstallTemplateData {
        installation_id: "i1".to_owned(),
        workspace_id: "w1".to_owned(),
        attempt,
    }
}

fn fan_out() -> FanOutTemplateUpdateData {
    FanOutTemplateUpdateData {
        template_id: "t1".to_owned(),
        workspace_id: "w1".to_owned(),
        version: 3,
        after_id: None,
    }
}

/// Port of: packages/worker-config/__tests__/template-queue.test.ts:82 "template queue > has its own queue on the bulk Redis group, apart from the shared default queue"
#[test]
fn has_its_own_queue_on_the_bulk_redis_group_apart_from_the() {
    assert_eq!(QueueName::Template.as_str(), "template");
    assert_eq!(queue_group(QueueName::Template), QueueGroup::Bulk);
}

/// Port of: packages/worker-config/__tests__/template-queue.test.ts:87 "template queue > no template job is left on the default queue"
#[test]
fn no_template_job_is_left_on_the_default_queue() {
    let default_actions: Vec<&str> = DefaultJobAction::all().iter().map(|a| a.as_str()).collect();
    for action in TemplateJobAction::all() {
        assert!(!default_actions.contains(&action.as_str()), "{action:?}");
    }
}

/// Port of: packages/worker-config/__tests__/template-queue.test.ts:93 "template queue > every enqueue lands on the template queue, never the default one, under a deterministic, BullMQ-safe id"
#[tokio::test]
async fn every_enqueue_lands_on_the_template_queue_never_the_default() {
    let queue = FakeTemplateQueue::new();
    enqueue_install_template(&queue, install("i1"))
        .await
        .unwrap();
    enqueue_sync_template_installation(&queue, sync("s1"), TemplateWork::Background)
        .await
        .unwrap();
    enqueue_uninstall_template(&queue, uninstall(2))
        .await
        .unwrap();
    enqueue_fan_out_template_update(&queue, fan_out(), TemplateWork::Background)
        .await
        .unwrap();

    assert!(queue.calls_on(QueueName::Default).is_empty());
    let calls = queue.calls_on(QueueName::Template);
    let names_and_ids: Vec<(String, Option<String>)> = calls
        .iter()
        .map(|c| (c.name.clone(), c.opts.job_id.clone()))
        .collect();
    let expected: Vec<(String, Option<String>)> = [
        ("installTemplate", "install-template-i1"),
        ("syncTemplateInstallation", "tpl-sync-s1"),
        ("uninstallTemplate", "tpl-uninstall-i1-2"),
        ("fanOutTemplateUpdate", "tpl-fanout-t1-3-first"),
    ]
    .iter()
    .map(|(n, id)| ((*n).to_owned(), Some((*id).to_owned())))
    .collect();
    assert_eq!(names_and_ids, expected);
    for call in &calls {
        assert_eq!(call.data["type"], json!(call.name));
        assert!(!call.opts.job_id.as_deref().unwrap().contains(':'));
        // The row is the outbox: a retained job would turn the sweep's re-add into a no-op.
        assert_eq!(call.opts.remove_on_complete, Some(Retention::Remove));
        assert_eq!(call.opts.remove_on_fail, Some(Retention::Remove));
    }
}

/// Port of: packages/worker-config/__tests__/template-queue.test.ts:126 "template queue > work a person waits on runs before the background syncs a save fans out"
#[tokio::test]
async fn work_a_person_waits_on_runs_before_the_background_syncs_a() {
    let queue = FakeTemplateQueue::new();
    enqueue_install_template(&queue, install("i1"))
        .await
        .unwrap();
    enqueue_uninstall_template(&queue, uninstall(1))
        .await
        .unwrap();
    enqueue_sync_template_installation(&queue, sync("manual"), TemplateWork::Interactive)
        .await
        .unwrap();
    enqueue_sync_template_installation(&queue, sync("auto"), TemplateWork::Background)
        .await
        .unwrap();
    enqueue_fan_out_template_update(&queue, fan_out(), TemplateWork::Background)
        .await
        .unwrap();

    // Lower runs first; every job names one, so none jumps the line unprioritized.
    assert!(TEMPLATE_PRIORITY_INTERACTIVE < TEMPLATE_PRIORITY_BACKGROUND);
    assert_eq!(
        TemplateWork::Interactive.priority(),
        TEMPLATE_PRIORITY_INTERACTIVE
    );
    assert_eq!(
        TemplateWork::Background.priority(),
        TEMPLATE_PRIORITY_BACKGROUND
    );
    let ids_and_priorities: Vec<(Option<String>, Option<u32>)> = queue
        .calls_on(QueueName::Template)
        .iter()
        .map(|c| (c.opts.job_id.clone(), c.opts.priority))
        .collect();
    let expected: Vec<(Option<String>, Option<u32>)> = [
        ("install-template-i1", TEMPLATE_PRIORITY_INTERACTIVE),
        ("tpl-uninstall-i1-1", TEMPLATE_PRIORITY_INTERACTIVE),
        ("tpl-sync-manual", TEMPLATE_PRIORITY_INTERACTIVE),
        ("tpl-sync-auto", TEMPLATE_PRIORITY_BACKGROUND),
        ("tpl-fanout-t1-3-first", TEMPLATE_PRIORITY_BACKGROUND),
    ]
    .iter()
    .map(|(id, p)| (Some((*id).to_owned()), Some(*p)))
    .collect();
    assert_eq!(ids_and_priorities, expected);
}

/// Port of: packages/worker-config/__tests__/template-queue.test.ts:168 "template queue > a sync someone asks for while it is already queued as background work > is raised to interactive"
#[tokio::test]
async fn is_raised_to_interactive() {
    let queue = FakeTemplateQueue::new();
    enqueue_sync_template_installation(&queue, sync("s1"), TemplateWork::Background)
        .await
        .unwrap();
    enqueue_sync_template_installation(&queue, sync("s1"), TemplateWork::Interactive)
        .await
        .unwrap();

    assert_eq!(
        queue.priority_of("tpl-sync-s1"),
        Some(TEMPLATE_PRIORITY_INTERACTIVE)
    );
}

/// Port of: packages/worker-config/__tests__/template-queue.test.ts:175 "template queue > a sync someone asks for while it is already queued as background work > a second request for an already raised job changes nothing"
#[tokio::test]
async fn a_second_request_for_an_already_raised_job_changes_nothing() {
    let queue = FakeTemplateQueue::new();
    enqueue_sync_template_installation(&queue, sync("s1"), TemplateWork::Background)
        .await
        .unwrap();
    enqueue_sync_template_installation(&queue, sync("s1"), TemplateWork::Interactive)
        .await
        .unwrap();
    queue.clear_change_calls("tpl-sync-s1");

    enqueue_sync_template_installation(&queue, sync("s1"), TemplateWork::Interactive)
        .await
        .unwrap();

    // A second change would give it a later FIFO slot among interactive jobs.
    let job = queue.job("tpl-sync-s1").unwrap();
    assert_eq!(job.change_priority_calls, 0);
    assert_eq!(job.opts.priority, Some(TEMPLATE_PRIORITY_BACKGROUND));
    assert_eq!(
        queue.priority_of("tpl-sync-s1"),
        Some(TEMPLATE_PRIORITY_INTERACTIVE)
    );
}

/// Port of: packages/worker-config/__tests__/template-queue.test.ts:189 "template queue > a sync someone asks for while it is already queued as background work > a waiting job (moved back after a stall) is left where it is: it already runs first"
#[tokio::test]
async fn a_waiting_job_moved_back_after_a_stall_is_left_where_it_is() {
    let queue = FakeTemplateQueue::new();
    enqueue_sync_template_installation(&queue, sync("s1"), TemplateWork::Background)
        .await
        .unwrap();
    queue.set_state("tpl-sync-s1", JobState::Waiting);

    enqueue_sync_template_installation(&queue, sync("s1"), TemplateWork::Interactive)
        .await
        .unwrap();

    assert_eq!(queue.job("tpl-sync-s1").unwrap().change_priority_calls, 0);
}

/// Port of: packages/worker-config/__tests__/template-queue.test.ts:201 "template queue > a sync someone asks for while it is already queued as background work > a later background enqueue never lowers it"
#[tokio::test]
async fn a_later_background_enqueue_never_lowers_it() {
    let queue = FakeTemplateQueue::new();
    enqueue_sync_template_installation(&queue, sync("s1"), TemplateWork::Interactive)
        .await
        .unwrap();
    enqueue_sync_template_installation(&queue, sync("s1"), TemplateWork::Background)
        .await
        .unwrap();

    assert_eq!(
        queue.priority_of("tpl-sync-s1"),
        Some(TEMPLATE_PRIORITY_INTERACTIVE)
    );
}

/// Port of: packages/worker-config/__tests__/template-queue.test.ts:208 "template queue > a sync someone asks for while it is already queued as background work > a job a worker already started is left alone"
#[tokio::test]
async fn a_job_a_worker_already_started_is_left_alone() {
    let queue = FakeTemplateQueue::new();
    enqueue_sync_template_installation(&queue, sync("s1"), TemplateWork::Background)
        .await
        .unwrap();
    queue.set_state("tpl-sync-s1", JobState::Active);

    enqueue_sync_template_installation(&queue, sync("s1"), TemplateWork::Interactive)
        .await
        .unwrap();

    assert_eq!(
        queue.priority_of("tpl-sync-s1"),
        Some(TEMPLATE_PRIORITY_BACKGROUND)
    );
}

/// Port of: packages/worker-config/__tests__/template-queue.test.ts:219 "template queue > a sync someone asks for while it is already queued as background work > a job picked up between the state read and the change is not an error"
#[tokio::test]
async fn a_job_picked_up_between_the_state_read_and_the_change_is_not() {
    let queue = FakeTemplateQueue::new();
    enqueue_sync_template_installation(&queue, sync("s1"), TemplateWork::Background)
        .await
        .unwrap();
    queue.set_change_behavior(
        "tpl-sync-s1",
        ChangeBehavior::ActivateThenFail("Job is not in a promotable state".to_owned()),
    );

    let result =
        enqueue_sync_template_installation(&queue, sync("s1"), TemplateWork::Interactive).await;

    assert_eq!(result, Ok(()));
}

/// Port of: packages/worker-config/__tests__/template-queue.test.ts:234 "template queue > a sync someone asks for while it is already queued as background work > a change that fails while the job still waits is reported"
#[tokio::test]
async fn a_change_that_fails_while_the_job_still_waits_is_reported() {
    let queue = FakeTemplateQueue::new();
    enqueue_sync_template_installation(&queue, sync("s1"), TemplateWork::Background)
        .await
        .unwrap();
    queue.set_change_behavior("tpl-sync-s1", ChangeBehavior::Fail("redis down".to_owned()));

    let result =
        enqueue_sync_template_installation(&queue, sync("s1"), TemplateWork::Interactive).await;

    assert_eq!(result, Err(JobsError::Store("redis down".to_owned())));
}

/// Port of: packages/worker-config/__tests__/template-queue.test.ts:247 "template queue > the sweep's catch-up raises a fan-out that still waits as background work"
#[tokio::test]
async fn the_sweep_s_catch_up_raises_a_fan_out_that_still_waits_as() {
    let queue = FakeTemplateQueue::new();
    enqueue_fan_out_template_update(&queue, fan_out(), TemplateWork::Background)
        .await
        .unwrap();
    enqueue_fan_out_template_update(&queue, fan_out(), TemplateWork::Interactive)
        .await
        .unwrap();

    assert_eq!(
        queue.priority_of("tpl-fanout-t1-3-first"),
        Some(TEMPLATE_PRIORITY_INTERACTIVE)
    );
}
