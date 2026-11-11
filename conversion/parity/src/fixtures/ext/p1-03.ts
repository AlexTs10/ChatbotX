// Fixture extension of unit P1.3 (public API: aiHandover, aiMcpServers,
// analytics). Setup with no public HTTP path only; assertions stay black-box.
// Each stack implements `create()` in `stacks/<stack>/ext/p1-03.ts`.
// Load with `await loadFixtureExt<P103Fixtures>("p1-03")`.
//
// All times are ISO-8601 strings (UTC instants). Ids are decimal strings.
// Rows that the app writes from worker listeners (analytics events, link
// clicks, flow node stats, ...) carry no foreign key to the contact tables
// unless noted, so tests may use synthetic contact/conversation ids there.

export type InboxStatus = "connected" | "disconnected"

export type CreateInboxInput = {
  workspaceId: string
  name: string
  /** `Inbox.channel` (plain text; e.g. "messenger", "telegram", "smtp"). */
  channel: string
  /** Default "connected". */
  status?: InboxStatus
  /** Channel-side id (page id, bot id). Default: unique. */
  sourceId?: string
}

export type CreateContactInput = {
  workspaceId: string
  inboxId: string
  /** `ContactInbox.channel`; must match the inbox channel. */
  channel: string
  firstName?: string | null
  lastName?: string | null
  /** `ContactInbox.sourceId` (channel user id). Default: unique. */
  sourceId?: string
  /** `ContactInbox.source`. Default "direct". */
  source?: string
  /** `ContactInbox.lastIncomingMessageAt`. Default null. */
  lastIncomingMessageAt?: string | null
  /** `Conversation.contactLastReadAt`. Default null. */
  contactLastReadAt?: string | null
}

export type CreatedContact = {
  contactId: string
  contactInboxId: string
  conversationId: string
  sourceId: string
}

export type ContactEventRow = {
  workspaceId: string
  contactId: string
  eventType: "contact_created" | "contact_deleted" | "contact_blocked"
  occurredAt: string
  source?: string | null
  sourceId?: string | null
  channel?: string | null
  inboxId?: string | null
  language?: string | null
  country?: string | null
}

export type MessageEventRow = {
  workspaceId: string
  contactId: string
  eventType: "message_human_sent" | "message_bot_sent"
  occurredAt: string
  senderType?: "bot" | "human" | null
  adminId?: string | null
  channel?: string | null
  inboxId?: string | null
}

export type ConversationEventRow = {
  workspaceId: string
  conversationId: string
  eventType:
    | "conversation_created"
    | "conversation_assigned"
    | "conversation_unassigned"
    | "conversation_transferred_to_human"
    | "conversation_transferred_to_bot"
    | "conversation_followed"
    | "conversation_unfollowed"
    | "conversation_archived"
    | "conversation_unarchived"
  occurredAt: string
  fromAssignee?: string | null
  toAssignee?: string | null
  channel?: string | null
  inboxId?: string | null
}

export type BotMessageEventRow = {
  workspaceId: string
  messageId: string
  conversationId: string
  occurredAt: string
  hasResponse: boolean
  responseType?: "automated_response" | "ai_agent" | "flow" | "none" | null
  routeType?: "flow" | "agent" | "fallback" | null
  result?: "success" | "fallback" | null
  aiProvider?: string | null
  channel?: string | null
  inboxId?: string | null
}

export type AgentResponseEventRow = {
  workspaceId: string
  conversationId: string
  contactId: string
  adminId: string
  /** Reply time. */
  occurredAt: string
  awaitingSince: string
  measuredFrom: string
  responseSeconds: number
  workingResponseSeconds?: number | null
  isFirstResponse: boolean
  slaTargetSeconds?: number | null
  slaMet?: boolean | null
  channel?: string | null
  inboxId?: string | null
}

export type ContactActiveHourlyRow = {
  workspaceId: string
  contactId: string
  contactInboxId: string
  inboxId: string
  hourBucket: string
}

export type RecipientTimestamps = {
  deliveredAt?: string | null
  seenAt?: string | null
  clickedAt?: string | null
  failedAt?: string | null
  errorContent?: string | null
}

export type FlowNodeStatRow = {
  workspaceId: string
  flowId: string
  analyticsId: string
  nodeId: string
  buttonId?: string | null
  contactId: string
  contactInboxId: string
  eventType: "message:delivered" | "message:failed" | "flow:clicked"
  occurredAt: string
  errorContent?: string | null
}

