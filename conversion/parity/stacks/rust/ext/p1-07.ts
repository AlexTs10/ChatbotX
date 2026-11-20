// Rust-stack implementation of the P1.07 fixture extension
// (`src/fixtures/ext/p1-07.ts`): rows written with `node:sqlite` into the
// shared `/rust-data` database, same columns as `stacks/original/ext/p1-07.ts`
// (dates as epoch ms, JSON as text). Methods are added as the P3.6 ports need
// them; the others throw.

import { randomBytes } from "node:crypto"
import { DatabaseSync } from "node:sqlite"
import {
  P107_META_APP_ID,
  type P107ChannelCreated,
  type P107ContactInput,
  type P107CreatedContact,
  type P107Fixtures,
  type P107InboxInput,
  type P107MessengerChannelInput,
  type P107WhatsappChannelInput,
} from "../../../src/fixtures/ext/p1-07.ts"

const SNOWFLAKE_EPOCH_MS = Date.parse("2004-02-01T00:00:00.000Z")
// A place id of its own (p1-12 uses 4, p1-06 5, p1-09 6), so fixture ids never collide.
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
const ms = (iso: string | null | undefined): number | null => (iso == null ? null : Date.parse(iso))
const id = (value: string | null | undefined): bigint | null => (value == null ? null : BigInt(value))

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

/** The channel integration `auth` JSON (plain, as the original stack stores it). */
const oauth2Auth = (accessToken: string, metadata: Record<string, unknown>) => ({
  authType: "oauth2",
  clientId: P107_META_APP_ID,
  clientSecret: "parity-secret",
  redirectUrl: "http://app/integrations/callback",
  version: "v23.0",
  tokens: { accessToken },
  metadata,
})

class SqliteP107Fixtures implements P107Fixtures {
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

  private insertChannel(
    db: DatabaseSync,
    input: { workspaceId: string; name: string; channel: string; sourceId: string },
  ): bigint {
    const inboxId = createId()
    const now = Date.now()
    db.prepare(
      `INSERT INTO "Inbox" ("id", "createdAt", "updatedAt", "name", "channel", "sourceId", "workspaceId", "status")
       VALUES (?, ?, ?, ?, ?, ?, ?, 'connected')`,
    ).run(inboxId, now, now, input.name, input.channel, input.sourceId, BigInt(input.workspaceId))
    db.prepare(
      `INSERT INTO "Connection"
         ("id", "createdAt", "updatedAt", "workspaceId", "provider", "kind", "channel", "inboxId", "sourceId",
          "displayName", "status", "connectedAt")
       VALUES (?, ?, ?, ?, ?, 'channel', ?, ?, ?, ?, 'connected', ?)`,
    ).run(
      createId(),
      now,
      now,
      BigInt(input.workspaceId),
      input.channel,
      input.channel,
      inboxId,
      input.sourceId,
      input.name,
      now,
    )
    return inboxId
  }

  async createWhatsappChannel(input: P107WhatsappChannelInput): Promise<P107ChannelCreated> {
    const accessToken = `wa-token-${input.phoneNumberId}`
    const wabaId = input.wabaId ?? `waba-${input.phoneNumberId}`
    const businessId = `biz-${input.phoneNumberId}`
    const displayPhoneNumber = input.displayPhoneNumber ?? "+1 555 0100"
    return this.transaction((db) => {
      const inboxId = this.insertChannel(db, {
        workspaceId: input.workspaceId,
        name: input.name,
        channel: "whatsapp",
        sourceId: input.phoneNumberId,
      })
      const integrationId = createId()
      const now = Date.now()
      db.prepare(
        `INSERT INTO "IntegrationWhatsapp"
           ("id", "createdAt", "updatedAt", "auth", "phoneNumberId", "wabaId", "businessId", "name",
            "displayPhoneNumber", "workspaceId", "inboxId")
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)`,
      ).run(
        integrationId,
        now,
        now,
        JSON.stringify(
          oauth2Auth(accessToken, {
            wabaId,
            businessId,
            webhookUrl: "http://app/api/hooks/whatsapp",
            phoneNumber: {
              id: input.phoneNumberId,
              verified_name: input.name,
              code_verification_status: "VERIFIED",
              display_phone_number: displayPhoneNumber,
              quality_rating: "GREEN",
              platform_type: "CLOUD_API",
              throughput: { level: "STANDARD" },
              webhook_configuration: {},
            },
          }),
        ),
        input.phoneNumberId,
        wabaId,
        businessId,
        input.name,
        displayPhoneNumber,
        BigInt(input.workspaceId),
        inboxId,
      )
      return { inboxId: inboxId.toString(), integrationId: integrationId.toString(), accessToken }
    })
  }

  async createMessengerChannel(input: P107MessengerChannelInput): Promise<P107ChannelCreated> {
    const accessToken = `page-token-${input.pageId}`
    return this.transaction((db) => {
      const inboxId = this.insertChannel(db, {
        workspaceId: input.workspaceId,
        name: input.name,
        channel: "messenger",
        sourceId: input.pageId,
      })
      const integrationId = createId()
      db.prepare(
        `INSERT INTO "IntegrationMessenger"
           ("id", "auth", "pageId", "name", "workspaceId", "inboxId", "conversationStarters", "persistentMenus", "personas")
         VALUES (?, ?, ?, ?, ?, ?, '[]', '[]', '[]')`,
      ).run(
        integrationId,
        JSON.stringify(oauth2Auth(accessToken, { pageId: input.pageId, pageName: input.name })),
        input.pageId,
        input.name,
        BigInt(input.workspaceId),
        inboxId,
      )
      return { inboxId: inboxId.toString(), integrationId: integrationId.toString(), accessToken }
    })
  }

