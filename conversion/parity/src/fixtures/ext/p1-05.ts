// Fixture extension of unit P1.5 (public API: channels, commentAutomations,
// connectSessions, connectionProviders, connections, contactScans). Setup with
// no public HTTP path only; assertions stay black-box. Each stack implements
// `create()` in `stacks/<stack>/ext/p1-05.ts`.
// Load with `await loadFixtureExt<P105Fixtures>("p1-05")`.
//
// Times are ISO-8601 strings (UTC instants). Ids are decimal strings.

export type CreatedChannel = {
  /** Channel integration id (IntegrationMessenger/-Instagram/-Threads/-Tiktok.id). */
  integrationId: string
  inboxId: string
  /** `Connection.id` of the channel. */
  connectionId: string
}

export type CommentAutomationType = "messenger" | "instagram" | "instagramFacebook" | "threads" | "tiktok"

export type CreateCommentAutomationInput = {
  workspaceId: string
  name: string
  type: CommentAutomationType
  /** Account scope (`CommentAutomation.inboxId`). Default null (every account). */
  inboxId?: string | null
  /** Default true. */
  isActive?: boolean
  /** `post` column. Default `{type:"all", value:[]}`. */
  post?: { type: "published" | "ads" | "reels" | "postIds" | "all" | "live"; value: string[] }
  /** `publicReply` column. Default `{type:"none", value:null}`. */
  publicReply?: { type: "text" | "none"; value: unknown }
  /** `privateReply` column. Default `{type:"none", value:null}`. */
  privateReply?: { type: "text" | "none"; value: unknown }
  /** "HH:mm" schedule window (`startTime`/`endTime`). Default null. */
  startTime?: string | null
  endTime?: string | null
}

export type ModerationLogRow = {
  workspaceId: string
  automationId: string
  contactId?: string | null
  contactInboxId?: string | null
  postId: string
  commentId: string
  commentText?: string | null
  source: "rule" | "ai"
  rule?: "all" | "phoneNumber" | "link" | "keyword" | "image" | "video" | "gif" | "emoji" | null
  matchedKeyword?: string | null
  categories?: Array<"hate" | "offensive" | "harassment" | "negative" | "spam">
  reason?: string | null
  action: "hide" | "delete"
  /** Default "pending". */
  status?: "pending" | "done" | "failed"
  errorDetail?: string | null
  occurredAt: string
}

export type P105Fixtures = {
  /**
   * Connected Messenger Page as the connect flow writes it: `Inbox` (channel
   * "messenger", sourceId = pageId), `IntegrationMessenger` (encrypted page
   * auth, page token `page-token-<pageId>`) and its `Connection` row
   * (kind channel, status connected).
   */
  createMessengerChannel(input: { workspaceId: string; pageId: string; name: string }): Promise<CreatedChannel>
  /**
   * Connected Instagram account: `Inbox` (channel "instagram", sourceId =
   * igId), `IntegrationInstagram` (`type` "instagram" = Instagram Login,
   * "facebook" = via Facebook Login) and its `Connection` row.
   */
  createInstagramChannel(input: {
    workspaceId: string
    igId: string
    pageId: string
    name: string
    username: string
    type: "instagram" | "facebook"
  }): Promise<CreatedChannel>
  /** An `Inbox` row only (no integration behind it). */
  createInbox(input: {
    workspaceId: string
    channel: string
    name: string
    sourceId: string
    /** Default "connected". */
    status?: "connected" | "disconnected"
  }): Promise<{ inboxId: string }>
  /** `Contact` + `ContactInbox` + DM `Conversation` (the rows `createContactWithInbox` writes). */
  createContact(input: {
    workspaceId: string
    inboxId: string
    channel: string
    sourceId: string
    firstName?: string | null
    lastName?: string | null
  }): Promise<{ contactId: string; contactInboxId: string; conversationId: string }>
  /** `CommentAutomation` row (the row `fbComments.create` & co. write). */
  createCommentAutomation(input: CreateCommentAutomationInput): Promise<{ automationId: string }>
  /** `CommentModerationLog` rows (the rows the moderation recorder writes); ids in input order. */
  insertModerationLogs(rows: ModerationLogRow[]): Promise<{ ids: string[] }>
  /**
   * User-owned `PlatformCredential` of type "messenger" (the row the
   * platform-credential settings page writes), encrypted with the
   * owner-bound AAD.
   */
  upsertMessengerCredential(input: {
    userId: string
    clientId: string
    clientSecret: string
    version: string
    verifyToken: string
  }): Promise<{ credentialId: string }>
  /** Moves a connect session's `expiresAt` into the past (simulates its TTL running out). */
  expireConnectSession(input: { sessionId: string }): Promise<void>
  close(): Promise<void>
}
