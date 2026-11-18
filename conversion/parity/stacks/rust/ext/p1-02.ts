// Rust-stack implementation of the P1.2 fixture extension
// (`src/fixtures/ext/p1-02.ts`): rows written with `node:sqlite` into the
// shared `/rust-data` database, same logical rows as
// `stacks/original/ext/p1-02.ts` (dates as epoch ms, JSON as text, booleans
// as 0/1). Channel auth is stored as plain JSON (the Rust channels read it
// as is); MessagingAdsConnection / IntegrationFacebookAds auth is
// encryptUtils.encryptObject without AAD, as the original. Key =
// ENCRYPTION_KEY of `stacks/rust/rust.env`.

import { createCipheriv, randomBytes } from "node:crypto"
import { readFileSync } from "node:fs"
import path from "node:path"
import { DatabaseSync } from "node:sqlite"
import { fileURLToPath } from "node:url"
import type { CreatedChannel, P102Fixtures } from "../../../src/fixtures/ext/p1-02.ts"

const here = path.dirname(fileURLToPath(import.meta.url))
const SNOWFLAKE_EPOCH_MS = Date.parse("2004-02-01T00:00:00.000Z")
// A place id of its own so fixture ids never collide with the other units.
const PLACE_ID = 11n

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

const readEnvFile = (): Record<string, string> => {
  const text = readFileSync(path.resolve(here, "../rust.env"), "utf8")
  const values: Record<string, string> = {}
  for (const line of text.split("\n")) {
    const match = /^([A-Z0-9_]+)=(.*)$/.exec(line.trim())
    if (match?.[1]) {
      values[match[1]] = match[2] ?? ""
    }
  }
  return values
}

const env = readEnvFile()
const ENCRYPTION_KEY = Buffer.from(env.ENCRYPTION_KEY ?? "", "hex")
const ENCRYPTION_KEY_ID = env.ENCRYPTION_KEY_ID || "default"

/** encryptUtils.encryptObject(value) of the app (no AAD). */
const encryptObject = (value: unknown) => {
  if (ENCRYPTION_KEY.length !== 32) {
    throw new Error("p1-02 fixtures: ENCRYPTION_KEY must be 32 bytes of hex")
  }
  const iv = randomBytes(12)
  const cipher = createCipheriv("aes-256-gcm", ENCRYPTION_KEY, iv, { authTagLength: 16 })
  const text = Buffer.concat([cipher.update(JSON.stringify(value), "utf8"), cipher.final()])
  return {
    v: 1,
    kid: ENCRYPTION_KEY_ID,
    iv: iv.toString("hex"),
    text: text.toString("hex"),
    tag: cipher.getAuthTag().toString("hex"),
  }
}

const oauth2Auth = (accessToken: string, metadata: Record<string, unknown>) => ({
  authType: "oauth2",
  clientId: "parity-client",
  clientSecret: "parity-secret",
  redirectUrl: "http://app/integrations/callback",
  version: "v23.0",
  tokens: { accessToken },
  metadata: { version: "v23.0", ...metadata },
})

const integrationColumnByChannel = {
  whatsapp: "integrationWhatsappId",
  messenger: "integrationMessengerId",
  instagram: "integrationInstagramId",
} as const

const json = (value: unknown) => (value === undefined || value === null ? null : JSON.stringify(value))
const ms = (iso: string | null | undefined): number | null => (iso == null ? null : Date.parse(iso))
const optId = (value: string | null | undefined): bigint | null => (value == null ? null : BigInt(value))

class SqliteP102Fixtures implements P102Fixtures {
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

  private insertInbox(db: DatabaseSync, input: { workspaceId: string; name: string; channel: string; sourceId: string }) {
    const inboxId = createId()
    const now = Date.now()
    db.prepare(
      `INSERT INTO "Inbox" ("id", "createdAt", "updatedAt", "name", "channel", "sourceId", "workspaceId", "status")
       VALUES (?, ?, ?, ?, ?, ?, ?, 'connected')`,
    ).run(inboxId, now, now, input.name, input.channel, input.sourceId, BigInt(input.workspaceId))
    return inboxId
  }

