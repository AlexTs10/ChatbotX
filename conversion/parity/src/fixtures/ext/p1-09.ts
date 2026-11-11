// Fixture extension of unit P1.9 (flows, folders, googleAds, igComments,
// igStories, inboundWebhookChannels, inboundWebhooks, inboxTeams). Setup that
// has no public HTTP path. Stack-neutral types only; each stack writes the
// same logical rows the app writes (see stacks/<stack>/ext/p1-09.ts).
// Setup only: no method returns data that a test asserts on.

// ---------------------------------------------------------------------------
// Section A: flows + folders
// ---------------------------------------------------------------------------

export interface P109FlowsExt {
  /**
   * A `Folder` row as `folderService.create` writes it (top level, any
   * folder type, e.g. `flow`, which the public folders API cannot create).
   */
  createFolder(input: { workspaceId: string; name: string; folderType: string }): Promise<{ folderId: string }>
}

// ---------------------------------------------------------------------------
// Section C: igComments, igStories, googleAds, inboxTeams
// (Part B, inbound webhooks, needs no fixture: all its setup is public HTTP.)
// ---------------------------------------------------------------------------

export type P109GoogleAdsEventRow = {
  conversionActionId: string
  conversionActionName: string | null
  conversionActionCategory: string
  channel: "whatsapp" | "messenger" | "zalo"
  /** Terminal statuses only, so no background job picks the row up. */
  status: "processed" | "failed" | "skipped_no_account" | "skipped_expired"
  failureStage?: "delivery" | "processing" | "timeout" | null
  processingStatus?: "success" | "failed" | null
  error?: string | null
  clickIdType: "gclid" | "gbraid"
  /** 10-512 characters. */
  clickId: string
  occurredAt: string
  sentAt?: string | null
  /** Decimal string; set together with `currency`. */
  value?: string | null
  currency?: string | null
  transactionId: string
  requestId?: string | null
  uploadMethod: "dataManager" | "legacy"
  /** `GoogleAdsEventOptions` document (v1/v2) or null. */
  options?: unknown
}

export interface P109IgExt {
  /**
   * Connected Instagram account: `Inbox` (channel "instagram", sourceId =
   * igId), `IntegrationInstagram` (`type` "instagram" = Instagram Login,
   * "facebook" = via Facebook Login; plain JSON oauth2 auth with access token
   * `ig-token-<igId>` and metadata {igId, pageId, version "v23.0"}) and its
   * `Connection` row.
   */
  createInstagramChannel(input: {
    workspaceId: string
    igId: string
    pageId: string
    name: string
    username: string
    type: "instagram" | "facebook"
  }): Promise<{ integrationId: string; inboxId: string }>
  /** An `Inbox` row only (no integration behind it). */
  createInbox(input: { workspaceId: string; channel: string; name: string; sourceId: string }): Promise<{
    inboxId: string
  }>
  /** A `WorkspaceMember` (role agent) of an existing user, as an accepted invitation writes it. */
  addWorkspaceMember(input: { workspaceId: string; userId: string }): Promise<{ memberId: string }>
  /**
   * A Google Ads connection as the connection engine + setup service write
   * it: `Integration` (type googleAds), `IntegrationGoogleAds` (plain JSON
   * oauth2 auth, `metadata.uploadMethod`) and its `Connection` row (kind
   * integration, provider googleAds, sourceId = customerId).
   */
  createGoogleAdsConnection(input: {
    workspaceId: string
    customerId: string
    descriptiveName: string | null
    currencyCode: string | null
    uploadMethod?: "dataManager" | "legacy"
    connectionStatus: "connected" | "degraded" | "needs_reauth" | "paused" | "disconnected"
    conversionCustomerId: string | null
    acceptedCustomerDataTerms: boolean | null
    conversionActions: Array<{
      id: string
      name: string
      category: string
      status: string
      countingType: string
      attributionModel?: string | null
    }> | null
    conversionActionsSyncedAt: string | null
    setupError: string | null
  }): Promise<{ integrationGoogleAdsId: string }>
  /** `GoogleAdsSettings` row (raw `settings` document, e.g. {version:1, consent}). */
  setGoogleAdsSettings(input: { workspaceId: string; settings: unknown }): Promise<void>
  /** `GoogleAdsConversionEvent` rows as `recordConversion` + delivery leave them. */
  insertGoogleAdsEvents(input: {
    workspaceId: string
    integrationGoogleAdsId: string | null
    customerId: string
    rows: P109GoogleAdsEventRow[]
  }): Promise<{ ids: string[] }>
}

export type P109Fixtures = P109FlowsExt &
  P109IgExt & {
    close(): Promise<void>
  }
