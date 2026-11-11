//! The `default` queue (`queues/default`): job actions.

/// `DefaultJobAction` (the members the ported tests use).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DefaultJobAction {
    ProcessInboundWebhookRequest,
}

impl DefaultJobAction {
    /// Every default-queue action (`Object.values(DefaultJobAction)`).
    pub fn all() -> &'static [DefaultJobAction] {
        todo!()
    }

    /// `"processInboundWebhookRequest"`, ...
    pub fn as_str(self) -> &'static str {
        todo!()
    }
}
