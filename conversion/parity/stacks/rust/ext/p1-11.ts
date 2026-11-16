// Rust-stack implementation of the P1.11 fixture extension
// (`src/fixtures/ext/p1-11.ts`): rows written with `node:sqlite` into the
// shared `/rust-data` database, same columns as `stacks/original/ext/p1-11.ts`
// (dates as epoch ms, JSON as text). Methods are added as the P3.6 ports need
// them; the others throw.

import { randomBytes } from "node:crypto"
import { DatabaseSync } from "node:sqlite"
import type {
  P111Contact,
  P111ContactInput,
  P111Fixtures,
  P111MessageInput,
} from "../../../src/fixtures/ext/p1-11.ts"

const SNOWFLAKE_EPOCH_MS = Date.parse("2004-02-01T00:00:00.000Z")
// A place id of its own (p1-12 uses 4, p1-06 5, p1-09 6), so fixture ids never collide.
const PLACE_ID = 8n

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

const notPorted = (name: string) => (): never => {
  throw new Error(`p1-11 fixture ${name} is not ported to the Rust stack yet`)
}

class SqliteP111Fixtures implements P111Fixtures {
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

  createMessengerPage = notPorted("createMessengerPage")
  createInstagramForPage = notPorted("createInstagramForPage")
  createWhatsappChannel = notPorted("createWhatsappChannel")
  createWhatsappTemplate = notPorted("createWhatsappTemplate")
  createMessengerTemplate = notPorted("createMessengerTemplate")
  setMustChangePassword = notPorted("setMustChangePassword")

  async createContact(input: P111ContactInput): Promise<P111Contact> {
    const contactId = createId()
    const createdAt = Date.now()
    const contactInboxIds = input.inboxes.map(() => createId())
    const sourceIds = input.inboxes.map((entry) => entry.sourceId ?? uniqueText("psid"))
    const conversationId = createId()
    this.transaction((db) => {
      db.prepare(
        `INSERT INTO "Contact" ("id", "createdAt", "updatedAt", "workspaceId", "firstName", "lastName")
         VALUES (?, ?, ?, ?, ?, 'Parity')`,
      ).run(contactId, createdAt, createdAt, BigInt(input.workspaceId), input.firstName)
      for (const [index, entry] of input.inboxes.entries()) {
        db.prepare(
          `INSERT INTO "ContactInbox"
             ("id", "createdAt", "updatedAt", "originalContactId", "contactId", "inboxId", "channel", "source",
              "sourceId", "lastIncomingMessageAt")
           VALUES (?, ?, ?, ?, ?, ?, ?, 'direct', ?, ?)`,
        ).run(
          contactInboxIds[index] as bigint,
          createdAt,
          createdAt,
          contactId,
          contactId,
          BigInt(entry.inboxId),
          entry.channel,
          sourceIds[index] as string,
          ms(entry.lastIncomingMessageAt),
        )
      }
      db.prepare(
        `INSERT INTO "Conversation" ("id", "createdAt", "updatedAt", "workspaceId", "contactId")
         VALUES (?, ?, ?, ?, ?)`,
      ).run(conversationId, createdAt, createdAt, BigInt(input.workspaceId), contactId)
    })
    return {
      contactId: contactId.toString(),
      contactInboxIds: contactInboxIds.map(String),
      sourceIds,
      conversationId: conversationId.toString(),
    }
  }

  async createMessage(input: P111MessageInput): Promise<{ messageId: string; createdAt: string }> {
    const messageId = createId()
    const createdAt = input.createdAt ?? new Date().toISOString()
    const at = Date.parse(createdAt)
    this.open()
      .prepare(
        `INSERT INTO "Message"
           ("id", "createdAt", "updatedAt", "conversationId", "contactInboxId", "workspaceId", "text",
            "contentAttributes", "messageType", "contentType", "senderType", "sourceId", "type", "parentId")
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, 'text', ?, ?, ?, ?)`,
      )
      .run(
        messageId,
        at,
        at,
        BigInt(input.conversationId),
        BigInt(input.contactInboxId),
        BigInt(input.workspaceId),
        input.text ?? null,
        input.contentAttributes ? JSON.stringify(input.contentAttributes) : null,
        input.messageType,
        input.senderType,
        input.sourceId ?? null,
        input.type ?? "message",
        input.parentId ?? null,
      )
    return { messageId: messageId.toString(), createdAt }
  }

  async close(): Promise<void> {
    this.db?.close()
    this.db = undefined
  }
}

export const create = (): P111Fixtures =>
  new SqliteP111Fixtures(process.env.PARITY_SQLITE_PATH ?? "/rust-data/chatbotx.db")
