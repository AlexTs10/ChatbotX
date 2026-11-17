// Rust-stack implementation of the P1.10 fixture extension
// (`src/fixtures/ext/p1-10.ts`): rows written with `node:sqlite` into the
// shared `/rust-data` database, same logical rows as
// `stacks/original/ext/p1-10.ts` (dates as epoch ms, JSON as text, booleans
// as 0/1; the Postgres `jsonb[]` defaults are JSON arrays here).

import { randomBytes } from "node:crypto"
import { DatabaseSync } from "node:sqlite"
import type { P110ContactInput, P110Fixtures, P110InboxInput, P110InstagramInput } from "../../../src/fixtures/ext/p1-10.ts"

const SNOWFLAKE_EPOCH_MS = Date.parse("2004-02-01T00:00:00.000Z")
// A place id of its own (p1-12 uses 4, p1-06 5, p1-09 6, p1-07 7, p1-11 8, p1-03 9, p1-05 10).
const PLACE_ID = 13n

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
    const now = Date.now()
    const status = input.status ?? "connected"
    db.prepare(
      `INSERT INTO "Inbox" ("id", "createdAt", "updatedAt", "name", "channel", "sourceId", "workspaceId", "status",
         "disconnectedAt")
       VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)`,
    ).run(
      inboxId,
      now,
      now,
      input.name,
      input.channel,
      input.sourceId ?? uniqueText("src"),
      BigInt(input.workspaceId),
      status,
      status === "disconnected" ? now : null,
    )
    return inboxId
  }

  async createInbox(input: P110InboxInput): Promise<{ inboxId: string }> {
    return { inboxId: String(this.insertInbox(this.open(), input)) }
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
      const now = Date.now()
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
           ("id", "createdAt", "updatedAt", "auth", "igId", "pageId", "name", "username", "workspaceId", "inboxId",
            "type", "conversationStarters", "persistentMenus", "hasCapiScope", "datasetId", "capiTestEventCode",
            "capiDisconnectedAt", "tokenRefreshError")
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, '[]', '[]', ?, ?, ?, ?, ?)`,
      ).run(
        integrationId,
        now,
        now,
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
        input.capiDisconnected ? now : null,
        input.tokenRefreshError ?? null,
      )
      db.prepare(
        `INSERT INTO "Connection"
           ("id", "createdAt", "updatedAt", "workspaceId", "provider", "kind", "channel", "inboxId", "sourceId",
            "displayName", "status", "connectedAt")
         VALUES (?, ?, ?, ?, ?, 'channel', 'instagram', ?, ?, ?, 'connected', ?)`,
      ).run(
        createId(),
        now,
        now,
        BigInt(input.workspaceId),
        input.type === "facebook" ? "instagramFacebook" : "instagram",
        inboxId,
        input.igId,
        input.name,
        now,
      )
      return { integrationId: String(integrationId), inboxId: String(inboxId) }
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
      const now = Date.now()
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
           ("id", "createdAt", "updatedAt", "auth", "pageId", "name", "workspaceId", "inboxId",
            "conversationStarters", "persistentMenus", "personas")
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, '[]', '[]', '[]')`,
      ).run(integrationId, now, now, JSON.stringify(auth), input.pageId, input.name, BigInt(input.workspaceId), inboxId)
      return { integrationId: String(integrationId), inboxId: String(inboxId) }
    })
  }

  async createZaloIntegration(input: {
    workspaceId: string
    name: string
    oaId: string
    tokenRefreshError?: string | null
  }) {
    return this.transaction((db) => {
      const inboxId = this.insertInbox(db, {
        workspaceId: input.workspaceId,
        name: input.name,
        channel: "zalo",
        sourceId: input.oaId,
      })
      const integrationId = createId()
      const now = Date.now()
      db.prepare(
        `INSERT INTO "IntegrationZalo"
           ("id", "createdAt", "updatedAt", "auth", "oaId", "name", "workspaceId", "inboxId", "tokenRefreshError")
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)`,
      ).run(
        integrationId,
        now,
        now,
        JSON.stringify({ authType: "secretText", secretText: "parity-zalo-token" }),
        input.oaId,
        input.name,
        BigInt(input.workspaceId),
        inboxId,
        input.tokenRefreshError ?? null,
      )
      return { integrationId: String(integrationId), inboxId: String(inboxId) }
    })
  }

  async createIntegration(input: { workspaceId: string; integrationType: string }) {
    const integrationId = createId()
    const now = Date.now()
    this.open()
      .prepare(
        `INSERT INTO "Integration" ("id", "createdAt", "updatedAt", "workspaceId", "integrationType")
         VALUES (?, ?, ?, ?, ?)`,
      )
      .run(integrationId, now, now, BigInt(input.workspaceId), input.integrationType)
    return { integrationId: String(integrationId) }
  }

  async createContact(input: P110ContactInput) {
    const contactId = createId()
    const now = Date.now()
    const inboxes = input.inboxes ?? []
    const contactInboxIds = inboxes.map(() => createId())
    const conversationId = input.conversation === false ? null : createId()
    this.transaction((db) => {
      db.prepare(
        `INSERT INTO "Contact" ("id", "createdAt", "updatedAt", "workspaceId", "firstName", "lastName", "phoneNumber",
           "avatar")
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)`,
      ).run(
        contactId,
        now,
        now,
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
          contactInboxIds[index] as bigint,
          now,
          now,
          contactId,
          contactId,
          BigInt(entry.inboxId),
          entry.channel,
          entry.sourceId ?? uniqueText("psid"),
        )
      }
      if (conversationId) {
        db.prepare(
          `INSERT INTO "Conversation" ("id", "createdAt", "updatedAt", "workspaceId", "contactId") VALUES (?, ?, ?, ?, ?)`,
        ).run(conversationId, now, now, BigInt(input.workspaceId), contactId)
      }
    })
    return {
      contactId: String(contactId),
      contactInboxIds: contactInboxIds.map(String),
      conversationId: conversationId == null ? null : String(conversationId),
    }
  }

  async createTelegramChat(input: {
    workspaceId: string
    inboxId: string
    chatId: string
    title: string | null
    type: string
    botStatus: string
  }): Promise<void> {
    const now = Date.now()
    this.open()
      .prepare(
        `INSERT INTO "TelegramChat" ("id", "createdAt", "updatedAt", "workspaceId", "inboxId", "chatId", "title", "type",
           "botStatus")
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)`,
      )
      .run(
        createId(),
        now,
        now,
        BigInt(input.workspaceId),
        BigInt(input.inboxId),
        input.chatId,
        input.title,
        input.type,
        input.botStatus,
      )
  }

  async close(): Promise<void> {
    this.db?.close()
    this.db = undefined
  }
}

export const create = (): P110Fixtures =>
  new SqliteP110Fixtures(process.env.PARITY_SQLITE_PATH ?? "/rust-data/chatbotx.db")
