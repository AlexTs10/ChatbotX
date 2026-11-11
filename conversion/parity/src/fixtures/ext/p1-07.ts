// Fixture extension of unit P1.7 (public API namespaces conversations,
// coupons, customFields, decisionConnections, decisionProfiles,
// discountCodes). Setup that has no public HTTP path only; every assertion
// stays black-box. Each stack implements `create()` in
// `stacks/<stack>/ext/p1-07.ts`. Load with
// `await loadFixtureExt<P107Fixtures>("p1-07")`.
//
// Times are ISO-8601 strings (UTC instants). Ids are decimal strings. Rows
// are the ones the app writes itself (see the stack file for the source of
// each row). Defaults are the app's column defaults.
//
// Write a row only before the app has read it (the app caches conversations
// and contact inboxes in Redis; a direct write cannot clear that cache).

/**
 * The Meta app id the stacks store as the channel's OAuth client id. On
 * Messenger it is "our own app id" (thread owner = us).
 */
export const P107_META_APP_ID = "1700000000000007"

export type P107InboxInput = {
  workspaceId: string
  /** `Inbox.channel` (plain text, e.g. "telegram", "messenger"). */
  channel: string
  name: string
  /** Channel-side id. Default: unique. */
  sourceId?: string
}

export type P107WhatsappChannelInput = {
  workspaceId: string
  name: string
  /** Meta phone number id; Graph calls go to `/v23.0/<phoneNumberId>/...`. */
  phoneNumberId: string
  wabaId?: string
  displayPhoneNumber?: string
}

export type P107MessengerChannelInput = {
  workspaceId: string
  name: string
  /** Facebook page id (`Inbox.sourceId`). */
  pageId: string
}

export type P107ChannelCreated = {
  inboxId: string
  integrationId: string
  /** Page / phone access token stored in the encrypted auth (sent as `Bearer`). */
  accessToken: string
}

export type P107ContactInboxInput = {
  inboxId: string
  /** `ContactInbox.channel`; must match the inbox channel. */
  channel: string
  /** `ContactInbox.source`. Default "direct". */
  source?: string
  /** `ContactInbox.sourceId` (channel user id: PSID, WhatsApp phone digits). Default: unique. */
  sourceId?: string
  sourceUserId?: string | null
  sourceUsername?: string | null
  language?: string | null
  contactLastReadAt?: string | null
  lastMessageAt?: string | null
  lastIncomingMessageAt?: string | null
  /** pg enum: "owned" | "standby" | "idle"; default null (routing never observed). */
  threadControlState?: string | null
  threadOwnerRole?: string | null
  threadOwnerAppId?: string | null
  threadControlUpdatedAt?: string | null
  threadOwnerExpiresAt?: string | null
  /** `ContactInbox.referral` jsonb (the raw webhook referral payload). */
  referral?: unknown
}

export type P107ConversationInput = {
  /** `Conversation.createdAt`; default now(). `updatedAt` gets the same value. */
  createdAt?: string
  archivedAt?: string | null
  followed?: boolean
  botEnabled?: boolean
  lastActivityAt?: string | null
  agentLastReadAt?: string | null
  contactLastReadAt?: string | null
  assignedUserId?: string | null
  assignedInboxTeamId?: string | null
  adminRepliedAt?: string | null
  contactRepliedAt?: string | null
  responseDueAt?: string | null
  responseSlaBreachingAt?: string | null
}

export type P107ContactInput = {
  workspaceId: string
  firstName?: string | null
  lastName?: string | null
  email?: string | null
  phoneNumber?: string | null
  /** pg enum `gender`: "male" | "female" | "unknown"; default null. */
  gender?: string | null
  avatar?: string | null
  blockedAt?: string | null
  /** `Contact.createdAt`; default now(). */
  createdAt?: string
  /** One ContactInbox row per entry (in order). Default: none. */
  inboxes?: P107ContactInboxInput[]
  /** One Conversation row; `null` = no conversation. Default: one with column defaults. */
  conversation?: P107ConversationInput | null
}

export type P107CreatedContact = {
  contactId: string
  /** Same order as `inboxes`. */
  contactInboxIds: string[]
  sourceIds: string[]
  conversationId: string | null
}

export interface P107Fixtures {
  /** An inbox row with no integration behind it (no worker job can reach a third party). */
  createInbox(input: P107InboxInput): Promise<{ inboxId: string }>
  /** Inbox + IntegrationWhatsapp with encrypted OAuth auth (Graph host graph.facebook.com). */
  createWhatsappChannel(input: P107WhatsappChannelInput): Promise<P107ChannelCreated>
  /** Inbox + IntegrationMessenger with encrypted OAuth auth. */
  createMessengerChannel(input: P107MessengerChannelInput): Promise<P107ChannelCreated>
  /** Contact + its ContactInbox rows + (default) one Conversation. */
  createContact(input: P107ContactInput): Promise<P107CreatedContact>
  /** An `InboxTeam` row (the enterprise inbox-team create). */
  createInboxTeam(input: { workspaceId: string; name: string }): Promise<{ inboxTeamId: string }>
  /** A `WorkspaceMember` row (role agent) for an existing user (invitation accept). */
  addMember(input: { workspaceId: string; userId: string }): Promise<{ memberId: string }>
  close(): Promise<void>
}
