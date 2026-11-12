// Postgres implementation of the P1.12–P1.14 fixture extension (original
// stack). Writes the same logical rows the app writes; setup only, assertions
// stay black-box. Row sources (baseline 40e733cee), copied in part from the
// P1.11 fixture (units do not share files):
// - Inbox + IntegrationWhatsapp + Connection: the WhatsApp embedded signup
//   (schema/integration-whatsapp.ts), auth metadata of
//   packages/business/src/integration-whatsapp/auth-schema.ts.
// - MiniAppSubmission: miniAppSubmissionService.create
//   (packages/business/src/mini-app/submission-service.ts): source "web",
//   sanitized answers, and `MiniApp.submissionsCount + 1` in one transaction.
// - MinigameContact / MinigamePlay: the public minigame play path
//   (packages/business/src/minigame/minigame-contact-service.ts).
// - PollVote: the chat vote path (packages/business/src/poll/vote.service.ts),
//   one vote per contact and poll.
// - Integration + IntegrationMetaCatalog: integrationMetaCatalogService.upsert
//   (packages/business/src/meta-catalog/integration.service.ts), auth =
//   encryptUtils.encryptObject({accessToken, version}) without AAD, the
//   format of packages/encryption/src/encryption.ts ({v:1, kid, iv, text,
//   tag}, AES-256-GCM, key = ENCRYPTION_KEY of stacks/original/original.env);
//   copied from the P1.2 fixture.
// - QuestionnaireSubmission + QuestionnaireAnswer: questionnaireSubmissionService
//   startOrResume / answerCurrent (packages/business/src/questionnaire/submission-service.ts).

import { createCipheriv, randomBytes } from "node:crypto"
import { readFileSync } from "node:fs"
import path from "node:path"
import { fileURLToPath } from "node:url"
import pg from "pg"
import type {
  P112Fixtures,
  P112MiniAppSubmissionInput,
  P112MinigamePlayerInput,
  P112MinigamePlayInput,
  P112MetaCatalogConnectionInput,
  P112PollVoteInput,
  P112QuestionnaireSubmissionInput,
  P112WhatsappChannel,
  P112WhatsappChannelInput,
} from "../../../src/fixtures/ext/p1-12.ts"

const SNOWFLAKE_EPOCH_MS = Date.parse("2004-02-01T00:00:00.000Z")
/** Snowflake place id of the P1.12–P1.14 batch (DECISIONS "P2.7 Redis replacement"). */
const PLACE_ID = 4n

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

const here = path.dirname(fileURLToPath(import.meta.url))

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

