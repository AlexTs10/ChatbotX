// Rust-stack implementation of the P1.3 fixture extension
// (`src/fixtures/ext/p1-03.ts`): rows written with `node:sqlite` into the
// shared `/rust-data` database, same columns as `stacks/original/ext/p1-03.ts`
// (dates as epoch ms, JSON as text). Q.D6 ports what aiHandover needs
// (inbox, contact, member); the analytics rows wait for their unit (Q.D11)
// and throw.

import { randomBytes } from "node:crypto"
import { DatabaseSync } from "node:sqlite"
import type {
  CreateContactInput,
  CreateInboxInput,
  CreatedContact,
  P103Fixtures,
} from "../../../src/fixtures/ext/p1-03.ts"

const SNOWFLAKE_EPOCH_MS = Date.parse("2004-02-01T00:00:00.000Z")
// A place id of its own (p1-12 uses 4, p1-06 5, p1-09 6, p1-07 7, p1-11 8).
const PLACE_ID = 9n

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
const ms = (iso: string | null | undefined): number | null => (iso == null ? null : Date.parse(iso))

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

const notPorted = (name: string) => (): never => {
  throw new Error(`p1-03 fixture ${name} is not ported to the Rust stack yet`)
}

class SqliteP103Fixtures implements P103Fixtures {
  private db: DatabaseSync | undefined

  constructor(private readonly path: string) {}

  private open(): DatabaseSync {
    if (!this.db) {
      const db = new DatabaseSync(this.path)
      db.exec("PRAGMA busy_timeout = 10000; PRAGMA foreign_keys = ON;")
      db.function("regexp_replace", { deterministic: true }, (text, pattern, replacement, flags) =>
        text == null
          ? null
          : String(text).replace(new RegExp(String(pattern), String(flags ?? "")), String(replacement)),
      )
      this.db = db
    }
    return this.db
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
        BigInt(input.workspaceId),
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
    const db = this.open()
    db.exec("BEGIN IMMEDIATE")
    try {
      db.prepare(`INSERT INTO "Contact" ("id", "workspaceId", "firstName", "lastName") VALUES (?, ?, ?, ?)`).run(
        contactId,
        BigInt(input.workspaceId),
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
        BigInt(input.inboxId),
        input.channel,
        input.source ?? "direct",
        sourceId,
        ms(input.lastIncomingMessageAt),
      )
      db.prepare(
        `INSERT INTO "Conversation" ("id", "workspaceId", "contactId", "contactLastReadAt") VALUES (?, ?, ?, ?)`,
      ).run(conversationId, BigInt(input.workspaceId), contactId, ms(input.contactLastReadAt))
      db.exec("COMMIT")
    } catch (error) {
      db.exec("ROLLBACK")
      throw error
    }
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
        BigInt(input.workspaceId),
        BigInt(input.userId),
        JSON.stringify(AGENT_PERMISSIONS),
        JSON.stringify(DEFAULT_MEMBER_NOTIFICATION_TYPES),
        JSON.stringify(DEFAULT_MEMBER_NOTIFICATION_CHANNELS),
      )
    return { memberId: memberId.toString() }
  }

  setResponseSlaPolicy = notPorted("setResponseSlaPolicy")
  insertContactEvents = notPorted("insertContactEvents")
  insertMessageEvents = notPorted("insertMessageEvents")
  insertConversationEvents = notPorted("insertConversationEvents")
  insertBotMessageEvents = notPorted("insertBotMessageEvents")
  insertAgentResponseEvents = notPorted("insertAgentResponseEvents")
  insertContactActiveHourly = notPorted("insertContactActiveHourly")
  insertWorkspaceMac = notPorted("insertWorkspaceMac")
  createBroadcast = notPorted("createBroadcast")
  insertBroadcastRecipients = notPorted("insertBroadcastRecipients")
  insertSequenceDispatches = notPorted("insertSequenceDispatches")
  findFlowAnalyticsSessionId = notPorted("findFlowAnalyticsSessionId")
  insertFlowNodeStats = notPorted("insertFlowNodeStats")
  insertSmartDelays = notPorted("insertSmartDelays")
  createCommentAutomation = notPorted("createCommentAutomation")
  insertCommentAutomationEvents = notPorted("insertCommentAutomationEvents")
  insertLinkStats = notPorted("insertLinkStats")

  async close(): Promise<void> {
    this.db?.close()
    this.db = undefined
  }
}

export const create = (): P103Fixtures =>
  new SqliteP103Fixtures(process.env.PARITY_SQLITE_PATH ?? "/rust-data/chatbotx.db")
