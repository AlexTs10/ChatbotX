// Rust-stack implementation of the P1.3 fixture extension
// (`src/fixtures/ext/p1-03.ts`): the rows of `stacks/original/ext/p1-03.ts`
// written with `node:sqlite` into the shared `/rust-data` database. Dates are
// epoch ms, booleans 0 / 1, ids integers (the Rust schema's types).

import { randomBytes } from "node:crypto"
import { DatabaseSync, type SQLInputValue } from "node:sqlite"
import type {
  AgentResponseEventRow,
  BotMessageEventRow,
  CommentAutomationCounters,
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
const PLACE_ID = 7n

let lastMs = 0n
let sequence = 0n
const createId = (): bigint => {
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
    sequence = 0n
  }
  return (now << 14n) | (PLACE_ID << 10n) | sequence
}

const uniqueText = (prefix: string) => `${prefix}-${Date.now().toString(36)}${randomBytes(4).toString("hex")}`
const id = (value: string): bigint => BigInt(value)
const optId = (value: string | null | undefined): bigint | null => (value == null ? null : BigInt(value))
const ms = (iso: string): number => Date.parse(iso)
const optMs = (iso: string | null | undefined): number | null => (iso == null ? null : Date.parse(iso))
const flag = (value: boolean | null | undefined): number | null => (value == null ? null : value ? 1 : 0)

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

class SqliteP103Fixtures implements P103Fixtures {
  private db: DatabaseSync | undefined

  constructor(private readonly path: string) {}

  private open(): DatabaseSync {
    if (!this.db) {
      const db = new DatabaseSync(this.path)
      db.exec("PRAGMA busy_timeout = 10000; PRAGMA foreign_keys = ON;")
      // The app function of the `Contact` full-name index (crates/db section 3).
      db.function("regexp_replace", { deterministic: true }, (text, pattern, replacement, flags) =>
        text == null
          ? null
          : String(text).replace(new RegExp(String(pattern), String(flags ?? "")), String(replacement)),
      )
      this.db = db
    }
    return this.db
  }

  private transaction<T>(fn: (db: DatabaseSync) => T): T {
    const db = this.open()
    db.exec("BEGIN IMMEDIATE")
    try {
      const result = fn(db)
      db.exec("COMMIT")
      return result
    } catch (error) {
      db.exec("ROLLBACK")
      throw error
    }
  }

  /** One insert per row, in one transaction. */
  private insertMany(table: string, columns: string[], rows: SQLInputValue[][]): void {
    if (rows.length === 0) {
      return
    }
    this.transaction((db) => {
      const statement = db.prepare(
        `INSERT INTO "${table}" (${columns.map((c) => `"${c}"`).join(", ")}) VALUES (${columns.map(() => "?").join(", ")})`,
      )
      for (const row of rows) {
        statement.run(...row)
      }
    })
  }

  async createInbox(input: CreateInboxInput): Promise<{ inboxId: string }> {
    const inboxId = createId()
    const status = input.status ?? "connected"
    this.open()
      .prepare(
        `INSERT INTO "Inbox" ("id", "name", "channel", "sourceId", "workspaceId", "status", "disconnectedAt")
         VALUES (?, ?, ?, ?, ?, ?, ?)`,
      )
      .run(
        inboxId,
        input.name,
        input.channel,
        input.sourceId ?? uniqueText("src"),
        id(input.workspaceId),
        status,
        status === "disconnected" ? Date.now() : null,
      )
    return { inboxId: inboxId.toString() }
  }

  async createContact(input: CreateContactInput): Promise<CreatedContact> {
    const contactId = createId()
    const contactInboxId = createId()
    const conversationId = createId()
    const sourceId = input.sourceId ?? uniqueText("psid")
    this.transaction((db) => {
      db.prepare(`INSERT INTO "Contact" ("id", "workspaceId", "firstName", "lastName") VALUES (?, ?, ?, ?)`).run(
        contactId,
        id(input.workspaceId),
        input.firstName ?? null,
        input.lastName ?? null,
      )
      db.prepare(
        `INSERT INTO "ContactInbox"
           ("id", "originalContactId", "contactId", "inboxId", "channel", "source", "sourceId", "lastIncomingMessageAt")
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)`,
      ).run(
        contactInboxId,
        contactId,
        contactId,
        id(input.inboxId),
        input.channel,
        input.source ?? "direct",
        sourceId,
        optMs(input.lastIncomingMessageAt),
      )
      db.prepare(
        `INSERT INTO "Conversation" ("id", "workspaceId", "contactId", "contactLastReadAt") VALUES (?, ?, ?, ?)`,
      ).run(conversationId, id(input.workspaceId), contactId, optMs(input.contactLastReadAt))
    })
    return {
      contactId: contactId.toString(),
      contactInboxId: contactInboxId.toString(),
      conversationId: conversationId.toString(),
      sourceId,
    }
  }

