// Rust-stack implementation of the P1.10 fixture extension
// (`src/fixtures/ext/p1-10.ts`): rows written with `node:sqlite` into the
// shared `/rust-data` database, same columns as `stacks/original/ext/p1-10.ts`
// (dates as epoch ms, JSON as text). Methods are added as the ports need
// them; the others throw.

import { randomBytes } from "node:crypto"
import { DatabaseSync } from "node:sqlite"
import type {
  P110ContactInput,
  P110Fixtures,
  P110InboxInput,
  P110InstagramInput,
} from "../../../src/fixtures/ext/p1-10.ts"

const SNOWFLAKE_EPOCH_MS = Date.parse("2004-02-01T00:00:00.000Z")
// A place id of its own (p1-12 uses 4, p1-06 5, p1-09 6, p1-07 7, p1-11 8, p1-03 9).
const PLACE_ID = 10n

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

const notPorted = (name: string) => (): never => {
  throw new Error(`p1-10 fixture ${name} is not ported to the Rust stack yet`)
}

class SqliteP110Fixtures implements P110Fixtures {
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

  private insertInbox(db: DatabaseSync, input: P110InboxInput): bigint {
    const inboxId = createId()
    const status = input.status ?? "connected"
    db.prepare(
      `INSERT INTO "Inbox" ("id", "name", "channel", "sourceId", "workspaceId", "status", "disconnectedAt")
       VALUES (?, ?, ?, ?, ?, ?, ?)`,
    ).run(
      inboxId,
      input.name,
      input.channel,
      input.sourceId ?? uniqueText("src"),
      BigInt(input.workspaceId),
      status,
      status === "disconnected" ? Date.now() : null,
    )
    return inboxId
  }

  async createInbox(input: P110InboxInput): Promise<{ inboxId: string }> {
    return { inboxId: this.transaction((db) => this.insertInbox(db, input)).toString() }
  }

  async createInstagramChannel(input: P110InstagramInput): Promise<{ integrationId: string; inboxId: string }> {
    return this.transaction((db) => {
      const inboxId = this.insertInbox(db, {
        workspaceId: input.workspaceId,
        name: input.name,
        channel: "instagram",
        sourceId: input.igId,
      })
      const integrationId = createId()
      const auth = {
        authType: "oauth2",
        clientId: "parity-client",
        clientSecret: "parity-secret",
        redirectUrl: "http://app/integrations/instagram/callback",
        version: "v23.0",
        tokens: { accessToken: `ig-token-${input.igId}` },
        metadata: { igId: input.igId, pageId: input.pageId, version: "v23.0" },
      }
      db.prepare(
        `INSERT INTO "IntegrationInstagram"
           ("id", "auth", "igId", "pageId", "name", "username", "workspaceId", "inboxId", "type",
            "conversationStarters", "persistentMenus", "hasCapiScope", "datasetId", "capiTestEventCode",
            "capiDisconnectedAt", "tokenRefreshError")
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, '[]', '[]', ?, ?, ?, ?, ?)`,
      ).run(
        integrationId,
        JSON.stringify(auth),
        input.igId,
        input.pageId,
        input.name,
        input.username,
        BigInt(input.workspaceId),
        inboxId,
        input.type,
        input.hasCapiScope ? 1 : 0,
        input.datasetId ?? null,
        input.capiTestEventCode ?? null,
        input.capiDisconnected ? Date.now() : null,
        input.tokenRefreshError ?? null,
      )
      db.prepare(
        `INSERT INTO "Connection"
           ("id", "workspaceId", "provider", "kind", "channel", "inboxId", "sourceId", "displayName", "status", "connectedAt")
         VALUES (?, ?, ?, 'channel', 'instagram', ?, ?, ?, 'connected', ?)`,
      ).run(
        createId(),
        BigInt(input.workspaceId),
        input.type === "facebook" ? "instagramFacebook" : "instagram",
        inboxId,
        input.igId,
        input.name,
        Date.now(),
      )
      return { integrationId: integrationId.toString(), inboxId: inboxId.toString() }
    })
  }

  async createMessengerPage(input: { workspaceId: string; pageId: string; name: string }) {
    return this.transaction((db) => {
      const inboxId = this.insertInbox(db, {
        workspaceId: input.workspaceId,
        name: input.name,
        channel: "messenger",
        sourceId: input.pageId,
      })
      const integrationId = createId()
      const auth = {
        authType: "oauth2",
        clientId: "parity-client",
        clientSecret: "parity-secret",
        redirectUrl: "http://app/integrations/callback",
        version: "v23.0",
        tokens: { accessToken: `page-token-${input.pageId}` },
        metadata: { pageId: input.pageId, pageName: input.name },
      }
      db.prepare(
        `INSERT INTO "IntegrationMessenger"
           ("id", "auth", "pageId", "name", "workspaceId", "inboxId", "conversationStarters", "persistentMenus", "personas")
         VALUES (?, ?, ?, ?, ?, ?, '[]', '[]', '[]')`,
      ).run(integrationId, JSON.stringify(auth), input.pageId, input.name, BigInt(input.workspaceId), inboxId)
      return { integrationId: integrationId.toString(), inboxId: inboxId.toString() }
    })
  }

  async createContact(input: P110ContactInput) {
    const contactId = createId()
    const createdAt = Date.now()
    const inboxes = input.inboxes ?? []
    const contactInboxIds = inboxes.map(() => createId())
    const conversationId = input.conversation === false ? null : createId()
    this.transaction((db) => {
      db.prepare(
        `INSERT INTO "Contact" ("id", "createdAt", "updatedAt", "workspaceId", "firstName", "lastName", "phoneNumber", "avatar")
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)`,
      ).run(
        contactId,
        createdAt,
        createdAt,
        BigInt(input.workspaceId),
        input.firstName ?? null,
        input.lastName ?? null,
        input.phoneNumber ?? null,
        input.avatar ?? null,
      )
      for (const [index, entry] of inboxes.entries()) {
        db.prepare(
          `INSERT INTO "ContactInbox"
             ("id", "createdAt", "updatedAt", "originalContactId", "contactId", "inboxId", "channel", "source", "sourceId")
           VALUES (?, ?, ?, ?, ?, ?, ?, 'direct', ?)`,
        ).run(
          contactInboxIds[index] ?? createId(),
          createdAt,
          createdAt,
          contactId,
          contactId,
          BigInt(entry.inboxId),
          entry.channel,
          entry.sourceId ?? uniqueText("psid"),
        )
      }
      if (conversationId) {
        db.prepare(
          `INSERT INTO "Conversation" ("id", "createdAt", "updatedAt", "workspaceId", "contactId")
           VALUES (?, ?, ?, ?, ?)`,
        ).run(conversationId, createdAt, createdAt, BigInt(input.workspaceId), contactId)
      }
    })
    return {
      contactId: contactId.toString(),
      contactInboxIds: contactInboxIds.map(String),
      conversationId: conversationId?.toString() ?? null,
    }
  }

  createZaloIntegration = notPorted("createZaloIntegration")
  createIntegration = notPorted("createIntegration")
  createTelegramChat = notPorted("createTelegramChat")

  async close(): Promise<void> {
    this.db?.close()
    this.db = undefined
  }
}

export const create = (): P110Fixtures =>
  new SqliteP110Fixtures(process.env.PARITY_SQLITE_PATH ?? "/rust-data/chatbotx.db")