  async createMessengerChannel(input: { workspaceId: string; pageId: string; name: string }): Promise<CreatedChannel> {
    return this.transaction((db) => {
      const inboxId = this.insertInbox(db, { ...input, channel: "messenger", sourceId: input.pageId })
      const integrationId = createId()
      const now = Date.now()
      db.prepare(
        `INSERT INTO "IntegrationMessenger"
           ("id", "createdAt", "updatedAt", "auth", "pageId", "name", "workspaceId", "inboxId",
            "conversationStarters", "persistentMenus", "personas")
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, '[]', '[]', '[]')`,
      ).run(
        integrationId,
        now,
        now,
        JSON.stringify(oauth2Auth(`page-token-${input.pageId}`, { pageId: input.pageId, pageName: input.name })),
        input.pageId,
        input.name,
        BigInt(input.workspaceId),
        inboxId,
      )
      return { integrationId: String(integrationId), inboxId: String(inboxId) }
    })
  }

  async createWhatsappChannel(input: {
    workspaceId: string
    phoneNumberId: string
    wabaId: string
    name: string
    displayPhoneNumber?: string
  }): Promise<CreatedChannel> {
    return this.transaction((db) => {
      const inboxId = this.insertInbox(db, { ...input, channel: "whatsapp", sourceId: input.phoneNumberId })
      const integrationId = createId()
      const now = Date.now()
      const businessId = `biz-${input.phoneNumberId}`
      db.prepare(
        `INSERT INTO "IntegrationWhatsapp"
           ("id", "createdAt", "updatedAt", "auth", "phoneNumberId", "wabaId", "businessId", "name",
            "displayPhoneNumber", "workspaceId", "inboxId")
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)`,
      ).run(
        integrationId,
        now,
        now,
        JSON.stringify(oauth2Auth(`wa-token-${input.phoneNumberId}`, { wabaId: input.wabaId, businessId })),
        input.phoneNumberId,
        input.wabaId,
        businessId,
        input.name,
        input.displayPhoneNumber ?? "",
        BigInt(input.workspaceId),
        inboxId,
      )
      return { integrationId: String(integrationId), inboxId: String(inboxId) }
    })
  }

  async createInstagramChannel(input: {
    workspaceId: string
    igId: string
    pageId: string
    name: string
    username: string
  }): Promise<CreatedChannel> {
    return this.transaction((db) => {
      const inboxId = this.insertInbox(db, { ...input, channel: "instagram", sourceId: input.igId })
      const integrationId = createId()
      const now = Date.now()
      db.prepare(
        `INSERT INTO "IntegrationInstagram"
           ("id", "createdAt", "updatedAt", "auth", "igId", "pageId", "name", "username", "workspaceId", "inboxId",
            "conversationStarters", "persistentMenus")
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, '[]', '[]')`,
      ).run(
        integrationId,
        now,
        now,
        JSON.stringify(oauth2Auth(`ig-token-${input.igId}`, { igId: input.igId, pageId: input.pageId })),
        input.igId,
        input.pageId,
        input.name,
        input.username,
        BigInt(input.workspaceId),
        inboxId,
      )
      return { integrationId: String(integrationId), inboxId: String(inboxId) }
    })
  }

  async connectMessagingAds(input: {
    workspaceId: string
    channel: "messenger" | "instagram" | "whatsapp"
    integrationId: string
    accessToken: string
    status?: "active" | "invalid"
    corruptAuth?: boolean
  }): Promise<{ connectionId: string }> {
    const connectionId = createId()
    const now = Date.now()
    const auth = input.corruptAuth
      ? { v: 1, kid: ENCRYPTION_KEY_ID, iv: "00".repeat(12), text: "00", tag: "00".repeat(16) }
      : encryptObject({ authType: "custom", accessToken: input.accessToken, version: "v23.0" })
    const column = integrationColumnByChannel[input.channel]
    this.open()
      .prepare(
        `INSERT INTO "MessagingAdsConnection" ("id", "createdAt", "updatedAt", "workspaceId", "channel", "${column}", "auth", "status")
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)`,
      )
      .run(
        connectionId,
        now,
        now,
        BigInt(input.workspaceId),
        input.channel,
        BigInt(input.integrationId),
        JSON.stringify(auth),
        input.status ?? "active",
      )
    return { connectionId: String(connectionId) }
  }

