// Postgres implementation of the P1.3 fixture extension (original stack).
// Writes the same logical rows the app writes; see the comment per method.
//
// Row sources (baseline 40e733cee):
// - Inbox: inboxService.create (packages/database/src/schema/inbox.ts); no
//   channel integration row, so no worker job can reach a third party.
// - Contact + ContactInbox + Conversation: createContactWithInbox
//   (packages/business/src/contact/create-with-inbox.ts). The ContactInbox
//   insert trigger `contact_inbox_insert_stats_trigger` keeps
//   InboxContactStat (packages/database/drizzle/20260624082115_*).
// - WorkspaceMember (agent): workspace invitation accept
//   (packages/database/src/schema/workspace-member.ts).
// - Analytics*Event rows: the `insertEvents` of the analytics repositories
//   (packages/analytics/src/repositories/postgres/*-stats.repository.ts),
//   eventId = createId().
// - ContactActiveHourly / WorkspaceMac: MacTrackingService
//   (packages/analytics/src/services/mac-tracking.service.ts).
// - Broadcast + ContactOnBroadcast: broadcast send pipeline
//   (packages/database/src/schema/broadcast.ts, contact-on-broadcast.ts).
// - SequenceDispatch: sequence scheduler (schema/sequence-dispatch.ts).
// - FlowNodeStat: FlowStatsRepository.insertNodeStats; FlowAnalyticsSession
//   is written by flowService.create* (read here only).
// - ContactOnSmartDelay: smartDelayService (schema/contact-on-smart-delay.ts).
// - CommentAutomation / CommentAutomationEvent: commentAutomationService and
//   the comment reply recorder (schema/comment-automation*.ts).
// - MagicLinkStat / RefLinkStat: LinkStatsRepository.insertStats.

import { randomBytes } from "node:crypto"
import pg from "pg"
import type {
  AgentResponseEventRow,
  BotMessageEventRow,
  CommentAutomationEventRow,
  ContactActiveHourlyRow,
  ContactEventRow,
  ConversationEventRow,
  CreateContactInput,
  CreateInboxInput,
  CreatedContact,
  FlowNodeStatRow,
  LinkStatRow,
  MessageEventRow,
  P103Fixtures,
  RecipientTimestamps,
  SmartDelayRow,
} from "../../../src/fixtures/ext/p1-03.ts"

const SNOWFLAKE_EPOCH_MS = Date.parse("2004-02-01T00:00:00.000Z")
/** App processes use place ids 1-3, core fixtures 15, P1.2 14; P1.3 uses 12. */
const PLACE_ID = 12n

let lastMs = 0n
let sequence = 0n
const createId = (): string => {
  let now = BigInt(Date.now() - SNOWFLAKE_EPOCH_MS)
  if (now <= lastMs) {
    sequence += 1n
    if (sequence >= 1024n) {
      lastMs += 1n
      sequence = 0n
    }
    now = lastMs
  } else {
    lastMs = now
    sequence = BigInt(randomBytes(1)[0] ?? 0)
  }
  return ((now << 14n) | (PLACE_ID << 10n) | sequence).toString()
}

const uniqueText = (prefix: string) => `${prefix}-${Date.now().toString(36)}${randomBytes(4).toString("hex")}`

const AGENT_PERMISSIONS = {
  superAdmin: false,
  analytics: true,
  flows: true,
  contacts: true,
  onlyAssignedContacts: false,
  emailAndPhone: true,
  broadcast: true,
  ecommerce: true,
}
const DEFAULT_MEMBER_NOTIFICATION_TYPES = { notifyAdmin: true, newMessageToHuman: true, newOrder: true }
const DEFAULT_MEMBER_NOTIFICATION_CHANNELS = { messenger: true, email: true, telegram: true, browser: true }

class PostgresP103Fixtures implements P103Fixtures {
  private readonly pool: pg.Pool

  constructor(connectionString: string) {
    this.pool = new pg.Pool({ connectionString, max: 2, idleTimeoutMillis: 1_000, allowExitOnIdle: true })
  }

  private async transaction<T>(fn: (client: pg.PoolClient) => Promise<T>): Promise<T> {
    const client = await this.pool.connect()
    try {
      await client.query("BEGIN")
      const result = await fn(client)
      await client.query("COMMIT")
      return result
    } catch (error) {
      await client.query("ROLLBACK")
      throw error
    } finally {
      client.release()
    }
  }