  async createInbox(input: P107InboxInput): Promise<{ inboxId: string }> {
    const inboxId = createId()
    this.open()
      .prepare(
        `INSERT INTO "Inbox" ("id", "name", "channel", "sourceId", "workspaceId", "status")
         VALUES (?, ?, ?, ?, ?, 'connected')`,
      )
      .run(inboxId, input.name, input.channel, input.sourceId ?? uniqueText("src"), BigInt(input.workspaceId))
    return { inboxId: inboxId.toString() }
  }

  async createContact(input: P107ContactInput): Promise<P107CreatedContact> {
    const contactId = createId()
    const createdAt = ms(input.createdAt) ?? Date.now()
    const inboxes = input.inboxes ?? []
    const contactInboxIds = inboxes.map(() => createId())
    const sourceIds = inboxes.map((entry) => entry.sourceId ?? uniqueText("psid"))
    const conversationId = input.conversation === null ? null : createId()
    this.transaction((db) => {
      db.prepare(
        `INSERT INTO "Contact"
           ("id", "createdAt", "updatedAt", "workspaceId", "firstName", "lastName", "email", "phoneNumber",
            "gender", "avatar", "blockedAt")
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)`,
      ).run(
        contactId,
        createdAt,
        createdAt,
        BigInt(input.workspaceId),
        input.firstName ?? null,
        input.lastName ?? null,
        input.email ?? null,
        input.phoneNumber ?? null,
        input.gender ?? null,
        input.avatar ?? null,
        ms(input.blockedAt),
      )
      for (const [index, entry] of inboxes.entries()) {
        db.prepare(
          `INSERT INTO "ContactInbox"
             ("id", "createdAt", "updatedAt", "originalContactId", "contactId", "inboxId", "channel", "source",
              "sourceId", "sourceUserId", "sourceUsername", "language", "contactLastReadAt", "lastMessageAt",
              "lastIncomingMessageAt", "threadControlState", "threadOwnerRole", "threadOwnerAppId",
              "threadControlUpdatedAt", "threadOwnerExpiresAt", "referral")
           VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)`,
        ).run(
          contactInboxIds[index] as bigint,
          createdAt,
          createdAt,
          contactId,
          contactId,
          BigInt(entry.inboxId),
          entry.channel,
          entry.source ?? "direct",
          sourceIds[index] as string,
          entry.sourceUserId ?? null,
          entry.sourceUsername ?? null,
          entry.language ?? null,
          ms(entry.contactLastReadAt),
          ms(entry.lastMessageAt),
          ms(entry.lastIncomingMessageAt),
          entry.threadControlState ?? null,
          entry.threadOwnerRole ?? null,
          entry.threadOwnerAppId ?? null,
          ms(entry.threadControlUpdatedAt),
          ms(entry.threadOwnerExpiresAt),
          entry.referral === undefined || entry.referral === null ? null : JSON.stringify(entry.referral),
        )
      }
      if (conversationId) {
        const c = input.conversation ?? {}
        const conversationCreatedAt = ms(c.createdAt) ?? Date.now()
        db.prepare(
          `INSERT INTO "Conversation"
             ("id", "createdAt", "updatedAt", "workspaceId", "contactId", "archivedAt", "followed", "botEnabled",
              "lastActivityAt", "agentLastReadAt", "contactLastReadAt", "assignedUserId", "assignedInboxTeamId",
              "adminRepliedAt", "contactRepliedAt", "responseDueAt", "responseSlaBreachingAt")
           VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)`,
        ).run(
          conversationId,
          conversationCreatedAt,
          conversationCreatedAt,
          BigInt(input.workspaceId),
          contactId,
          ms(c.archivedAt),
          c.followed ? 1 : 0,
          c.botEnabled === false ? 0 : 1,
          ms(c.lastActivityAt),
          ms(c.agentLastReadAt),
          ms(c.contactLastReadAt),
          id(c.assignedUserId),
          id(c.assignedInboxTeamId),
          ms(c.adminRepliedAt),
          ms(c.contactRepliedAt),
          ms(c.responseDueAt),
          ms(c.responseSlaBreachingAt),
        )
      }
    })
    return {
      contactId: contactId.toString(),
      contactInboxIds: contactInboxIds.map(String),
      sourceIds,
      conversationId: conversationId?.toString() ?? null,
    }
  }

  async createInboxTeam(input: { workspaceId: string; name: string }): Promise<{ inboxTeamId: string }> {
    const inboxTeamId = createId()
    this.open()
      .prepare(`INSERT INTO "InboxTeam" ("id", "workspaceId", "name") VALUES (?, ?, ?)`)
      .run(inboxTeamId, BigInt(input.workspaceId), input.name)
    return { inboxTeamId: inboxTeamId.toString() }
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

  async close(): Promise<void> {
    this.db?.close()
    this.db = undefined
  }
}

export const create = (): P107Fixtures =>
  new SqliteP107Fixtures(process.env.PARITY_SQLITE_PATH ?? "/rust-data/chatbotx.db")
