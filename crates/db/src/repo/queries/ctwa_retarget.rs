//! Port of `queries/contact-filter/ctwa-retarget.ts`: the shared "contacts from an ad"
//! segment predicate (conversations / leads / purchases).
//!
//! Aliases: the conversations predicate reads `ci` (`ContactInbox`); the leads /
//! purchases predicate reads `ace` (`AdsConversionEvent`).

use super::ad_referral::{ad_conversation_predicate, any_channel_ad_conversation_predicate};
use crate::repo::sql::Sql;

/// `ctwaRetargetSegments`.
pub const CTWA_RETARGET_SEGMENTS: &[&str] = &["conversations", "leads", "purchases"];

/// `CtwaSegmentPredicateInput`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CtwaSegmentPredicateInput {
    /// `conversations`, `leads` or `purchases`.
    pub segment: String,
    pub ad_id: Option<String>,
    pub since: i64,
    pub until: i64,
    pub workspace_id: Option<i64>,
    pub integration_whatsapp_id: Option<i64>,
    /// `AdsConversionChannel` (`whatsapp`, `facebook`, `messenger`, `instagram`).
    pub channel: Option<String>,
    pub integration_messenger_id: Option<i64>,
    pub integration_instagram_id: Option<i64>,
}

fn integration_table(channel: &str) -> &'static str {
    match channel {
        "whatsapp" => "IntegrationWhatsapp",
        "messenger" => "IntegrationMessenger",
        _ => "IntegrationInstagram",
    }
}

fn integration_inbox_exists(
    channel: &str,
    integration_id: i64,
    workspace_id: Option<i64>,
    ci: &str,
) -> Sql {
    let table = integration_table(channel);
    let mut out = Sql::bind(
        format!("EXISTS (SELECT 1 FROM \"{table}\" AS i WHERE i.\"id\" = "),
        integration_id,
    );
    if let Some(ws) = workspace_id {
        out.push(" AND i.\"workspaceId\" = ");
        out.push_bind(ws);
    }
    out.push(&format!(" AND i.\"inboxId\" = {ci}.\"inboxId\")"));
    out
}

/// `buildCtwaSegmentPredicate(input)` (conversations over `ci`, events over `ace`).
pub fn build_ctwa_segment_predicate(input: &CtwaSegmentPredicateInput) -> Sql {
    if input.segment == "conversations" {
        return build_conversations_predicate(input, "ci");
    }
    let event_type = if input.segment == "leads" {
        "lead"
    } else {
        "purchase"
    };
    let mut parts = vec![
        Some(Sql::bind("ace.\"eventType\" = ", event_type.to_owned())),
        Some(Sql::bind("ace.\"occurredAt\" >= ", input.since)),
        Some(Sql::bind("ace.\"occurredAt\" <= ", input.until)),
    ];
    if let Some(ad) = input.ad_id.as_deref().filter(|a| !a.is_empty()) {
        parts.push(Some(Sql::bind("ace.\"adId\" = ", ad.to_owned())));
    }
    if let Some(ws) = input.workspace_id {
        parts.push(Some(Sql::bind("ace.\"workspaceId\" = ", ws)));
    }
    let channel = if let Some(c) = &input.channel {
        Some(c.clone())
    } else if input.integration_whatsapp_id.is_some() {
        Some("whatsapp".to_owned())
    } else if input.integration_messenger_id.is_some() {
        Some("messenger".to_owned())
    } else if input.integration_instagram_id.is_some() {
        Some("instagram".to_owned())
    } else {
        None
    };
    if let Some(c) = channel {
        parts.push(Some(Sql::bind("ace.\"channel\" = ", c)));
    }
    if let Some(id) = input.integration_whatsapp_id {
        parts.push(Some(Sql::bind("ace.\"integrationWhatsappId\" = ", id)));
    }
    if let Some(id) = input.integration_messenger_id {
        parts.push(Some(Sql::bind("ace.\"integrationMessengerId\" = ", id)));
    }
    if let Some(id) = input.integration_instagram_id {
        parts.push(Some(Sql::bind("ace.\"integrationInstagramId\" = ", id)));
    }
    Sql::and(parts)
}