  /** Multi-row insert; `columns` are quoted identifiers, rows are value arrays. */
  private async insertMany(table: string, columns: string[], rows: unknown[][]): Promise<void> {
    if (rows.length === 0) {
      return
    }
    const params: unknown[] = []
    const tuples = rows.map((row) => {
      const placeholders = row.map((value) => {
        params.push(value)
        return `$${params.length}`
      })
      return `(${placeholders.join(", ")})`
    })
    await this.pool.query(
      `INSERT INTO "${table}" (${columns.map((c) => `"${c}"`).join(", ")}) VALUES ${tuples.join(", ")}`,
      params,
    )
  }

  async createInbox(input: CreateInboxInput): Promise<{ inboxId: string }> {
    const inboxId = createId()
    const status = input.status ?? "connected"
    await this.pool.query(
      `INSERT INTO "Inbox" ("id", "name", "channel", "sourceId", "workspaceId", "status", "disconnectedAt")
       VALUES ($1, $2, $3, $4, $5, $6, $7)`,
      [
        inboxId,
        input.name,
        input.channel,
        input.sourceId ?? uniqueText("src"),
        input.workspaceId,
        status,
        status === "disconnected" ? new Date().toISOString() : null,
      ],
    )
    return { inboxId }
  }

  async createContact(input: CreateContactInput): Promise<CreatedContact> {
    const contactId = createId()
    const contactInboxId = createId()
    const conversationId = createId()
    const sourceId = input.sourceId ?? uniqueText("psid")
    await this.transaction(async (client) => {
      await client.query(
        `INSERT INTO "Contact" ("id", "workspaceId", "firstName", "lastName") VALUES ($1, $2, $3, $4)`,
        [contactId, input.workspaceId, input.firstName ?? null, input.lastName ?? null],
      )
      await client.query(
        `INSERT INTO "ContactInbox"
           ("id", "originalContactId", "contactId", "inboxId", "channel", "source", "sourceId", "lastIncomingMessageAt")
         VALUES ($1, $2, $2, $3, $4, $5, $6, $7)`,
        [
          contactInboxId,
          contactId,
          input.inboxId,
          input.channel,
          input.source ?? "direct",
          sourceId,
          input.lastIncomingMessageAt ?? null,
        ],
      )
      await client.query(
        `INSERT INTO "Conversation" ("id", "workspaceId", "contactId", "contactLastReadAt") VALUES ($1, $2, $3, $4)`,
        [conversationId, input.workspaceId, contactId, input.contactLastReadAt ?? null],
      )
    })
    return { contactId, contactInboxId, conversationId, sourceId }
  }

  async addMember(input: { workspaceId: string; userId: string }): Promise<{ memberId: string }> {
    const memberId = createId()
    await this.pool.query(
      `INSERT INTO "WorkspaceMember"
         ("id", "workspaceId", "userId", "role", "permissions", "notificationTypes", "notificationChannels")
       VALUES ($1, $2, $3, 'agent', $4, $5, $6)`,
      [
        memberId,
        input.workspaceId,
        input.userId,
        JSON.stringify(AGENT_PERMISSIONS),
        JSON.stringify(DEFAULT_MEMBER_NOTIFICATION_TYPES),
        JSON.stringify(DEFAULT_MEMBER_NOTIFICATION_CHANNELS),
      ],
    )
    return { memberId }
  }

  async setResponseSlaPolicy(input: {
    workspaceId: string
    enabled: boolean
    firstResponseTargetSeconds: number
    nextResponseTargetSeconds?: number | null
  }): Promise<void> {
    await this.pool.query(
      `INSERT INTO "ResponseSlaPolicy"
         ("id", "workspaceId", "enabled", "firstResponseTargetSeconds", "nextResponseTargetSeconds")
       VALUES ($1, $2, $3, $4, $5)`,
      [createId(), input.workspaceId, input.enabled, input.firstResponseTargetSeconds, input.nextResponseTargetSeconds ?? null],
    )
  }

