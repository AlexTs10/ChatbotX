// Fixture extension of unit P1.8 (dynamicImages, ecommerceAnalytics,
// ecommerceSettings, emailTopics, errorLogs, externalWebhooks,
// facebookLeadAds, facebookMarketingMessages, fbComments). Setup that has no
// public HTTP path. Stack-neutral types only; each stack writes the same
// logical rows the app writes (see stacks/<stack>/ext/p1-08.ts).

export type CreatedMessengerChannel = {
  integrationId: string
  inboxId: string
  /** The page access token stored in the integration auth (sent to Graph). */
  accessToken: string
}

export type ErrorLogRow = {
  /** Provider slug written to `ErrorLog.action`, e.g. "messenger", "smtp". */
  action: string
  detail: string
  httpCode: string | null
  createdAt: Date
  /** Channel-side contact id; never returned by the public route. */
  sourceId?: string | null
  /** Developer-only stack; never returned by the public route. */
  stackTrace?: string | null
}

export type FolderType = "emailTopic" | "fbComment"

export type OrderItemRow = {
  /** Null: the product was deleted (top products group such lines by name). */
  productId: null
  name: string
  quantity: number
  unitPrice: number
  lineTotal: number
  vendorName: string | null
}

export type OrderRow = {
  number: number
  status: "pending" | "accepted" | "shipped" | "concluded" | "cancelled"
  currency: string
  total: number
  createdAt: Date
  items: OrderItemRow[]
}

// ---- p1-08 marketing messages: begin ----

export type MmCampaignStatus =
  | "notSent"
  | "scheduled"
  | "sending"
  | "waitingNextDay"
  | "paused"
  | "stopped"
  | "completed"
  | "failed"

export type MmCampaignInput = {
  workspaceId: string
  name: string
  pageId: string
  /** `act_<digits>`. */
  adAccountId: string
  currency: string
  currencyOffset: number
  budgetType: "daily" | "lifetime"
  budgetMinorUnits: number
  content: unknown
  targetType?: "optIn" | "customAudience"
  audienceId?: string | null
  /** The Meta message campaign (ad) id. */
  campaignId: string
  status: MmCampaignStatus
  statusReason?: string | null
  startsAt?: Date | null
  firstSentAt?: Date | null
  metaEndsAt?: Date | null
  timezoneName?: string | null
  adSetId?: string | null
  tokenCount?: number | null
  createdAt?: Date
}

export type MmAudienceInput = {
  workspaceId: string
  customAudienceId: string
  adAccountId: string
  pageId: string
  name: string
  description?: string | null
  rowsUploaded: number
  readyAt: Date | null
  createdAt?: Date
}

export type MmRecipientInput = {
  status: "pending" | "unconfirmed" | "sent" | "delivered" | "failed"
  errorCode?: number | null
  errorMessage?: string | null
}

export interface P108MmFixtures {
  /**
   * User-owned `PlatformCredential` of type "messenger" (the Facebook app the
   * grant's token belongs to), encrypted with the owner-bound AAD
   * `user:<userId>:messenger:false`.
   */
  createMessengerAppCredential(input: {
    userId: string
    clientId: string
    clientSecret: string
    version: string
  }): Promise<{ credentialId: string }>
  /**
   * The workspace's Marketing Messages grant (`FacebookMarketingMessagesAuth`)
   * as `facebookMarketingMessagesService.upsertAuth` writes it: auth =
   * encryptObject({authType:"custom", accessToken, version}) without AAD.
   */
  createMarketingMessagesAuth(input: {
    workspaceId: string
    accessToken: string
    version?: string
    status?: "active" | "invalid"
    tokenExpiresAt?: Date | null
    facebookUserId?: string | null
  }): Promise<{ authId: string }>
  /** A `FacebookMarketingMessage` row (a campaign in any delivery state). */
  createMarketingMessage(input: MmCampaignInput): Promise<{ id: string }>
  /** A `FacebookMarketingMessageAudience` row. */
  createMarketingMessageAudience(input: MmAudienceInput): Promise<{ id: string }>
  /** `FacebookMarketingMessageRecipient` rows of one campaign. */
  insertMarketingMessageRecipients(input: {
    workspaceId: string
    campaignRowId: string
    rows: MmRecipientInput[]
  }): Promise<void>
}

// ---- p1-08 marketing messages: end ----

export interface P108Fixtures extends P108MmFixtures {
  /**
   * A connected Messenger Page as the OAuth connect writes it: `Inbox`
   * (channel "messenger", sourceId = pageId) and `IntegrationMessenger`
   * (oauth2 auth with client id/secret, page token and `metadata.version`).
   */
  createMessengerChannel(input: {
    workspaceId: string
    pageId: string
    name: string
    /** Default `page-token-<pageId>`. */
    accessToken?: string
    /** Default true; false stores an auth without the app client id/secret. */
    withAppCredentials?: boolean
  }): Promise<CreatedMessengerChannel>
  /** A folder of one folder type (`folderService.create`, no parent). */
  createFolder(input: { workspaceId: string; name: string; folderType: FolderType }): Promise<{ folderId: string }>
  /** `ErrorLog` rows as the error-log event-bus writer inserts them. */
  insertErrorLogs(input: { workspaceId: string; rows: ErrorLogRow[] }): Promise<{ ids: string[] }>
  /**
   * A contact reached on one inbox, as `createContactWithInbox` writes it:
   * `Contact` + `ContactInbox` (sourceId = the channel-side id) + `Conversation`.
   */
  createContact(input: {
    workspaceId: string
    inboxId: string
    channel: string
    sourceId: string
    firstName: string
  }): Promise<{ contactId: string; contactInboxId: string; conversationId: string }>
  /** A contact's custom field value (`ContactCustomField`), as `contactCustomFieldService.setValues` writes it. */
  setCustomFieldValue(input: { contactId: string; customFieldId: string; value: string }): Promise<void>
  /**
   * Placed orders with a chosen `createdAt` (`Order` + `OrderItem`, the
   * columns `orderService.place` fills; source "api", cash on delivery).
   * `number` is the per-workspace order number (unique).
   */
  insertOrders(input: { workspaceId: string; orders: OrderRow[] }): Promise<{ ids: string[] }>
  close(): Promise<void>
}
