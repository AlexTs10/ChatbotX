//! `/rpc/*`: the builder's oRPC RPC endpoint (`app/rpc/[[...rest]]/route.ts`)
//! with the session auth of `authorizedAPI` (`middlewares/auth.ts`
//! `authMiddleware`) and, when the input carries a `workspaceId`,
//! `workspaceAuthorizedMidddleware`.
//!
//! The procedures themselves are the domain ports (P3.6+): after auth a call
//! answers the RPC-encoded `NOT_FOUND "Operation not ported yet"`.

use std::sync::Arc;

use axum::Router;

use crate::session::AuthState;

/// The top-level keys of the builder router (`routers/index.ts`).
pub const RPC_ROUTER_KEYS: &[&str] = &[
    "adsAPI",
    "adsCampaignAPI",
    "googleAdsAPI",
    "appointmentCalendarsAPI",
    "appointmentsAPI",
    "aiMcpServerAPIs",
    "aiAgentsAPI",
    "decisionProfilesAPI",
    "broadcastAPIs",
    "conversationsAPI",
    "miniAppsAPI",
    "couponsAPI",
    "pipelinesAPI",
    "pollsAPI",
    "placesNearMeAPI",
    "leaderboardsAPI",
    "deviceTokensAPI",
    "dynamicImagesAPI",
    "connectionsAPI",
    "connectSessionsAPI",
    "emailTopicsAPI",
    "tagsAPI",
    "contactFilterAPI",
    "customFieldsAPI",
    "flowsAPI",
    "contactsAPIs",
    "contactNotesAPI",
    "contactSequencesAPI",
    "contactScanAPIs",
    "aiHandoverAPIs",
    "channelPostAPIs",
    "botFieldAPIs",
    "integrationActiveCampaignAPI",
    "integrationFacebookAdsAPI",
    "integrationDripAPI",
    "integrationGetResponseAPI",
    "integrationInstagramAPIs",
    "integrationKlaviyoAPI",
    "integrationMailchimpAPI",
    "integrationMailerLiteAPI",
    "integrationMessengerAPIs",
    "integrationMoosendAPI",
    "integrationSmtpAPI",
    "integrationSendGridAPI",
    "integrationWhatsappAPIs",
    "whatsappMessageTemplateAPIs",
    "whatsappFlowAPIs",
    "messengerMessageTemplateAPIs",
    "savedRepliesAPI",
    "commentAutomationsAPI",
    "fbCommentsAPI",
    "igCommentsAPI",
    "threadsCommentsAPI",
    "igStoriesAPI",
    "facebookLeadAdsAPI",
    "facebookMarketingMessagesAPI",
    "responseSlaAPI",
    "workingHoursAPI",
    "sequencesAPI",
    "aiFilesAPI",
    "inboxesAPI",
    "spreadsheetsAPI",
    "workspaceMembersAPI",
    "inboxTeamsAPI",
    "foldersAPI",
    "messagesAPI",
    "personasAPIs",
    "workspacesAPI",
    "aiFunctionsAPI",
    "platformCredentialsAPI",
    "productsAPI",
    "productCategoriesAPI",
    "questionnairesAPI",
    "refLinksAPI",
    "realtimeAPI",
    "userPersistentMenusAPI",
    "templatesAPI",
    "analyticsRoutes",
    "mediaLibraryAPI",
];

/// The router of `/rpc/*`.
pub fn rpc_router(state: Arc<AuthState>) -> Router {
    let _ = state;
    todo!("P3.5d")
}
