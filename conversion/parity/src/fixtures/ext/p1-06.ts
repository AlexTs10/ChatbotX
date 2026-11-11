// Fixture extension of unit P1.6 (public API namespace `contacts`). Setup
// that has no public HTTP path only; every assertion stays black-box.
// Each stack implements `create()` in `stacks/<stack>/ext/p1-06.ts`.
// Load with `await loadFixtureExt<P106Fixtures>("p1-06")`.
//
// Times are ISO-8601 strings (UTC instants). Ids are decimal strings.
// Rows are the ones the app writes itself (see the stack file for the source
// of each row). Defaults are the app's column defaults.

export type P106InboxInput = {
  workspaceId: string
  /** `Inbox.channel` (plain text, e.g. "webchat", "messenger", "telegram"). */
  channel: string
  name: string
  /** Channel-side id (page id, bot id). Default: unique. */
  sourceId?: string
}

export type P106ContactInboxInput = {
  inboxId: string
  /** `ContactInbox.channel`; must match the inbox channel. */
  channel: string
  /** `ContactInbox.source`. Default "direct". */
  source?: string
  /** `ContactInbox.sourceId` (channel user id). Default: unique. */
  sourceId?: string
  language?: string | null
  contactLastReadAt?: string | null
  lastIncomingMessageAt?: string | null
  lastOutboundMessageAt?: string | null
  consecutiveFailedReply?: number
  lastUserInput?: string | null
}

export type P106ConversationInput = {
  archivedAt?: string | null
  followed?: boolean
  botEnabled?: boolean
  lastActivityAt?: string | null
  agentLastReadAt?: string | null
  assignedUserId?: string | null
}

export type P106ContactInput = {
  workspaceId: string
  firstName?: string | null
  lastName?: string | null
  email?: string | null
  phoneNumber?: string | null
  /** pg enum `gender`: "male" | "female" | "unknown"; default null. */
  gender?: string | null
  country?: string | null
  locale?: string | null
  timezone?: string | null
  avatar?: string | null
  /** `Contact.createdAt`; default now(). `updatedAt` gets the same value. */
  createdAt?: string
  blockedAt?: string | null
  broadcastSubscribedAt?: string | null
  subscribedAt?: string | null
  /** Default false (column default). */
  emailVerified?: boolean
  /** Default true (column default). */
  emailOptIn?: boolean
  /** One ContactInbox row per entry (in order). Default: none. */
  inboxes?: P106ContactInboxInput[]
  /** One Conversation row; `null` = no conversation. Default: one with column defaults. */
  conversation?: P106ConversationInput | null
}

export type P106CreatedContact = {
  contactId: string
  /** Same order as `inboxes`. */
  contactInboxIds: string[]
  sourceIds: string[]
  conversationId: string | null
}

export type P106AdReferralInput = {
  workspaceId: string
  contactId: string
  contactInboxId: string
  channel: string
  /** "meta" | "google" | null */
  provider: string | null
  occurredAt: string
  adId: string | null
  campaignId?: string | null
  adGroupId?: string | null
  source?: string | null
  sourceUrl?: string | null
  /** `metadata.adTitle`. */
  title?: string | null
}

export type P106SequenceDispatchInput = {
  workspaceId: string
  sequenceId: string
  stepId: string
  contactId: string
  contactInboxId: string
  deliveredAt?: string | null
  seenAt?: string | null
  clickedAt?: string | null
  failedAt?: string | null
}

export interface P106Fixtures {
  /** An inbox row with no integration behind it (no worker job can reach a third party). */
  createInbox(input: P106InboxInput): Promise<{ inboxId: string }>
  /** Contact + its ContactInbox rows + (default) one Conversation. */
  createContact(input: P106ContactInput): Promise<P106CreatedContact>
  /** A `ContactToTag` link (the tag comes from `tags.create`). */
  linkTag(input: { contactId: string; tagId: string }): Promise<void>
  /** A `ContactCustomField` row with the value already in canonical storage form. */
  setCustomFieldValue(input: { contactId: string; customFieldId: string; value: string }): Promise<void>
  /** A `ContactReferralEvent` row of type "ad" (an ad click the webhooks recorded). */
  createAdReferral(input: P106AdReferralInput): Promise<{ id: string }>
  /**
   * A finished sequence step send: a completed `ContactOnSequence` enrollment
   * plus a completed `SequenceDispatch` with the given outcome times.
   */
  insertSequenceDispatch(input: P106SequenceDispatchInput): Promise<void>
  /**
   * Sets `Workspace.targetCountry` (phone normalization of contacts.create).
   * Call it before the first HTTP call of the workspace (the app caches it).
   */ (phone normalization of contacts.create / import). */
  setWorkspaceTargetCountry(input: { workspaceId: string; targetCountry: string | null }): Promise<void>
  /** Read-only setup lookup: the contact ids of a workspace whose email is in `emails`. */
  contactIdsByEmail(input: { workspaceId: string; emails: string[] }): Promise<Record<string, string>>
  close(): Promise<void>
}