export type SmartDelayRow = {
  workspaceId: string
  flowId: string
  flowVersionId?: string | null
  contactInboxId: string
  conversationId: string
  nodeId?: string | null
  stepId?: string | null
  type: "waitNode" | "followUp" | "quickReplyFollowUp" | "stepInputTimeout"
  status: "pending" | "scheduled" | "completed" | "failed" | "canceled"
  /** Keep it in the future for pending/scheduled rows (the scanner claims due rows). */
  triggerAt: string
}

export type CommentAutomationCounters = Partial<{
  sentCount: number
  deliveredCount: number
  seenCount: number
  clickedCount: number
  failedCount: number
  missedCount: number
}>

export type CommentAutomationEventRow = {
  workspaceId: string
  automationId: string
  contactId?: string | null
  contactInboxId?: string | null
  postId: string
  commentId: string
  commentText?: string | null
  replyChannel: "public" | "private"
  replyType: "AIAgent" | "text" | "flow" | "none"
  replyText?: string | null
  status: "sent" | "failed"
  occurredAt: string
  deliveredAt?: string | null
  seenAt?: string | null
  clickedAt?: string | null
  failedAt?: string | null
  errorDetail?: string | null
  httpCode?: string | null
}

export type LinkStatRow = {
  workspaceId: string
  linkId: string
  contactId: string
  contactInboxId: string
  occurredAt: string
}

export type P103Fixtures = {
  /** `Inbox` row only (no channel integration row: nothing can reach Meta). */
  createInbox(input: CreateInboxInput): Promise<{ inboxId: string }>
  /**
   * `Contact` + `ContactInbox` + DM `Conversation` (sourceId NULL), the rows
   * `createContactWithInbox` writes. The `ContactInbox` insert trigger keeps
   * `InboxContactStat.totalContacts`.
   */
  createContact(input: CreateContactInput): Promise<CreatedContact>
  /** `WorkspaceMember` with role "agent" for an existing user. */
  addMember(input: { workspaceId: string; userId: string }): Promise<{ memberId: string }>
  /** `ResponseSlaPolicy` row of the workspace. */
  setResponseSlaPolicy(input: {
    workspaceId: string
    enabled: boolean
    firstResponseTargetSeconds: number
    nextResponseTargetSeconds?: number | null
  }): Promise<void>
  insertContactEvents(rows: ContactEventRow[]): Promise<void>
  insertMessageEvents(rows: MessageEventRow[]): Promise<void>
  insertConversationEvents(rows: ConversationEventRow[]): Promise<void>
  insertBotMessageEvents(rows: BotMessageEventRow[]): Promise<void>
  insertAgentResponseEvents(rows: AgentResponseEventRow[]): Promise<void>
  insertContactActiveHourly(rows: ContactActiveHourlyRow[]): Promise<void>
  /** `WorkspaceMac` billing-period row. */
  insertWorkspaceMac(input: {
    workspaceId: string
    ownerId: string
    periodStart: string
    periodEnd: string
    macCount: number
  }): Promise<void>
  /** A sent `Broadcast` (status "sent", schedulesType "now"). */
  createBroadcast(input: { workspaceId: string; name: string; channel: string }): Promise<{ broadcastId: string }>
  /** `ContactOnBroadcast` recipient rows (sent = true). */
  insertBroadcastRecipients(
    rows: Array<
      {
        broadcastId: string
        contactId: string
        contactInboxId: string
        conversationId: string
      } & RecipientTimestamps
    >,
  ): Promise<void>
  /** `SequenceDispatch` rows (status "completed"; never picked by the dispatcher). */
  insertSequenceDispatches(
    rows: Array<
      {
        workspaceId: string
        sequenceId: string
        stepId: string
        contactId: string
        contactInboxId: string
      } & RecipientTimestamps
    >,
  ): Promise<void>
  /** Id of the flow's live `FlowAnalyticsSession` (deletedAt NULL), or null. */
  findFlowAnalyticsSessionId(input: { workspaceId: string; flowId: string }): Promise<string | null>
  insertFlowNodeStats(rows: FlowNodeStatRow[]): Promise<void>
  insertSmartDelays(rows: SmartDelayRow[]): Promise<void>
  /** `CommentAutomation` row with its lifetime counters. */
  createCommentAutomation(input: {
    workspaceId: string
    inboxId: string
    name: string
    type?: "messenger" | "instagram"
    counters?: CommentAutomationCounters
  }): Promise<{ automationId: string }>
  /** Returns the event ids in input order. */
  insertCommentAutomationEvents(rows: CommentAutomationEventRow[]): Promise<{ ids: string[] }>
  /** `MagicLinkStat` ("magic") or `RefLinkStat` ("ref") click rows. */
  insertLinkStats(kind: "magic" | "ref", rows: LinkStatRow[]): Promise<void>
  close(): Promise<void>
}