  async insertContactEvents(rows: ContactEventRow[]): Promise<void> {
    await this.insertMany(
      "AnalyticsContactEvent",
      ["eventId", "workspaceId", "contactId", "eventType", "occurredAt", "source", "sourceId", "channel", "inboxId", "language", "country"],
      rows.map((r) => [
        createId(),
        r.workspaceId,
        r.contactId,
        r.eventType,
        r.occurredAt,
        r.source ?? null,
        r.sourceId ?? null,
        r.channel ?? null,
        r.inboxId ?? null,
        r.language ?? null,
        r.country ?? null,
      ]),
    )
  }

  async insertMessageEvents(rows: MessageEventRow[]): Promise<void> {
    await this.insertMany(
      "AnalyticsMessageEvent",
      ["eventId", "workspaceId", "contactId", "eventType", "occurredAt", "senderType", "adminId", "channel", "inboxId"],
      rows.map((r) => [
        createId(),
        r.workspaceId,
        r.contactId,
        r.eventType,
        r.occurredAt,
        r.senderType ?? null,
        r.adminId ?? null,
        r.channel ?? null,
        r.inboxId ?? null,
      ]),
    )
  }

  async insertConversationEvents(rows: ConversationEventRow[]): Promise<void> {
    await this.insertMany(
      "AnalyticsConversationEvent",
      ["eventId", "workspaceId", "conversationId", "eventType", "occurredAt", "fromAssignee", "toAssignee", "channel", "inboxId"],
      rows.map((r) => [
        createId(),
        r.workspaceId,
        r.conversationId,
        r.eventType,
        r.occurredAt,
        r.fromAssignee ?? null,
        r.toAssignee ?? null,
        r.channel ?? null,
        r.inboxId ?? null,
      ]),
    )
  }

  async insertBotMessageEvents(rows: BotMessageEventRow[]): Promise<void> {
    await this.insertMany(
      "AnalyticsBotMessageEvent",
      [
        "eventId",
        "workspaceId",
        "messageId",
        "conversationId",
        "occurredAt",
        "hasResponse",
        "responseType",
        "routeType",
        "result",
        "aiProvider",
        "channel",
        "inboxId",
      ],
      rows.map((r) => [
        createId(),
        r.workspaceId,
        r.messageId,
        r.conversationId,
        r.occurredAt,
        r.hasResponse,
        r.responseType ?? null,
        r.routeType ?? null,
        r.result ?? null,
        r.aiProvider ?? null,
        r.channel ?? null,
        r.inboxId ?? null,
      ]),
    )
  }

  async insertAgentResponseEvents(rows: AgentResponseEventRow[]): Promise<void> {
    await this.insertMany(
      "AnalyticsAgentResponseEvent",
      [
        "eventId",
        "workspaceId",
        "conversationId",
        "contactId",
        "adminId",
        "occurredAt",
        "awaitingSince",
        "measuredFrom",
        "responseSeconds",
        "workingResponseSeconds",
        "isFirstResponse",
        "slaTargetSeconds",
        "slaMet",
        "channel",
        "inboxId",
      ],
      rows.map((r) => [
        createId(),
        r.workspaceId,
        r.conversationId,
        r.contactId,
        r.adminId,
        r.occurredAt,
        r.awaitingSince,
        r.measuredFrom,
        r.responseSeconds,
        r.workingResponseSeconds ?? null,
        r.isFirstResponse,
        r.slaTargetSeconds ?? null,
        r.slaMet ?? null,
        r.channel ?? null,
        r.inboxId ?? null,
      ]),
    )
  }

  async insertContactActiveHourly(rows: ContactActiveHourlyRow[]): Promise<void> {
    await this.insertMany(
      "ContactActiveHourly",
      ["workspaceId", "contactId", "contactInboxId", "hourBucket", "inboxId"],
      rows.map((r) => [r.workspaceId, r.contactId, r.contactInboxId, r.hourBucket, r.inboxId]),
    )
  }

  async insertWorkspaceMac(input: {
    workspaceId: string
    ownerId: string
    periodStart: string
    periodEnd: string
    macCount: number
  }): Promise<void> {
    const tenant = await this.pool.query<{ tenantId: string }>(
      `SELECT "tenantId"::text AS "tenantId" FROM "Workspace" WHERE "id" = $1`,
      [input.workspaceId],
    )
    await this.pool.query(
      `INSERT INTO "WorkspaceMac" ("id", "workspaceId", "ownerId", "tenantId", "periodStart", "periodEnd", "macCount")
       VALUES ($1, $2, $3, $4, $5, $6, $7)`,
      [
        createId(),
        input.workspaceId,
        input.ownerId,
        tenant.rows[0]?.tenantId ?? null,
        input.periodStart,
        input.periodEnd,
        input.macCount,
      ],
    )
  }