  async addMember(input: { workspaceId: string; userId: string }): Promise<{ memberId: string }> {
    const memberId = createId()
    this.open()
      .prepare(
        `INSERT INTO "WorkspaceMember"
           ("id", "workspaceId", "userId", "role", "permissions", "notificationTypes", "notificationChannels")
         VALUES (?, ?, ?, 'agent', ?, ?, ?)`,
      )
      .run(
        memberId,
        id(input.workspaceId),
        id(input.userId),
        JSON.stringify(AGENT_PERMISSIONS),
        JSON.stringify(DEFAULT_MEMBER_NOTIFICATION_TYPES),
        JSON.stringify(DEFAULT_MEMBER_NOTIFICATION_CHANNELS),
      )
    return { memberId: memberId.toString() }
  }

  async setResponseSlaPolicy(input: {
    workspaceId: string
    enabled: boolean
    firstResponseTargetSeconds: number
    nextResponseTargetSeconds?: number | null
  }): Promise<void> {
    this.open()
      .prepare(
        `INSERT INTO "ResponseSlaPolicy"
           ("id", "workspaceId", "enabled", "firstResponseTargetSeconds", "nextResponseTargetSeconds")
         VALUES (?, ?, ?, ?, ?)`,
      )
      .run(
        createId(),
        id(input.workspaceId),
        flag(input.enabled),
        input.firstResponseTargetSeconds,
        input.nextResponseTargetSeconds ?? null,
      )
  }

  async insertContactEvents(rows: ContactEventRow[]): Promise<void> {
    this.insertMany(
      "AnalyticsContactEvent",
      ["eventId", "workspaceId", "contactId", "eventType", "occurredAt", "source", "sourceId", "channel", "inboxId", "language", "country"],
      rows.map((r) => [
        createId().toString(),
        id(r.workspaceId),
        id(r.contactId),
        r.eventType,
        ms(r.occurredAt),
        r.source ?? null,
        r.sourceId ?? null,
        r.channel ?? null,
        optId(r.inboxId),
        r.language ?? null,
        r.country ?? null,
      ]),
    )
  }

  async insertMessageEvents(rows: MessageEventRow[]): Promise<void> {
    this.insertMany(
      "AnalyticsMessageEvent",
      ["eventId", "workspaceId", "contactId", "eventType", "occurredAt", "senderType", "adminId", "channel", "inboxId"],
      rows.map((r) => [
        createId().toString(),
        id(r.workspaceId),
        id(r.contactId),
        r.eventType,
        ms(r.occurredAt),
        r.senderType ?? null,
        optId(r.adminId),
        r.channel ?? null,
        optId(r.inboxId),
      ]),
    )
  }

  async insertConversationEvents(rows: ConversationEventRow[]): Promise<void> {
    this.insertMany(
      "AnalyticsConversationEvent",
      ["eventId", "workspaceId", "conversationId", "eventType", "occurredAt", "fromAssignee", "toAssignee", "channel", "inboxId"],
      rows.map((r) => [
        createId().toString(),
        id(r.workspaceId),
        id(r.conversationId),
        r.eventType,
        ms(r.occurredAt),
        optId(r.fromAssignee),
        optId(r.toAssignee),
        r.channel ?? null,
        optId(r.inboxId),
      ]),
    )
  }

