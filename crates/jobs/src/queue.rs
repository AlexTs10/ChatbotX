//! Queue names and Redis groups (spec 1.2, `packages/worker-config/src/lib/types.ts`).

/// One member of `queueNames`. `as_str()` returns the TypeScript name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum QueueName {
    Integration,
    Chat,
    AiAgent,
    Heavy,
    Schedule,
    Trigger,
    Webhook,
    Default,
    SequenceScheduler,
    Broadcast,
    Quota,
    Notification,
    CallTranscription,
    ProfileSnapshot,
    WhatsappVoipSignaling,
    Low,
    Leaderboard,
    MarketingMessage,
    Template,
}

impl QueueName {
    /// Every queue, in the order of spec 1.2.
    pub const ALL: [QueueName; 19] = [
        QueueName::Integration,
        QueueName::Chat,
        QueueName::AiAgent,
        QueueName::Heavy,
        QueueName::Schedule,
        QueueName::Trigger,
        QueueName::Webhook,
        QueueName::Default,
        QueueName::SequenceScheduler,
        QueueName::Broadcast,
        QueueName::Quota,
        QueueName::Notification,
        QueueName::CallTranscription,
        QueueName::ProfileSnapshot,
        QueueName::WhatsappVoipSignaling,
        QueueName::Low,
        QueueName::Leaderboard,
        QueueName::MarketingMessage,
        QueueName::Template,
    ];

    /// The TypeScript queue name: `"integration"`, `"aiAgent"`, `"whatsappVoipSignaling"`, ...
    pub fn as_str(self) -> &'static str {
        match self {
            QueueName::Integration => "integration",
            QueueName::Chat => "chat",
            QueueName::AiAgent => "aiAgent",
            QueueName::Heavy => "heavy",
            QueueName::Schedule => "schedule",
            QueueName::Trigger => "trigger",
            QueueName::Webhook => "webhook",
            QueueName::Default => "default",
            QueueName::SequenceScheduler => "sequenceScheduler",
            QueueName::Broadcast => "broadcast",
            QueueName::Quota => "quota",
            QueueName::Notification => "notification",
            QueueName::CallTranscription => "callTranscription",
            QueueName::ProfileSnapshot => "profileSnapshot",
            QueueName::WhatsappVoipSignaling => "whatsappVoipSignaling",
            QueueName::Low => "low",
            QueueName::Leaderboard => "leaderboard",
            QueueName::MarketingMessage => "marketingMessage",
            QueueName::Template => "template",
        }
    }

    /// Position in [`QueueName::ALL`].
    pub(crate) fn index(self) -> usize {
        QueueName::ALL
            .iter()
            .position(|q| *q == self)
            .unwrap_or_default()
    }

    /// Inverse of [`QueueName::as_str`].
    pub fn parse(name: &str) -> Option<QueueName> {
        QueueName::ALL.into_iter().find(|q| q.as_str() == name)
    }
}

/// The Redis connection group of a queue (`queueGroupByName`). Kept as
/// metadata only: there is one SQLite file (spec 6.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum QueueGroup {
    Hot,
    Bulk,
}

/// `queueGroupByName[queue]`.
pub fn queue_group(queue: QueueName) -> QueueGroup {
    match queue {
        QueueName::Integration
        | QueueName::Chat
        | QueueName::SequenceScheduler
        | QueueName::Broadcast
        | QueueName::Notification
        | QueueName::CallTranscription
        | QueueName::WhatsappVoipSignaling
        | QueueName::Low => QueueGroup::Hot,
        _ => QueueGroup::Bulk,
    }
}

impl std::fmt::Display for QueueName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}
