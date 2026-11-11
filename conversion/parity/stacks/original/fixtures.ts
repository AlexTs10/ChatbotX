// Postgres fixture adapter for the original (TypeScript) stack. Writes the
// same rows as the app's own services (see the comments per method), straight
// to the database. Only setup goes through here; assertions stay black-box.
//
// Row sources (baseline 40e733cee):
// - User/Account: better-auth email+password sign-up (packages/auth/src/server.ts),
//   password = better-auth scrypt "<salt hex>:<key hex>" (N=16384, r=16, p=1, dkLen=64).
// - Workspace/WorkspaceMember: workspaceService.create -> writeWorkspace
//   (packages/business/src/workspace/service.ts).
// - WorkspaceApiToken: createWorkspaceTokenAction + generateWorkspaceToken
//   (packages/business/src/workspace-api-token/credentials.ts).

import { createHash, randomBytes, scrypt } from "node:crypto"
import pg from "pg"
import type {
  CreateUserInput,
  CreateWorkspaceInput,
  CreateWorkspaceTokenInput,
  FixtureDriver,
} from "../../src/fixtures/driver.ts"

const ROOT_TENANT_ID = "1"
const SNOWFLAKE_EPOCH_MS = Date.parse("2004-02-01T00:00:00.000Z")
/** App processes use place ids 1-3 (compose); fixtures use 15. */
const FIXTURE_PLACE_ID = 15n

const OWNER_PERMISSIONS = {
  superAdmin: true,
  analytics: true,
  flows: true,
  contacts: true,
  onlyAssignedContacts: true,
  emailAndPhone: true,
  broadcast: true,
  ecommerce: true,
}
const DEFAULT_MEMBER_NOTIFICATION_TYPES = {
  notifyAdmin: true,
  newMessageToHuman: true,
  newOrder: true,
}
const DEFAULT_MEMBER_NOTIFICATION_CHANNELS = {
  messenger: true,
  email: true,
  telegram: true,
  browser: true,
}

let lastMs = 0n
let sequence = 0n
/** Snowflake id in the app layout: ms since 2004-02-01 << 14 | place << 10 | seq. */
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
  return ((now << 14n) | (FIXTURE_PLACE_ID << 10n) | sequence).toString()
}

const hashPassword = (password: string): Promise<string> =>
  new Promise((resolve, reject) => {
    const salt = randomBytes(16).toString("hex")
    scrypt(
      password.normalize("NFKC"),
      salt,
      64,
      { N: 16384, r: 16, p: 1, maxmem: 128 * 16384 * 16 * 2 },
      (error, key) => (error ? reject(error) : resolve(`${salt}:${key.toString("hex")}`)),
    )
  })

const sha256Hex = (value: string) => createHash("sha256").update(value).digest("hex")

const workspaceToken = () => {
  const token = `cbx_ws_${randomBytes(32).toString("base64url")}`
  return { token, tokenHash: sha256Hex(token), tokenPrefix: token.slice(0, 12) }
}

class PostgresFixtureDriver implements FixtureDriver {
  private readonly pool: pg.Pool

  constructor(connectionString: string) {
    this.pool = new pg.Pool({
      connectionString,
      max: 2,
      idleTimeoutMillis: 1_000,
      allowExitOnIdle: true,
    })
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

  async createUser(input: CreateUserInput): Promise<{ userId: string }> {
    const userId = createId()
    const password = await hashPassword(input.password)
    await this.transaction(async (client) => {
      await client.query(
        `INSERT INTO "User" ("id", "name", "email", "emailVerified", "tenantId")
         VALUES ($1, $2, $3, $4, $5)`,
        [userId, input.name, input.email.toLowerCase(), input.emailVerified ?? true, ROOT_TENANT_ID],
      )
      await client.query(
        `INSERT INTO "Account" ("id", "accountId", "providerId", "password", "userId", "tenantId")
         VALUES ($1, $2, 'credential', $3, $4, $5)`,
        [createId(), userId, password, userId, ROOT_TENANT_ID],
      )
    })
    return { userId }
  }

  async createWorkspace(input: CreateWorkspaceInput): Promise<{ workspaceId: string }> {
    const workspaceId = createId()
    await this.transaction(async (client) => {
      const owner = await client.query<{ tenantId: string }>(
        `SELECT "tenantId"::text AS "tenantId" FROM "User" WHERE "id" = $1`,
        [input.ownerUserId],
      )
      const tenantId = owner.rows[0]?.tenantId
      if (!tenantId) {
        throw new Error(`createWorkspace: no user ${input.ownerUserId}`)
      }
      await client.query(
        `INSERT INTO "Workspace" ("id", "name", "timezone", "ownerId", "tenantId")
         VALUES ($1, $2, $3, $4, $5)`,
        [workspaceId, input.name, input.timezone ?? "UTC", input.ownerUserId, tenantId],
      )
      await client.query(
        `INSERT INTO "WorkspaceMember"
           ("id", "workspaceId", "userId", "role", "permissions", "notificationTypes", "notificationChannels")
         VALUES ($1, $2, $3, 'owner', $4, $5, $6)`,
        [
          createId(),
          workspaceId,
          input.ownerUserId,
          JSON.stringify(OWNER_PERMISSIONS),
          JSON.stringify(DEFAULT_MEMBER_NOTIFICATION_TYPES),
          JSON.stringify(DEFAULT_MEMBER_NOTIFICATION_CHANNELS),
        ],
      )
    })
    return { workspaceId }
  }

  async createWorkspaceToken(
    input: CreateWorkspaceTokenInput,
  ): Promise<{ tokenId: string; token: string }> {
    const tokenId = createId()
    const { token, tokenHash, tokenPrefix } = workspaceToken()
    await this.pool.query(
      `INSERT INTO "WorkspaceApiToken"
         ("id", "workspaceId", "name", "permission", "tokenHash", "tokenPrefix", "isDefault", "scopes")
       VALUES ($1, $2, $3, $4, $5, $6, false, $7)`,
      [tokenId, input.workspaceId, input.name, input.permission, tokenHash, tokenPrefix, input.scopes],
    )
    return { tokenId, token }
  }

  async close(): Promise<void> {
    await this.pool.end()
  }
}

export const createFixtureDriver = (): FixtureDriver =>
  new PostgresFixtureDriver(
    process.env.PARITY_PG_URL ?? "postgresql://chatbotx:secretkey@postgres:5432/chatbotx",
  )