  async insertBotMessageEvents(rows: BotMessageEventRow[]): Promise<void> {
    this.insertMany(
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
        createId().toString(),
        id(r.workspaceId),
        id(r.messageId),
        id(r.conversationId),
        ms(r.occurredAt),
        flag(r.hasResponse),
        r.responseType ?? null,
        r.routeType ?? null,
        r.result ?? null,
        r.aiProvider ?? null,
        r.channel ?? null,
        optId(r.inboxId),
      ]),
    )
  }

  async insertAgentResponseEvents(rows: AgentResponseEventRow[]): Promise<void> {
    this.insertMany(
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
        createId().toString(),
        id(r.workspaceId),
        id(r.conversationId),
        id(r.contactId),
        id(r.adminId),
        ms(r.occurredAt),
        ms(r.awaitingSince),
        ms(r.measuredFrom),
        r.responseSeconds,
        r.workingResponseSeconds ?? null,
        flag(r.isFirstResponse),
        r.slaTargetSeconds ?? null,
        flag(r.slaMet),
        r.channel ?? null,
        optId(r.inboxId),
      ]),
    )
  }

  async insertContactActiveHourly(rows: ContactActiveHourlyRow[]): Promise<void> {
    this.insertMany(
      "ContactActiveHourly",
      ["workspaceId", "contactId", "contactInboxId", "hourBucket", "inboxId"],
      rows.map((r) => [id(r.workspaceId), id(r.contactId), id(r.contactInboxId), ms(r.hourBucket), id(r.inboxId)]),
    )
  }

  async insertWorkspaceMac(input: {
    workspaceId: string
    ownerId: string
    periodStart: string
    periodEnd: string
    macCount: number
  }): Promise<void> {
    this.open()
      .prepare(
        `INSERT INTO "WorkspaceMac" ("id", "workspaceId", "ownerId", "tenantId", "periodStart", "periodEnd", "macCount")
         VALUES (?, ?, ?, (SELECT "tenantId" FROM "Workspace" WHERE "id" = ?), ?, ?, ?)`,
      )
      .run(
        createId(),
        id(input.workspaceId),
        id(input.ownerId),
        id(input.workspaceId),
        ms(input.periodStart),
        ms(input.periodEnd),
        input.macCount,
      )
  }

  async createBroadcast(input: { workspaceId: string; name: string; channel: string }): Promise<{ broadcastId: string }> {
    const broadcastId = createId()
    const now = Date.now()
    this.open()
      .prepare(
        `INSERT INTO "Broadcast"
           ("id", "name", "workspaceId", "status", "schedulesType", "schedulesAt", "subaction", "channel", "handoffCompletedAt")
         VALUES (?, ?, ?, 'sent', 'now', ?, 'allContacts', ?, ?)`,
      )
      .run(broadcastId, input.name, id(input.workspaceId), now, input.channel, now)
    return { broadcastId: broadcastId.toString() }
  }

  async insertBroadcastRecipients(
    rows: Array<{ broadcastId: string; contactId: string; contactInboxId: string; conversationId: string } & RecipientTimestamps>,
  ): Promise<void> {
    this.insertMany(
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
        id(r.broadcastId),
        id(r.contactId),
        id(r.contactInboxId),
        id(r.conversationId),
        1,
        optMs(r.deliveredAt),
        optMs(r.seenAt),
        optMs(r.clickedAt),
        optMs(r.failedAt),
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
    const enrollments = new Map<string, bigint>()
    const now = Date.now()
    this.transaction((db) => {
      const enroll = db.prepare(
        `INSERT INTO "ContactOnSequence" ("id", "workspaceId", "sequenceId", "contactId", "status", "completedAt", "currentStep")
         VALUES (?, ?, ?, ?, 'completed', ?, 1)`,
      )
      for (const r of rows) {
        const key = `${r.workspaceId}|${r.sequenceId}|${r.contactId}`
        if (!enrollments.has(key)) {
          const enrollmentId = createId()
          enrollments.set(key, enrollmentId)
          enroll.run(enrollmentId, id(r.workspaceId), id(r.sequenceId), id(r.contactId), now)
        }
      }
    })
    this.insertMany(
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
        const dispatchId = createId()
        return [
          dispatchId,
          now,
          "completed",
          `parity:${dispatchId}`,
          now,
          optMs(r.deliveredAt),
          optMs(r.seenAt),
          optMs(r.clickedAt),
          optMs(r.failedAt),
          r.errorContent ?? null,
          id(r.workspaceId),
          id(r.sequenceId),
          id(r.contactId),
          id(r.contactInboxId),
          id(r.stepId),
          enrollments.get(`${r.workspaceId}|${r.sequenceId}|${r.contactId}`) ?? null,
        ]
      }),
    )
  }

  async findFlowAnalyticsSessionId(input: { workspaceId: string; flowId: string }): Promise<string | null> {
    const row = this.open()
      .prepare(
        `SELECT CAST("id" AS TEXT) AS id FROM "FlowAnalyticsSession"
         WHERE "workspaceId" = ? AND "flowId" = ? AND "deletedAt" IS NULL LIMIT 1`,
      )
      .get(id(input.workspaceId), id(input.flowId)) as { id: string } | undefined
    return row?.id ?? null
  }

  async insertFlowNodeStats(rows: FlowNodeStatRow[]): Promise<void> {
    this.insertMany(
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
        id(r.workspaceId),
        id(r.flowId),
        id(r.analyticsId),
        r.nodeId,
        r.buttonId ?? null,
        id(r.contactId),
        id(r.contactInboxId),
        r.eventType,
        r.errorContent ?? null,
        ms(r.occurredAt),
      ]),
    )
  }

  async insertSmartDelays(rows: SmartDelayRow[]): Promise<void> {
    this.insertMany(
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
        id(r.workspaceId),
        id(r.flowId),
        optId(r.flowVersionId),
        id(r.contactInboxId),
        id(r.conversationId),
        r.nodeId ?? null,
        r.stepId ?? null,
        r.type,
        ms(r.triggerAt),
        r.status,
      ]),
    )
  }

  async createCommentAutomation(input: {
    workspaceId: string
    inboxId: string
    name: string
    type?: "messenger" | "instagram"
    counters?: CommentAutomationCounters
  }): Promise<{ automationId: string }> {
    const automationId = createId()
    const c = input.counters ?? {}
    this.open()
      .prepare(
        `INSERT INTO "CommentAutomation"
           ("id", "name", "workspaceId", "type", "inboxId", "isActive",
            "sentCount", "deliveredCount", "seenCount", "clickedCount", "failedCount", "missedCount")
         VALUES (?, ?, ?, ?, ?, 0, ?, ?, ?, ?, ?, ?)`,
      )
      .run(
        automationId,
        input.name,
        id(input.workspaceId),
        input.type ?? "messenger",
        id(input.inboxId),
        c.sentCount ?? 0,
        c.deliveredCount ?? 0,
        c.seenCount ?? 0,
        c.clickedCount ?? 0,
        c.failedCount ?? 0,
        c.missedCount ?? 0,
      )
    return { automationId: automationId.toString() }
  }

  async insertCommentAutomationEvents(rows: CommentAutomationEventRow[]): Promise<{ ids: string[] }> {
    const ids = rows.map(() => createId())
    this.insertMany(
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
        ids[index] ?? createId(),
        id(r.workspaceId),
        id(r.automationId),
        optId(r.contactId),
        optId(r.contactInboxId),
        r.postId,
        r.commentId,
        r.commentText ?? null,
        r.replyChannel,
        r.replyType,
        r.replyText ?? null,
        r.status,
        optMs(r.deliveredAt),
        optMs(r.seenAt),
        optMs(r.clickedAt),
        optMs(r.failedAt),
        r.errorDetail ?? null,
        r.httpCode ?? null,
        ms(r.occurredAt),
      ]),
    )
    return { ids: ids.map(String) }
  }

  async insertLinkStats(kind: "magic" | "ref", rows: LinkStatRow[]): Promise<void> {
    this.insertMany(
      kind === "magic" ? "MagicLinkStat" : "RefLinkStat",
      ["workspaceId", "linkId", "contactId", "contactInboxId", "occurredAt"],
      rows.map((r) => [id(r.workspaceId), id(r.linkId), id(r.contactId), id(r.contactInboxId), ms(r.occurredAt)]),
    )
  }

  async close(): Promise<void> {
    this.db?.close()
    this.db = undefined
  }
}

export const create = (): P103Fixtures =>
  new SqliteP103Fixtures(process.env.PARITY_SQLITE_PATH ?? "/rust-data/chatbotx.db")