/// The conversations predicate over the `ContactInbox` alias `ci`.
pub fn build_conversations_predicate(input: &CtwaSegmentPredicateInput, ci: &str) -> Sql {
    if input.channel.as_deref() == Some("facebook") {
        return Sql::raw("0");
    }
    let ch = input.channel.as_deref();
    let is_whatsapp =
        ch == Some("whatsapp") || (ch.is_none() && input.integration_whatsapp_id.is_some());
    let is_messenger =
        ch == Some("messenger") || (ch.is_none() && input.integration_messenger_id.is_some());
    let is_instagram =
        ch == Some("instagram") || (ch.is_none() && input.integration_instagram_id.is_some());
    let scoped = if is_whatsapp {
        Some("whatsapp")
    } else if is_messenger {
        Some("messenger")
    } else if is_instagram {
        Some("instagram")
    } else {
        None
    };
    let referral = match scoped {
        Some(c) => ad_conversation_predicate(ci, c),
        None => any_channel_ad_conversation_predicate(ci),
    };
    let mut parts = vec![
        Some(referral),
        Some(Sql::bind(
            format!("{ci}.\"firstInteractionAt\" >= "),
            input.since,
        )),
        Some(Sql::bind(
            format!("{ci}.\"firstInteractionAt\" <= "),
            input.until,
        )),
    ];
    if let Some(ad) = input.ad_id.as_deref().filter(|a| !a.is_empty()) {
        parts.push(Some(Sql::bind(
            format!("{ci}.\"referral\"->>'adId' = "),
            ad.to_owned(),
        )));
    }
    if let Some(c) = scoped.filter(|c| *c != "whatsapp") {
        parts.push(Some(Sql::bind(
            format!("{ci}.\"channel\" = "),
            c.to_owned(),
        )));
    }
    if let (true, Some(id)) = (is_whatsapp, input.integration_whatsapp_id) {
        parts.push(Some(integration_inbox_exists(
            "whatsapp",
            id,
            input.workspace_id,
            ci,
        )));
    } else if let (true, Some(id)) = (is_messenger, input.integration_messenger_id) {
        parts.push(Some(integration_inbox_exists(
            "messenger",
            id,
            input.workspace_id,
            ci,
        )));
    } else if let (true, Some(id)) = (is_instagram, input.integration_instagram_id) {
        parts.push(Some(integration_inbox_exists(
            "instagram",
            id,
            input.workspace_id,
            ci,
        )));
    }
    Sql::and(parts)
}

/// `buildCtwaSegmentContactExists(input, contactIdColumn)`: the `SELECT 1 ...` body of an
/// `EXISTS` for the contact id SQL `contact_id_sql`.
pub fn build_ctwa_segment_contact_exists(
    input: &CtwaSegmentPredicateInput,
    contact_id_sql: &str,
) -> Sql {
    let predicate = build_ctwa_segment_predicate(input);
    let mut out = if input.segment == "conversations" {
        Sql::raw(format!(
            "SELECT 1 FROM \"ContactInbox\" AS ci WHERE ci.\"contactId\" = {contact_id_sql} AND "
        ))
    } else {
        Sql::raw(format!(
            "SELECT 1 FROM \"AdsConversionEvent\" AS ace INNER JOIN \"ContactInbox\" AS ci ON ci.\"id\" = ace.\"contactInboxId\" WHERE ci.\"contactId\" = {contact_id_sql} AND "
        ))
    };
    out.append(predicate);
    out
}

/// `ctwaRetargetDateRange(since, until)`: `since` at 00:00:00.000Z, `until` at
/// 23:59:59.999Z (unix ms). `None` for a malformed date.
pub fn ctwa_retarget_date_range(since: &str, until: &str) -> Option<(i64, i64)> {
    let start = crate::repo::parse_iso(&format!("{since}T00:00:00.000Z"))?;
    let end = crate::repo::parse_iso(&format!("{until}T23:59:59.999Z"))?;
    Some((start, end))
}
