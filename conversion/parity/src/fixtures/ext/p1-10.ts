// Fixture extension of unit P1.10 (public API namespaces `inboxes`,
// `instagramChannels`, `integrations`, `keywords`, `leaderboards`). Setup that
// has no public HTTP path only; every assertion stays black-box. Each stack
// implements `create()` in `stacks/<stack>/ext/p1-10.ts`.
// Load with `await loadFixtureExt<P110Fixtures>("p1-10")`.
//
// Ids are decimal strings. Rows are the ones the app writes itself (see the
// stack file for the source of each row). No method returns data that a test
// asserts on, except the ids of the rows it wrote.

export type P110InboxInput = {
  workspaceId: string
  /** `Inbox.channel` (plain text, e.g. "zalo", "telegram", "webchat"). */
  channel: string
  name: string
  /** Channel-side id. Default: unique. */
  sourceId?: string
  /** `Inbox.status`. Default "connected". */
  status?: "connected" | "disconnected"
}

export type P110InstagramInput = {
  workspaceId: string
  igId: string
  pageId: string
  name: string
  username: string
  /** "instagram" = Instagram Login, "facebook" = Instagram via Facebook Login. */
  type: "instagram" | "facebook"
  /** Default false. */
  hasCapiScope?: boolean
  /** Default null. */
  datasetId?: string | null
  /** Default null. */
  capiTestEventCode?: string | null
  /** Sets `capiDisconnectedAt` to now. Default false. */
  capiDisconnected?: boolean
  /** Default null. */
  tokenRefreshError?: string | null
}

export type P110ContactInput = {
  workspaceId: string
  firstName?: string | null
  lastName?: string | null
  phoneNumber?: string | null
  avatar?: string | null
  /** One ContactInbox row per entry. Default: none. */
  inboxes?: Array<{ inboxId: string; channel: string; sourceId?: string }>
  /** One Conversation row (column defaults). Default true. */
  conversation?: boolean
}

export interface P110Fixtures {
  /** An inbox row with no integration behind it (no worker job can reach a third party). */
  createInbox(input: P110InboxInput): Promise<{ inboxId: string }>
  /**
   * A connected Instagram account: `Inbox` (channel "instagram", sourceId =
   * igId), `IntegrationInstagram` (plain JSON oauth2 auth: clientId
   * "parity-client", clientSecret "parity-secret", access token
   * `ig-token-<igId>`, metadata {igId, pageId, version "v23.0"}) and its
   * `Connection` row.
   */
  createInstagramChannel(input: P110InstagramInput): Promise<{ integrationId: string; inboxId: string }>
  /**
   * A Messenger page of the workspace (`Inbox` + `IntegrationMessenger`, auth
   * clientId "parity-client"), the "Facebook Page still connected to
   * Messenger" sibling of an Instagram-via-Facebook account.
   */
  createMessengerPage(input: { workspaceId: string; pageId: string; name: string }): Promise<{
    integrationId: string
    inboxId: string
  }>
  /** A Zalo OA (`Inbox` + `IntegrationZalo`), optionally with a failed token refresh. */
  createZaloIntegration(input: {
    workspaceId: string
    name: string
    oaId: string
    tokenRefreshError?: string | null
  }): Promise<{ integrationId: string; inboxId: string }>
  /** A bare `Integration` row (e.g. type "metaCatalog" with no catalog behind it). */
  createIntegration(input: { workspaceId: string; integrationType: string }): Promise<{ integrationId: string }>
  /** Contact + its ContactInbox rows + (default) one Conversation. */
  createContact(input: P110ContactInput): Promise<{
    contactId: string
    contactInboxIds: string[]
    conversationId: string | null
  }>
  /** A `TelegramChat` row (the bot's membership in a group/channel, as `my_chat_member` records it). */
  createTelegramChat(input: {
    workspaceId: string
    inboxId: string
    chatId: string
    title: string | null
    type: "private" | "group" | "supergroup" | "channel"
    botStatus: "creator" | "administrator" | "member" | "restricted" | "left" | "kicked"
  }): Promise<void>
  close(): Promise<void>
}
