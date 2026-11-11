// Postgres implementation of the P1.2 fixture extension (original stack).
// Writes the same logical rows the app writes; see the comment per method.
//
// Row sources (baseline 40e733cee):
// - Inbox + IntegrationMessenger/-Whatsapp/-Instagram: the channel connect
//   services (inboxService.create + the integration insert); same shape as
//   apps/worker/__tests__/integration/support/seed.ts.
// - MessagingAdsConnection: messagingAdsConnectionService.upsertFromOAuth
//   (packages/business/src/messaging-ads-connection/service.ts), auth =
//   encryptUtils.encryptObject({authType:"custom", accessToken, version}).
// - Integration + IntegrationFacebookAds: integrationFacebookAdsService.upsert
//   (packages/business/src/integration-facebook-ads/service.ts).
// - Contact/ContactInbox/AdsConversionEvent/ContactReferralEvent: the rows the
//   inbound webhook + conversion pipelines write
//   (packages/database/src/schema/*.ts).
// Encryption: AES-256-GCM, 12-byte IV, 16-byte tag, hex fields,
// {v:1, kid:"default", iv, text, tag} (packages/encryption/src/encryption.ts),
// key = ENCRYPTION_KEY of stacks/original/original.env.

import { createCipheriv, randomBytes } from "node:crypto"
import { readFileSync } from "node:fs"
import path from "node:path"
import { fileURLToPath } from "node:url"
import pg from "pg"
import type { P102AdsFixtures, P102AiFixtures, P102Fixtures } from "../../../src/fixtures/ext/p1-02.ts"

const here = path.dirname(fileURLToPath(import.meta.url))
const SNOWFLAKE_EPOCH_MS = Date.parse("2004-02-01T00:00:00.000Z")