  async createBroadcast(input: { workspaceId: string; name: string; channel: string }): Promise<{ broadcastId: string }> {
    const broadcastId = createId()
    await this.pool.query(
      `INSERT INTO "Broadcast"
         ("id", "name", "workspaceId", "status", "schedulesType", "schedulesAt", "subaction", "channel", "handoffCompletedAt")
       VALUES ($1, $2, $3, 'sent', 'now', now(), 'allContacts', $4, now())`,
      [broadcastId, input.name, input.workspaceId, input.channel],
    )
    return { broadcastId }
  }

  async insertBroadcastRecipients(
    rows: Array<{ broadcastId: string; contactId: string; contactInboxId: string; conversationId: string } & RecipientTimestamps>,
  ): Promise<void> {
    await this.insertMany(
      "ContactOnBroadcast",
      [
        "broadcastId",
        "contactId",
        "contactInboxId",
        "conversationId",
        "sent",
        "deliveredAt",
        "seenAt",
        "clickedAt",
        "failedAt",
        "errorContent",
      ],
      rows.map((r) => [
        r.broadcastId,
        r.contactId,
        r.contactInboxId,
        r.conversationId,
        true,
        r.deliveredAt ?? null,
        r.seenAt ?? null,
        r.clickedAt ?? null,
        r.failedAt ?? null,
        r.errorContent ?? null,
      ]),
    )
  }

  async insertSequenceDispatches(
    rows: Array<
      { workspaceId: string; sequenceId: string; stepId: string; contactId: string; contactInboxId: string } & RecipientTimestamps
    >,
  ): Promise<void> {
    // One finished enrollment (`ContactOnSequence`, the FK target of
    // SequenceDispatch.enrollmentId) per contact and sequence.
    const enrollments = new Map<string, string>()
    for (const r of rows) {
      const key = `${r.workspaceId}|${r.sequenceId}|${r.contactId}`
      if (enrollments.has(key)) {
        continue
      }
      const enrollmentId = createId()
      enrollments.set(key, enrollmentId)
      await this.pool.query(
        `INSERT INTO "ContactOnSequence"
           ("id", "workspaceId", "sequenceId", "contactId", "status", "completedAt", "currentStep")
         VALUES ($1, $2, $3, $4, 'completed', now(), 1)`,
        [enrollmentId, r.workspaceId, r.sequenceId, r.contactId],
      )
    }
    await this.insertMany(
      "SequenceDispatch",
      [
        "id",
        "runAtMs",
        "status",
        "idempotencyKey",
        "completedAt",
        "deliveredAt",
        "seenAt",
        "clickedAt",
        "failedAt",
        "errorContent",
        "workspaceId",
        "sequenceId",
        "contactId",
        "contactInboxId",
        "stepId",
        "enrollmentId",
      ],
      rows.map((r) => {
        const id = createId()
        return [
          id,
          String(Date.now()),
          "completed",
          `parity:${id}`,
          new Date().toISOString(),
          r.deliveredAt ?? null,
          r.seenAt ?? null,
          r.clickedAt ?? null,
          r.failedAt ?? null,
          r.errorContent ?? null,
          r.workspaceId,
          r.sequenceId,
          r.contactId,
          r.contactInboxId,
          r.stepId,
          enrollments.get(`${r.workspaceId}|${r.sequenceId}|${r.contactId}`),
        ]
      }),
    )
  }

  async findFlowAnalyticsSessionId(input: { workspaceId: string; flowId: string }): Promise<string | null> {
    const result = await this.pool.query<{ id: string }>(
      `SELECT "id"::text AS id FROM "FlowAnalyticsSession"
       WHERE "workspaceId" = $1 AND "flowId" = $2 AND "deletedAt" IS NULL
       LIMIT 1`,
      [input.workspaceId, input.flowId],
    )
    return result.rows[0]?.id ?? null
  }

