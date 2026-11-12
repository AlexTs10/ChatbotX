//! `packages/utils/src/broadcast.ts`.

string_enum! {
    /// `broadcastFlowTypes`.
    pub enum BroadcastFlowType {
        Flow = "flow",
        Template = "template",
    }
}

string_enum! {
    /// `broadcastSubactions`.
    pub enum BroadcastSubaction {
        AllContacts = "allContacts",
        MessengerActiveContacts = "messengerActiveContacts",
        MessengerTemplateMessage = "messengerTemplateMessage",
        WhatsappTemplateMessage = "whatsappTemplateMessage",
        WhatsappWithin24Hours = "whatsappWithin24Hours",
        InstagramActiveContacts = "instagramActiveContacts",
        TelegramAllContacts = "telegramAllContacts",
        TiktokActiveContacts = "tiktokActiveContacts",
    }
}

/// `templateBroadcastSubactions`: the subactions that send a template.
pub const TEMPLATE_BROADCAST_SUBACTIONS: &[BroadcastSubaction] = &[
    BroadcastSubaction::MessengerTemplateMessage,
    BroadcastSubaction::WhatsappTemplateMessage,
];

/// `isTemplateBroadcastSubaction`.
pub fn is_template_broadcast_subaction(subaction: Option<BroadcastSubaction>) -> bool {
    subaction.is_some_and(|s| TEMPLATE_BROADCAST_SUBACTIONS.contains(&s))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_two_template_subactions_send_templates() {
        let templates: Vec<_> = BroadcastSubaction::ALL
            .iter()
            .filter(|s| is_template_broadcast_subaction(Some(**s)))
            .map(|s| s.as_str())
            .collect();
        assert_eq!(
            templates,
            ["messengerTemplateMessage", "whatsappTemplateMessage"]
        );
        assert!(!is_template_broadcast_subaction(None));
        assert_eq!(
            BroadcastFlowType::parse("template"),
            Some(BroadcastFlowType::Template)
        );
    }
}