const readEnvFile = (): Record<string, string> => {
  const text = readFileSync(path.resolve(here, "../original.env"), "utf8")
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

/** Snowflake id generator per place id (app processes use 1-3, core fixtures 15). */
export const idGenerator = (placeId: bigint) => {
  let lastMs = 0n
  let sequence = 0n
  return (): string => {
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
    return ((now << 14n) | (placeId << 10n) | sequence).toString()
  }
}

/** encryptUtils.encryptObject of the app. */
export const encryptObject = (value: unknown) => {
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

const pool = new pg.Pool({
  connectionString: process.env.PARITY_PG_URL ?? "postgresql://chatbotx:secretkey@postgres:5432/chatbotx",
  max: 2,
  idleTimeoutMillis: 1_000,
  allowExitOnIdle: true,
})

const transaction = async <T>(fn: (client: pg.PoolClient) => Promise<T>): Promise<T> => {
  const client = await pool.connect()
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

const json = (value: unknown) => (value === undefined || value === null ? null : JSON.stringify(value))

// ---- ads: begin ----

const adsId = idGenerator(14n)

const insertInbox = async (
  client: pg.PoolClient,
  input: { workspaceId: string; name: string; channel: string; sourceId: string },
): Promise<string> => {
  const inboxId = adsId()
  await client.query(
    `INSERT INTO "Inbox" ("id", "name", "channel", "sourceId", "workspaceId")
     VALUES ($1, $2, $3, $4, $5)`,
    [inboxId, input.name, input.channel, input.sourceId, input.workspaceId],
  )
  return inboxId
}

const oauth2Auth = (accessToken: string, metadata: Record<string, unknown>) =>
  encryptObject({
    authType: "oauth2",
    clientId: "parity-client",
    clientSecret: "parity-secret",
    redirectUrl: "http://app/integrations/callback",
    version: "v23.0",
    tokens: { accessToken },
    metadata,
  })

const integrationColumnByChannel = {
  whatsapp: "integrationWhatsappId",
  messenger: "integrationMessengerId",
  instagram: "integrationInstagramId",
} as const

const adsFixtures: P102AdsFixtures = {
  async createMessengerChannel(input) {
    return transaction(async (client) => {
      const inboxId = await insertInbox(client, {
        workspaceId: input.workspaceId,
        name: input.name,
        channel: "messenger",
        sourceId: input.pageId,
      })
      const integrationId = adsId()
      await client.query(
        `INSERT INTO "IntegrationMessenger"
           ("id", "auth", "pageId", "name", "workspaceId", "inboxId", "conversationStarters", "persistentMenus", "personas")
         VALUES ($1, $2, $3, $4, $5, $6, '[]'::jsonb, '[]'::jsonb, '[]'::jsonb)`,
        [
          integrationId,
          JSON.stringify(oauth2Auth(`page-token-${input.pageId}`, { pageId: input.pageId, pageName: input.name })),
          input.pageId,
          input.name,
          input.workspaceId,
          inboxId,
        ],
      )
      return { integrationId, inboxId }
    })
  },

  async createWhatsappChannel(input) {
    return transaction(async (client) => {
      const inboxId = await insertInbox(client, {
        workspaceId: input.workspaceId,
        name: input.name,
        channel: "whatsapp",
        sourceId: input.phoneNumberId,
      })
      const integrationId = adsId()
      await client.query(
        `INSERT INTO "IntegrationWhatsapp"
           ("id", "auth", "phoneNumberId", "wabaId", "businessId", "name", "displayPhoneNumber", "workspaceId", "inboxId")
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)`,
        [
          integrationId,
          JSON.stringify(
            oauth2Auth(`wa-token-${input.phoneNumberId}`, {
              wabaId: input.wabaId,
              businessId: `biz-${input.phoneNumberId}`,
            }),
          ),
          input.phoneNumberId,
          input.wabaId,
          `biz-${input.phoneNumberId}`,
          input.name,
          input.displayPhoneNumber ?? "",
          input.workspaceId,
          inboxId,
        ],
      )
      return { integrationId, inboxId }
    })
  },

  async createInstagramChannel(input) {
    return transaction(async (client) => {
      const inboxId = await insertInbox(client, {
        workspaceId: input.workspaceId,
        name: input.name,
        channel: "instagram",
        sourceId: input.igId,
      })
      const integrationId = adsId()
      await client.query(
        `INSERT INTO "IntegrationInstagram"
           ("id", "auth", "igId", "pageId", "name", "username", "workspaceId", "inboxId", "conversationStarters", "persistentMenus")
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, '{}'::jsonb[], '{}'::jsonb[])`,
        [
          integrationId,
          JSON.stringify(oauth2Auth(`ig-token-${input.igId}`, { igId: input.igId, pageId: input.pageId })),
          input.igId,
          input.pageId,
          input.name,
          input.username,
          input.workspaceId,
          inboxId,
        ],
      )
      return { integrationId, inboxId }
    })
  },

  async connectMessagingAds(input) {
    const connectionId = adsId()
    const auth = input.corruptAuth
      ? { v: 1, kid: ENCRYPTION_KEY_ID, iv: "00".repeat(12), text: "00", tag: "00".repeat(16) }
      : encryptObject({ authType: "custom", accessToken: input.accessToken, version: "v23.0" })
    const column = integrationColumnByChannel[input.channel]
    await pool.query(
      `INSERT INTO "MessagingAdsConnection" ("id", "workspaceId", "channel", "${column}", "auth", "status")
       VALUES ($1, $2, $3, $4, $5, $6)`,
      [connectionId, input.workspaceId, input.channel, input.integrationId, JSON.stringify(auth), input.status ?? "active"],
    )
    return { connectionId }
  },

  async connectWorkspaceFacebookAds(input) {
    return transaction(async (client) => {
      const integrationId = adsId()
      const integrationFacebookAdsId = adsId()
      await client.query(
        `INSERT INTO "Integration" ("id", "workspaceId", "integrationType") VALUES ($1, $2, 'facebookAds')`,
        [integrationId, input.workspaceId],
      )
      await client.query(
        `INSERT INTO "IntegrationFacebookAds" ("id", "workspaceId", "integrationId", "auth", "tokenExpiresAt", "status")
         VALUES ($1, $2, $3, $4, NULL, 'active')`,
        [
          integrationFacebookAdsId,
          input.workspaceId,
          integrationId,
          JSON.stringify(encryptObject({ authType: "custom", accessToken: input.accessToken, version: "v23.0" })),
        ],
      )
      return { integrationFacebookAdsId }
    })
  },

  async createContactInbox(input) {
    return transaction(async (client) => {
      const contactId = adsId()
      const contactInboxId = adsId()
      await client.query(
        `INSERT INTO "Contact" ("id", "workspaceId", "firstName", "lastName", "email", "phoneNumber")
         VALUES ($1, $2, $3, $4, $5, $6)`,
        [
          contactId,
          input.workspaceId,
          input.firstName ?? null,
          input.lastName ?? null,
          input.email ?? null,
          input.phoneNumber ?? null,
        ],
      )
      await client.query(
        `INSERT INTO "ContactInbox"
           ("id", "originalContactId", "contactId", "inboxId", "channel", "source", "sourceId", "firstInteractionAt", "referral")
         VALUES ($1, $2, $3, $4, $5, 'direct', $6, $7, $8)`,
        [
          contactInboxId,
          contactId,
          contactId,
          input.inboxId,
          input.channel,
          input.sourceId,
          input.firstInteractionAt ?? null,
          json(input.referral),
        ],
      )
      return { contactId, contactInboxId }
    })
  },

  async createConversionEvent(input) {
    const eventId = adsId()
    const column = integrationColumnByChannel[input.channel]
    await pool.query(
      `INSERT INTO "AdsConversionEvent"
         ("id", "workspaceId", "channel", "${column}", "wabaId", "source", "eventType", "ctwaClid", "adId",
          "contactInboxId", "referralEventId", "currency", "value", "orderId", "contents", "occurredAt",
          "sourceEventId", "capiStatus")
       VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18)`,
      [
        eventId,
        input.workspaceId,
        input.channel,
        input.integrationId,
        input.wabaId ?? null,
        input.source ?? "rule",
        input.eventType,
        input.ctwaClid ?? null,
        input.adId ?? null,
        input.contactInboxId ?? null,
        input.referralEventId ?? null,
        input.currency ?? null,
        input.value ?? null,
        input.orderId ?? null,
        json(input.contents),
        input.occurredAt,
        input.sourceEventId,
        input.capiStatus ?? "pending",
      ],
    )
    return { eventId }
  },

  async createReferralEvent(input) {
    const referralEventId = adsId()
    await pool.query(
      `INSERT INTO "ContactReferralEvent"
         ("id", "workspaceId", "contactId", "contactInboxId", "channel", "type", "provider", "occurredAt",
          "adId", "source", "metadata", "dedupKey")
       VALUES ($1, $2, $3, $4, $5, 'ad', 'meta', $6, $7, 'ADS', $8, $9)`,
      [
        referralEventId,
        input.workspaceId,
        input.contactId,
        input.contactInboxId,
        input.channel,
        input.occurredAt,
        input.adId,
        json(input.adTitle === undefined ? undefined : { adTitle: input.adTitle }),
        input.dedupKey,
      ],
    )
    return { referralEventId }
  },
}

// ---- ads: end ----

// ---- ai: begin ----

const aiFixtures: P102AiFixtures = {}

// ---- ai: end ----

export const create = (): P102Fixtures => ({ ...adsFixtures, ...aiFixtures })
