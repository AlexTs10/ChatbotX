// Rust-stack implementation of the P1.6 fixture extension
// (`src/fixtures/ext/p1-06.ts`): rows written with `node:sqlite` into the
// shared `/rust-data` database, same columns as `stacks/original/ext/p1-06.ts`
// (dates as epoch ms). Methods are added as the P3.6 contact ports need
// them; the others throw.

import { randomBytes } from "node:crypto"
import { DatabaseSync } from "node:sqlite"
import type {
  P106ContactInput,
  P106CreatedContact,
  P106Fixtures,
  P106InboxInput,
} from "../../../src/fixtures/ext/p1-06.ts"

const SNOWFLAKE_EPOCH_MS = Date.parse("2004-02-01T00:00:00.000Z")
// A place id of its own (p1-12 uses 4), so fixture ids never collide.
const PLACE_ID = 5n

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

const notPorted = (name: string) => (): never => {
  throw new Error(`p1-06 fixture ${name} is not ported to the Rust stack yet`)
}

class SqliteP106Fixtures implements P106Fixtures {
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

  createAdReferral = notPorted("createAdReferral")
  insertSequenceDispatch = notPorted("insertSequenceDispatch")

  async createInbox(input: P106InboxInput): Promise<{ inboxId: string }> {
    const inboxId = createId()
    this.open()
      .prepare(
        `INSERT INTO "Inbox" ("id", "name", "channel", "sourceId", "workspaceId", "status")
         VALUES (?, ?, ?, ?, ?, 'connected')`,
      )
      .run(inboxId, input.name, input.channel, input.sourceId ?? uniqueText("src"), BigInt(input.workspaceId))
    return { inboxId: inboxId.toString() }
  }

  async createContact(input: P106ContactInput): Promise<P106CreatedContact> {
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
            "gender", "country", "locale", "timezone", "avatar", "blockedAt", "broadcastSubscribedAt",
            "subscribedAt", "emailVerified", "emailOptIn")
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)`,
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
        input.country ?? null,
        input.locale ?? null,
        input.timezone ?? null,
        input.avatar ?? null,
        ms(input.blockedAt),
        ms(input.broadcastSubscribedAt),
        ms(input.subscribedAt),
        input.emailVerified ? 1 : 0,
        input.emailOptIn === false ? 0 : 1,
      )
      for (const [index, entry] of inboxes.entries()) {
        db.prepare(
          `INSERT INTO "ContactInbox"
             ("id", "createdAt", "updatedAt", "originalContactId", "contactId", "inboxId", "channel", "source",
              "sourceId", "language", "contactLastReadAt", "lastIncomingMessageAt", "lastOutboundMessageAt",
              "consecutiveFailedReply", "lastUserInput")
           VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)`,
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
          entry.language ?? null,
          ms(entry.contactLastReadAt),
          ms(entry.lastIncomingMessageAt),
          ms(entry.lastOutboundMessageAt),
          entry.consecutiveFailedReply ?? 0,
          entry.lastUserInput ?? null,
        )
      }
      if (conversationId) {
        const conversation = input.conversation ?? {}
        db.prepare(
          `INSERT INTO "Conversation"
             ("id", "createdAt", "updatedAt", "workspaceId", "contactId", "archivedAt", "followed", "botEnabled",
              "lastActivityAt", "agentLastReadAt", "assignedUserId")
           VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)`,
        ).run(
          conversationId,
          createdAt,
          createdAt,
          BigInt(input.workspaceId),
          contactId,
          ms(conversation.archivedAt),
          conversation.followed ? 1 : 0,
          conversation.botEnabled === false ? 0 : 1,
          ms(conversation.lastActivityAt),
          ms(conversation.agentLastReadAt),
          id(conversation.assignedUserId),
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

  async linkTag(input: { contactId: string; tagId: string }): Promise<void> {
    this.open()
      .prepare(`INSERT INTO "ContactToTag" ("contactId", "tagId") VALUES (?, ?)`)
      .run(BigInt(input.contactId), BigInt(input.tagId))
  }

  async setCustomFieldValue(input: { contactId: string; customFieldId: string; value: string }): Promise<void> {
    this.open()
      .prepare(
        `INSERT INTO "ContactCustomField" ("id", "contactId", "customFieldId", "value")
         VALUES (?, ?, ?, ?)
         ON CONFLICT ("contactId", "customFieldId") DO UPDATE SET "value" = excluded."value",
           "updatedAt" = CAST(unixepoch('subsec') * 1000 AS INTEGER)`,
      )
      .run(createId(), BigInt(input.contactId), BigInt(input.customFieldId), input.value)
  }

  async setWorkspaceTargetCountry(input: { workspaceId: string; targetCountry: string | null }): Promise<void> {
    this.open()
      .prepare(`UPDATE "Workspace" SET "targetCountry" = ? WHERE "id" = ?`)
      .run(input.targetCountry, BigInt(input.workspaceId))
  }

  async contactIdsByEmail(input: { workspaceId: string; emails: string[] }): Promise<Record<string, string>> {
    const rows = this.open()
      .prepare(
        `SELECT CAST("id" AS TEXT) AS "id", "email" FROM "Contact"
         WHERE "workspaceId" = ? AND "email" IN (SELECT value FROM json_each(?))`,
      )
      .all(BigInt(input.workspaceId), JSON.stringify(input.emails)) as Array<{ id: string; email: string }>
    return Object.fromEntries(rows.map((row) => [row.email, row.id]))
  }

  async close(): Promise<void> {
    this.db?.close()
    this.db = undefined
  }
}

export const create = (): P106Fixtures =>
  new SqliteP106Fixtures(process.env.PARITY_SQLITE_PATH ?? "/rust-data/chatbotx.db")
