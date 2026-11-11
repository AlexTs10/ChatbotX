// Fixture extension of unit P1.2 (public API: ads, aiAgents, aiFiles,
// aiFunctions). Setup with no public HTTP path only; assertions stay
// black-box. Each stack implements `create()` in `stacks/<stack>/ext/p1-02.ts`.
// Load with `await loadFixtureExt<P102Fixtures>("p1-02")`.

// ---- ads: begin ----

export type MessagingAdChannel = "messenger" | "instagram" | "whatsapp"

export type CreatedChannel = {
  /** Channel integration id (IntegrationMessenger/-Whatsapp/-Instagram.id). */
  integrationId: string
  inboxId: string
}

export type P102AdsFixtures = {
  /**
   * Connected Messenger Page: an `Inbox` (channel "messenger", sourceId =
   * pageId) and its `IntegrationMessenger` row (encrypted page auth).
   */
  createMessengerChannel(input: {
    workspaceId: string
    pageId: string
    name: string
  }): Promise<CreatedChannel>
  /**
   * Connected WhatsApp number: an `Inbox` (channel "whatsapp", sourceId =
   * phoneNumberId) and its `IntegrationWhatsapp` row.
   */
  createWhatsappChannel(input: {
    workspaceId: string
    phoneNumberId: string
    wabaId: string
    name: string
    /** E.164, e.g. "+15550001111". Default "". */
    displayPhoneNumber?: string
  }): Promise<CreatedChannel>
  /**
   * Connected Instagram account: an `Inbox` (channel "instagram", sourceId =
   * igId) and its `IntegrationInstagram` row.
   */
  createInstagramChannel(input: {
    workspaceId: string
    igId: string
    pageId: string
    name: string
    username: string
  }): Promise<CreatedChannel>
  /**
   * Per-integration messaging-ads connection (`MessagingAdsConnection`), the
   * row the OAuth callback writes: auth = encrypted
   * `{authType:"custom", accessToken, version?}`.
   */
  connectMessagingAds(input: {
    workspaceId: string
    channel: MessagingAdChannel
    integrationId: string
    accessToken: string
    /** Default "active". */
    status?: "active" | "invalid"
    /** Stored auth is not a valid encrypted blob (decrypt fails). */
    corruptAuth?: boolean
  }): Promise<{ connectionId: string }>
  /**
   * Workspace-wide Facebook Ads connection: `Integration` (type
   * "facebookAds") + `IntegrationFacebookAds` (encrypted auth, status active),
   * the rows `integrationFacebookAdsService.upsert` writes.
   */
  connectWorkspaceFacebookAds(input: {
    workspaceId: string
    accessToken: string
  }): Promise<{ integrationFacebookAdsId: string }>
  /** A `Contact` and one `ContactInbox` in `inboxId`. */
  createContactInbox(input: {
    workspaceId: string
    inboxId: string
    channel: MessagingAdChannel
    sourceId: string
    firstName?: string
    lastName?: string
    email?: string
    phoneNumber?: string
    /** ISO time; ContactInbox.firstInteractionAt. */
    firstInteractionAt?: string
    /** ContactInbox.referral jsonb, e.g. {adId, source:"ADS"} or {ctwaClid, adId}. */
    referral?: Record<string, unknown>
  }): Promise<{ contactId: string; contactInboxId: string }>
  /** One `AdsConversionEvent` row. */
  createConversionEvent(input: {
    workspaceId: string
    channel: MessagingAdChannel
    integrationId: string
    eventType: "lead" | "purchase"
    source?: "automatic" | "rule" | "trigger"
    /** ISO time. */
    occurredAt: string
    sourceEventId: string
    adId?: string | null
    contactInboxId?: string | null
    referralEventId?: string | null
    /** Required for whatsapp (DB check). */
    ctwaClid?: string | null
    wabaId?: string | null
    currency?: string | null
    /** Decimal string, e.g. "12.50". */
    value?: string | null
    orderId?: string | null
    contents?: Array<{ id: string; quantity: number; itemPrice: number }> | null
    capiStatus?: "pending" | "sent" | "failed" | "skipped_no_scope" | "skipped_region"
  }): Promise<{ eventId: string }>
  /** One `ContactReferralEvent` (type "ad", provider "meta"). */
  createReferralEvent(input: {
    workspaceId: string
    contactId: string
    contactInboxId: string
    channel: MessagingAdChannel
    adId: string
    /** ISO time. */
    occurredAt: string
    adTitle?: string | null
    dedupKey: string
  }): Promise<{ referralEventId: string }>
}

// ---- ads: end ----

// ---- ai: begin ----

// biome-ignore lint/complexity/noBannedTypes: filled by the AI part of P1.2.
export type P102AiFixtures = {}

// ---- ai: end ----

export type P102Fixtures = P102AdsFixtures & P102AiFixtures