  async connectWorkspaceFacebookAds(input: { workspaceId: string; accessToken: string }) {
    return this.transaction((db) => {
      const integrationId = createId()
      const integrationFacebookAdsId = createId()
      const now = Date.now()
      db.prepare(
        `INSERT INTO "Integration" ("id", "createdAt", "updatedAt", "workspaceId", "integrationType")
         VALUES (?, ?, ?, ?, 'facebookAds')`,
      ).run(integrationId, now, now, BigInt(input.workspaceId))
      db.prepare(
        `INSERT INTO "IntegrationFacebookAds"
           ("id", "createdAt", "updatedAt", "workspaceId", "integrationId", "auth", "tokenExpiresAt", "status")
         VALUES (?, ?, ?, ?, ?, ?, NULL, 'active')`,
      ).run(
        integrationFacebookAdsId,
        now,
        now,
        BigInt(input.workspaceId),
        integrationId,
        JSON.stringify(encryptObject({ authType: "custom", accessToken: input.accessToken, version: "v23.0" })),
      )
      return { integrationFacebookAdsId: String(integrationFacebookAdsId) }
    })
  }

  async createContactInbox(input: {
    workspaceId: string
    inboxId: string
    channel: "messenger" | "instagram" | "whatsapp"
    sourceId: string
    firstName?: string
    lastName?: string
    email?: string
    phoneNumber?: string
    firstInteractionAt?: string
    referral?: Record<string, unknown>
  }) {
    return this.transaction((db) => {
      const contactId = createId()
      const contactInboxId = createId()
      const now = Date.now()
      db.prepare(
        `INSERT INTO "Contact" ("id", "createdAt", "updatedAt", "workspaceId", "firstName", "lastName", "email", "phoneNumber")
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)`,
      ).run(
        contactId,
        now,
        now,
        BigInt(input.workspaceId),
        input.firstName ?? null,
        input.lastName ?? null,
        input.email ?? null,
        input.phoneNumber ?? null,
      )
      db.prepare(
        `INSERT INTO "ContactInbox"
           ("id", "createdAt", "updatedAt", "originalContactId", "contactId", "inboxId", "channel", "source",
            "sourceId", "firstInteractionAt", "referral")
         VALUES (?, ?, ?, ?, ?, ?, ?, 'direct', ?, ?, ?)`,
      ).run(
        contactInboxId,
        now,
        now,
        contactId,
        contactId,
        BigInt(input.inboxId),
        input.channel,
        input.sourceId,
        ms(input.firstInteractionAt),
        json(input.referral),
      )
      return { contactId: String(contactId), contactInboxId: String(contactInboxId) }
    })
  }

  async createConversionEvent(input: Parameters<P102Fixtures["createConversionEvent"]>[0]) {
    const eventId = createId()
    const now = Date.now()
    const column = integrationColumnByChannel[input.channel]
    this.open()
      .prepare(
        `INSERT INTO "AdsConversionEvent"
           ("id", "createdAt", "updatedAt", "workspaceId", "channel", "${column}", "wabaId", "source", "eventType",
            "ctwaClid", "adId", "contactInboxId", "referralEventId", "currency", "value", "orderId", "contents",
            "occurredAt", "sourceEventId", "capiStatus")
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)`,
      )
      .run(
        eventId,
        now,
        now,
        BigInt(input.workspaceId),
        input.channel,
        BigInt(input.integrationId),
        input.wabaId ?? null,
        input.source ?? "rule",
        input.eventType,
        input.ctwaClid ?? null,
        input.adId ?? null,
        optId(input.contactInboxId),
        optId(input.referralEventId),
        input.currency ?? null,
        input.value ?? null,
        input.orderId ?? null,
        json(input.contents),
        ms(input.occurredAt),
        input.sourceEventId,
        input.capiStatus ?? "pending",
      )
    return { eventId: String(eventId) }
  }

  async createReferralEvent(input: Parameters<P102Fixtures["createReferralEvent"]>[0]) {
    const referralEventId = createId()
    const now = Date.now()
    this.open()
      .prepare(
        `INSERT INTO "ContactReferralEvent"
           ("id", "createdAt", "updatedAt", "workspaceId", "contactId", "contactInboxId", "channel", "type",
            "provider", "occurredAt", "adId", "source", "metadata", "dedupKey")
         VALUES (?, ?, ?, ?, ?, ?, ?, 'ad', 'meta', ?, ?, 'ADS', ?, ?)`,
      )
      .run(
        referralEventId,
        now,
        now,
        BigInt(input.workspaceId),
        BigInt(input.contactId),
        BigInt(input.contactInboxId),
        input.channel,
        ms(input.occurredAt),
        input.adId,
        json(input.adTitle === undefined ? undefined : { adTitle: input.adTitle }),
        input.dedupKey,
      )
    return { referralEventId: String(referralEventId) }
  }
}

export const create = (): P102Fixtures =>
  new SqliteP102Fixtures(process.env.PARITY_SQLITE_PATH ?? "/rust-data/chatbotx.db")