/** encryptUtils.encryptObject of the app (no AAD). */
const encryptObject = (value: unknown) => {
  if (ENCRYPTION_KEY.length !== 32) {
    throw new Error("p1-12 fixtures: ENCRYPTION_KEY must be 32 bytes of hex")
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

const oauth2Auth = (accessToken: string, metadata: Record<string, unknown>, redirectUrl: string) => ({
  authType: "oauth2",
  clientId: "parity-client",
  clientSecret: "parity-secret",
  redirectUrl,
  version: "v23.0",
  tokens: { accessToken },
  metadata,
})

class PostgresP112Fixtures implements P112Fixtures {
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

  async createWhatsappChannel(input: P112WhatsappChannelInput): Promise<P112WhatsappChannel> {
    const accessToken = `wa-token-${input.phoneNumberId}`
    const wabaId = `waba-${input.phoneNumberId}`
    const businessId = `biz-${input.phoneNumberId}`
    const displayPhoneNumber = "+1 555 0100"
    return await this.transaction(async (client) => {
      const inboxId = createId()
      await client.query(
        `INSERT INTO "Inbox" ("id", "name", "channel", "sourceId", "workspaceId", "status")
         VALUES ($1, $2, 'whatsapp', $3, $4, 'connected')`,
        [inboxId, input.name, input.phoneNumberId, input.workspaceId],
      )
      const integrationId = createId()
      const auth = oauth2Auth(
        accessToken,
        {
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
        },
        "http://app/integrations/callback",
      )
      await client.query(
        `INSERT INTO "IntegrationWhatsapp"
           ("id", "auth", "phoneNumberId", "wabaId", "businessId", "name", "displayPhoneNumber", "workspaceId", "inboxId")
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)`,
        [
          integrationId,
          JSON.stringify(auth),
          input.phoneNumberId,
          wabaId,
          businessId,
          input.name,
          displayPhoneNumber,
          input.workspaceId,
          inboxId,
        ],
      )
      await client.query(
        `INSERT INTO "Connection"
           ("id", "workspaceId", "provider", "kind", "channel", "inboxId", "sourceId", "displayName", "status", "connectedAt")
         VALUES ($1, $2, 'whatsapp', 'channel', 'whatsapp', $3, $4, $5, 'connected', now())`,
        [createId(), input.workspaceId, inboxId, input.phoneNumberId, input.name],
      )
      return { integrationId, inboxId, accessToken, wabaId }
    })
  }

  async createMiniAppSubmission(
    input: P112MiniAppSubmissionInput,
  ): Promise<{ submissionId: string; createdAt: string }> {
    const submissionId = createId()
    const createdAt = input.createdAt ?? new Date().toISOString()
    await this.transaction(async (client) => {
      await client.query(
        `INSERT INTO "MiniAppSubmission"
           ("id", "createdAt", "updatedAt", "workspaceId", "miniAppId", "contactId", "source", "answers")
         VALUES ($1, $2, $2, $3, $4, $5, 'web', $6)`,
        [
          submissionId,
          createdAt,
          input.workspaceId,
          input.miniAppId,
          input.contactId ?? null,
          JSON.stringify(input.answers),
        ],
      )
      await client.query(
        `UPDATE "MiniApp" SET "submissionsCount" = "submissionsCount" + 1 WHERE "id" = $1`,
        [input.miniAppId],
      )
    })
    return { submissionId, createdAt }
  }

  async createMinigamePlayer(input: P112MinigamePlayerInput): Promise<{ playerId: string }> {
    const playerId = createId()
    const now = new Date().toISOString()
    await this.pool.query(
      `INSERT INTO "MinigameContact"
         ("id", "createdAt", "updatedAt", "minigameId", "contactId", "contactInboxId", "openedAt", "played",
          "remaining", "sharesCount")
       VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)`,
      [
        playerId,
        input.openedAt ?? now,
        input.lastPlayedAt ?? now,
        input.minigameId,
        input.contactId,
        input.contactInboxId ?? null,
        input.openedAt ?? now,
        input.played,
        input.remaining,
        input.sharesCount ?? 0,
      ],
    )
    return { playerId }
  }

  async createMinigamePlay(input: P112MinigamePlayInput): Promise<{ playId: string; createdAt: string }> {
    const playId = createId()
    const createdAt = input.createdAt ?? new Date().toISOString()
    await this.pool.query(
      `INSERT INTO "MinigamePlay"
         ("id", "createdAt", "updatedAt", "minigameId", "contactId", "isWinning", "prizeId", "prizeName")
       VALUES ($1, $2, $2, $3, $4, $5, $6, $7)`,
      [
        playId,
        createdAt,
        input.minigameId,
        input.contactId,
        input.isWinning,
        input.prizeId ?? null,
        input.prizeName ?? null,
      ],
    )
    return { playId, createdAt }
  }

  async createPollVote(input: P112PollVoteInput): Promise<{ voteId: string }> {
    const voteId = createId()
    await this.pool.query(
      `INSERT INTO "PollVote" ("id", "workspaceId", "pollId", "candidateId", "contactId", "contactInboxId")
       VALUES ($1, $2, $3, $4, $5, $6)`,
      [voteId, input.workspaceId, input.pollId, input.candidateId, input.contactId, input.contactInboxId ?? null],
    )
    return { voteId }
  }

  async createMetaCatalogConnection(
    input: P112MetaCatalogConnectionInput,
  ): Promise<{ connectionId: string; integrationId: string }> {
    const integrationId = createId()
    const connectionId = createId()
    await this.transaction(async (client) => {
      await client.query(
        `INSERT INTO "Integration" ("id", "workspaceId", "integrationType") VALUES ($1, $2, 'metaCatalog')`,
        [integrationId, input.workspaceId],
      )
      await client.query(
        `INSERT INTO "IntegrationMetaCatalog"
           ("id", "workspaceId", "integrationId", "catalogId", "catalogName", "businessId", "encryptedAuth",
            "authMode", "tokenExpiresAt", "status")
         VALUES ($1, $2, $3, $4, $5, $6, $7, 'oauth', NULL, 'active')`,
        [
          connectionId,
          input.workspaceId,
          integrationId,
          input.catalogId ?? null,
          input.catalogName ?? null,
          input.businessId ?? null,
          JSON.stringify(encryptObject({ accessToken: input.accessToken, version: "v23.0" })),
        ],
      )
    })
    return { connectionId, integrationId }
  }

  async createQuestionnaireSubmission(input: P112QuestionnaireSubmissionInput): Promise<{ submissionId: string }> {
    const submissionId = createId()
    const createdAt = input.createdAt ?? new Date().toISOString()
    const completedAt =
      input.completedAt !== undefined ? input.completedAt : input.status === "completed" ? createdAt : null
    await this.transaction(async (client) => {
      await client.query(
        `INSERT INTO "QuestionnaireSubmission"
           ("id", "createdAt", "updatedAt", "workspaceId", "questionnaireId", "contactId", "conversationId", "status",
            "totalPoints", "startedAt", "completedAt")
         VALUES ($1, $2, $2, $3, $4, $5, $6, $7, $8, $2, $9)`,
        [
          submissionId,
          createdAt,
          input.workspaceId,
          input.questionnaireId,
          input.contactId,
          input.conversationId ?? null,
          input.status,
          input.totalPoints ?? null,
          completedAt,
        ],
      )
      const base = Date.parse(createdAt)
      for (const [index, answer] of (input.answers ?? []).entries()) {
        const answeredAt = new Date(base + index + 1).toISOString()
        await client.query(
          `INSERT INTO "QuestionnaireAnswer"
             ("id", "createdAt", "updatedAt", "submissionId", "questionId", "questionIdSnapshot", "questionTitleSnapshot",
              "questionTypeSnapshot", "labelSnapshot", "value", "pointsEarned", "answeredAt")
           VALUES ($1, $2, $2, $3, $4, $5, $6, $7, $8, $9, $10, $2)`,
          [
            createId(),
            answeredAt,
            submissionId,
            answer.questionId,
            answer.questionId,
            answer.questionTitle,
            answer.questionType,
            answer.label,
            JSON.stringify(answer.value),
            answer.pointsEarned ?? null,
          ],
        )
      }
    })
    return { submissionId }
  }

  async close(): Promise<void> {
    await this.pool.end()
  }
}

export const create = (): P112Fixtures =>
  new PostgresP112Fixtures(process.env.PARITY_PG_URL ?? "postgresql://chatbotx:secretkey@postgres:5432/chatbotx")
