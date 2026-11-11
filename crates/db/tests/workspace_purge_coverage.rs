//! Port of `packages/database/__tests__/workspace-purge-coverage.test.ts`, the FK-index case
//! (P2.3 spec section 5.1, row 12.1). The other cases need the purge-table constants
//! (workspace-purge unit, P3) or P2.5 tables.
//! Class C: the original reads the Drizzle snapshot; the port reads the migrated SQLite
//! catalog (`pragma_foreign_key_list`, `pragma_index_list`/`pragma_index_info`, PK columns).

mod common;

use common::{fk_column_lists, fresh_db, index_columns, indexes, primary_key, table_names};

/// The frozen `LEGACY_UNINDEXED_FKS` list (`Table:col[,col]`).
const LEGACY_UNINDEXED_FKS: &[&str] = &[
    "AdsConversionEvent:integrationWhatsappId",
    "AdsConversionEvent:integrationMessengerId",
    "AdsConversionEvent:integrationInstagramId",
    "AdsConversionRule:integrationWhatsappId",
    "AdsConversionRule:integrationFacebookAdsId",
    "AdsConversionRule:integrationMessengerId",
    "AdsConversionRule:integrationInstagramId",
    "AIFunction:triggerFlowId",
    "AIFunction:workspaceId",
    "AIMCPServer:workspaceId",
    "Appointment:conversationId",
    "Appointment:contactInboxId",
    "AppointmentCalendar:confirmationFlowId",
    "AppointmentCalendar:cancellationFlowId",
    "AppointmentCalendar:externalConnectionId",
    "AppointmentCalendarReminder:flowId",
    "AppointmentReminderDispatch:workspaceId",
    "AppointmentReminderDispatch:reminderConfigId",
    "Account:userId",
    "Account:tenantId",
    "Invitation:workspaceId",
    "Invitation:invitedBy",
    "Session:userId",
    "AutomatedResponse:folderId",
    "AutomatedResponse:flowId",
    "BotField:folderId",
    "Broadcast:integrationWhatsappId",
    "Broadcast:integrationMessengerId",
    "CoexistSyncRun:requestedByUserId",
    "ConnectSession:targetConnectionId",
    "ConnectSession:actorUserId",
    "ConnectSession:actorTokenId",
    "Connection:createdBy",
    "ContactNote:createdById",
    "Conversation:assignedUserId",
    "Conversation:assignedInboxTeamId",
    "ConversationParticipant:userId",
    "Coupon:topicId",
    "CouponTopic:createdById",
    "CustomField:folderId",
    "DynamicImage:customFieldId",
    "AuditLog:userId",
    "File:userId",
    "FacebookMarketingMessageRecipient:workspaceId",
    "Flow:folderId",
    "GoogleAdsConversionEvent:integrationGoogleAdsId",
    "Import:userId",
    "InboxTeamMember:inboxTeamId",
    "IntegrationGoogleCalendar:workspaceId",
    "IntegrationGoogleSheet:workspaceId",
    "IntegrationOpenai:aiAgentId",
    "IntegrationOutlookCalendar:workspaceId",
    "Leaderboard:referralWelcomeFlowId",
    "LeaderboardAuditLog:contactId",
    "LeaderboardAuditLog:actorUserId",
    "MagicLinkStat:linkId",
    "MessagingAdOperation:createdBy",
    "MetaCatalogSyncRun:integrationMetaCatalogId",
    "MetaCatalogSyncRun:categoryId",
    "MiniAppPublication:integrationWhatsappId",
    "MiniAppPublication:whatsappFlowId",
    "MiniAppSubmission:workspaceId",
    "MiniAppUpload:workspaceId",
    "DiscountCodeRedemption:workspaceId",
    "OrderItem:productId",
    "OrderItem:vendorId",
    "Questionnaire:triggerFlowId",
    "RefLinkStat:linkId",
    "Reflink:flowId",
    "Reflink:customFieldId",
    "Reflink:widgetLogoFileId",
    "Template:createdBy",
    "TemplateInstallation:installFolderId",
    "TemplateInstallation:installedBy",
    "UserDeviceToken:workspaceId",
    "WhatsappCall:answeredByUserId",
    "WhatsappCall:initiatedByUserId",
    "WhatsappSignupSession:ownerId",
    "WhatsappSignupSession:workspaceId",
    "WorkingHoursDateOverride:workingHoursId,workspaceId",
    "WorkingHoursMember:userId",
    "WorkingHoursMember:workingHoursId,workspaceId",
    "WorkspaceMember:userId",
];

fn has_prefix(candidate: &[Option<String>], expected: &[String]) -> bool {
    candidate.len() >= expected.len()
        && expected
            .iter()
            .zip(candidate)
            .all(|(e, c)| c.as_deref() == Some(e.as_str()))
}

/// Port of: packages/database/__tests__/workspace-purge-coverage.test.ts:188 "workspace purge coverage > does not add an unindexed foreign key or retain a fixed baseline entry"
///
/// Same algorithm: an FK is covered when an index (every index, partial ones too, and the
/// autoindexes of UNIQUE / composite PRIMARY KEY constraints) or the primary key of its
/// table starts with the FK columns. The baseline is limited to the tables that exist in
/// the migrated schema (P2.3: the 18 part-1 entries; after P2.5: the whole list).
#[tokio::test]
async fn does_not_add_an_unindexed_foreign_key_or_retain_a_fixed_baseline_entry() {
    let db = fresh_db().await;
    let tables = table_names(&db).await;

    let mut uncovered: Vec<String> = Vec::new();
    for table in &tables {
        let mut support: Vec<Vec<Option<String>>> = Vec::new();
        for index in indexes(&db, table).await {
            support.push(index_columns(&db, &index.name).await);
        }
        support.push(
            primary_key(&db, table)
                .await
                .into_iter()
                .map(Some)
                .collect(),
        );

        for fk in fk_column_lists(&db, table).await {
            if !support.iter().any(|s| has_prefix(s, &fk)) {
                uncovered.push(format!("{table}:{}", fk.join(",")));
            }
        }
    }
    uncovered.sort();

    let baseline: Vec<&str> = LEGACY_UNINDEXED_FKS
        .iter()
        .copied()
        .filter(|entry| {
            let table = entry.split_once(':').map(|(t, _)| t).unwrap_or(entry);
            tables.iter().any(|t| t == table)
        })
        .collect();
    assert_eq!(
        baseline.len(),
        83,
        "expected all 83 baseline entries, found {}",
        baseline.len()
    );

    let unexpected: Vec<&String> = uncovered
        .iter()
        .filter(|fk| !LEGACY_UNINDEXED_FKS.contains(&fk.as_str()))
        .collect();
    let mut fixed_baseline: Vec<&str> = baseline
        .iter()
        .copied()
        .filter(|fk| !uncovered.iter().any(|u| u == fk))
        .collect();
    fixed_baseline.sort_unstable();

    assert!(
        unexpected.is_empty(),
        "Add a leading FK index before adding a new foreign key: {unexpected:?}"
    );
    assert!(
        fixed_baseline.is_empty(),
        "Remove fixed FKs from LEGACY_UNINDEXED_FKS: {fixed_baseline:?}"
    );
}
