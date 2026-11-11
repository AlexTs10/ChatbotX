//! The `marketingMessage` queue (`queues/marketing-message`).

/// `MarketingMessageJobAction`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MarketingMessageJobAction {
    ProcessCampaign,
}

impl MarketingMessageJobAction {
    /// `"processCampaign"`.
    pub fn as_str(self) -> &'static str {
        todo!()
    }
}

/// `mm-campaign-<step>-<campaignRowId>`.
pub fn marketing_message_campaign_job_id(campaign_row_id: &str, step: &str) -> String {
    todo!()
}
