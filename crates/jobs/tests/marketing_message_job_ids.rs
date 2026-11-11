//! Port of `packages/worker-config/__tests__/marketing-message-job-ids.test.ts` (spec 5.1).

use chatbotx_jobs::QueueName;
use chatbotx_jobs::queues::marketing_message::{
    MarketingMessageJobAction, marketing_message_campaign_job_id,
};

/// Port of: packages/worker-config/__tests__/marketing-message-job-ids.test.ts:9 "marketing message queue > has its own queue name, separate from the shared schedule queue"
#[test]
fn has_its_own_queue_name_separate_from_the_shared_schedule() {
    assert_eq!(QueueName::MarketingMessage.as_str(), "marketingMessage");
    assert_eq!(
        MarketingMessageJobAction::ProcessCampaign.as_str(),
        "processCampaign"
    );
}

/// Port of: packages/worker-config/__tests__/marketing-message-job-ids.test.ts:14 "marketing message queue > the per-campaign job id is deterministic per step and BullMQ-safe"
#[test]
fn the_per_campaign_job_id_is_deterministic_per_step_and() {
    let id = marketing_message_campaign_job_id("c1", "dispatch");
    assert_eq!(id, "mm-campaign-dispatch-c1");
    assert!(!id.contains(':'));
}