  async insertFlowNodeStats(rows: FlowNodeStatRow[]): Promise<void> {
    await this.insertMany(
      "FlowNodeStat",
      [
        "id",
        "workspaceId",
        "flowId",
        "analyticsId",
        "nodeId",
        "buttonId",
        "contactId",
        "contactInboxId",
        "eventType",
        "errorContent",
        "occurredAt",
      ],
      rows.map((r) => [
        createId(),
        r.workspaceId,
        r.flowId,
        r.analyticsId,
        r.nodeId,
        r.buttonId ?? null,
        r.contactId,
        r.contactInboxId,
        r.eventType,
        r.errorContent ?? null,
        r.occurredAt,
      ]),
    )
  }

  async insertSmartDelays(rows: SmartDelayRow[]): Promise<void> {
    await this.insertMany(
      "ContactOnSmartDelay",
      [
        "id",
        "workspaceId",
        "flowId",
        "flowVersionId",
        "contactInboxId",
        "conversationId",
        "nodeId",
        "stepId",
        "type",
        "triggerAt",
        "status",
      ],
      rows.map((r) => [
        createId(),
        r.workspaceId,
        r.flowId,
        r.flowVersionId ?? null,
        r.contactInboxId,
        r.conversationId,
        r.nodeId ?? null,
        r.stepId ?? null,
        r.type,
        r.triggerAt,
        r.status,
      ]),
    )
  }

  async createCommentAutomation(input: {
    workspaceId: string
    inboxId: string
    name: string
    type?: "messenger" | "instagram"
    counters?: Partial<Record<string, number>>
  }): Promise<{ automationId: string }> {
    const automationId = createId()
    const c = input.counters ?? {}
    await this.pool.query(
      `INSERT INTO "CommentAutomation"
         ("id", "name", "workspaceId", "type", "inboxId", "isActive",
          "sentCount", "deliveredCount", "seenCount", "clickedCount", "failedCount", "missedCount")
       VALUES ($1, $2, $3, $4, $5, false, $6, $7, $8, $9, $10, $11)`,
      [
        automationId,
        input.name,
        input.workspaceId,
        input.type ?? "messenger",
        input.inboxId,
        c.sentCount ?? 0,
        c.deliveredCount ?? 0,
        c.seenCount ?? 0,
        c.clickedCount ?? 0,
        c.failedCount ?? 0,
        c.missedCount ?? 0,
      ],
    )
    return { automationId }
  }

  async insertCommentAutomationEvents(rows: CommentAutomationEventRow[]): Promise<{ ids: string[] }> {
    const ids = rows.map(() => createId())
    await this.insertMany(
      "CommentAutomationEvent",
      [
        "id",
        "workspaceId",
        "automationId",
        "contactId",
        "contactInboxId",
        "postId",
        "commentId",
        "commentText",
        "replyChannel",
        "replyType",
        "replyText",
        "status",
        "deliveredAt",
        "seenAt",
        "clickedAt",
        "failedAt",
        "errorDetail",
        "httpCode",
        "occurredAt",
      ],
      rows.map((r, index) => [
        ids[index],
        r.workspaceId,
        r.automationId,
        r.contactId ?? null,
        r.contactInboxId ?? null,
        r.postId,
        r.commentId,
        r.commentText ?? null,
        r.replyChannel,
        r.replyType,
        r.replyText ?? null,
        r.status,
        r.deliveredAt ?? null,
        r.seenAt ?? null,
        r.clickedAt ?? null,
        r.failedAt ?? null,
        r.errorDetail ?? null,
        r.httpCode ?? null,
        r.occurredAt,
      ]),
    )
    return { ids }
  }

  async insertLinkStats(kind: "magic" | "ref", rows: LinkStatRow[]): Promise<void> {
    await this.insertMany(
      kind === "magic" ? "MagicLinkStat" : "RefLinkStat",
      ["workspaceId", "linkId", "contactId", "contactInboxId", "occurredAt"],
      rows.map((r) => [r.workspaceId, r.linkId, r.contactId, r.contactInboxId, r.occurredAt]),
    )
  }

  async close(): Promise<void> {
    await this.pool.end()
  }
}

export const create = (): P103Fixtures =>
  new PostgresP103Fixtures(process.env.PARITY_PG_URL ?? "postgresql://chatbotx:secretkey@postgres:5432/chatbotx")
