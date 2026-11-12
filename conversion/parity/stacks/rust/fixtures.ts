// Fixture driver of the Rust stack: writes the same rows as
// stacks/original/fixtures.ts, into the server's SQLite database (the
// `rust-data` volume, shared with the runner) with `node:sqlite`.
//
// Same row shapes as the original driver: user + credential account (scrypt
// "salt:key" hash, the better-auth format), workspace + owner member,
// workspace API token (SHA-256 hash, `cbx_ws_` prefix). Ids are snowflakes in
// the app layout (place id 15, like the original fixtures).

import { createHash, randomBytes, scrypt } from "node:crypto"
import { DatabaseSync } from "node:sqlite"
import type {
  CreateUserInput,
  CreateWorkspaceInput,
  CreateWorkspaceTokenInput,
  FixtureDriver,
} from "../../src/fixtures/driver.ts"

const ROOT_TENANT_ID = 1n
const SNOWFLAKE_EPOCH_MS = Date.parse("2004-02-01T00:00:00.000Z")
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
    sequence = BigInt(randomBytes(1)[0] ?? 0)
  }
  return (now << 14n) | (FIXTURE_PLACE_ID << 10n) | sequence
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

class SqliteFixtureDriver implements FixtureDriver {
  private db: DatabaseSync | undefined

  constructor(private readonly path: string) {}

  /** Opened per call batch and closed again, so an idle worker can exit. */
  private open(): DatabaseSync {
    if (!this.db) {
      const db = new DatabaseSync(this.path)
      db.exec("PRAGMA busy_timeout = 10000; PRAGMA foreign_keys = ON;")
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

  async createUser(input: CreateUserInput): Promise<{ userId: string }> {
    const userId = createId()
    const password = await hashPassword(input.password)
    this.transaction((db) => {
      db.prepare(
        `INSERT INTO "User" ("id", "name", "email", "emailVerified", "tenantId") VALUES (?, ?, ?, ?, ?)`,
      ).run(userId, input.name, input.email.toLowerCase(), input.emailVerified === false ? 0 : 1, ROOT_TENANT_ID)
      db.prepare(
        `INSERT INTO "Account" ("id", "accountId", "providerId", "password", "userId", "tenantId")
         VALUES (?, ?, 'credential', ?, ?, ?)`,
      ).run(createId(), userId.toString(), password, userId, ROOT_TENANT_ID)
    })
    return { userId: userId.toString() }
  }

  async createWorkspace(input: CreateWorkspaceInput): Promise<{ workspaceId: string }> {
    const workspaceId = createId()
    const ownerId = BigInt(input.ownerUserId)
    this.transaction((db) => {
      const owner = db.prepare(`SELECT "tenantId" FROM "User" WHERE "id" = ?`).get(ownerId) as
        | { tenantId: number | bigint }
        | undefined
      if (!owner) {
        throw new Error(`createWorkspace: no user ${input.ownerUserId}`)
      }
      db.prepare(
        `INSERT INTO "Workspace" ("id", "name", "timezone", "ownerId", "tenantId") VALUES (?, ?, ?, ?, ?)`,
      ).run(workspaceId, input.name, input.timezone ?? "UTC", ownerId, owner.tenantId)
      db.prepare(
        `INSERT INTO "WorkspaceMember"
           ("id", "workspaceId", "userId", "role", "permissions", "notificationTypes", "notificationChannels")
         VALUES (?, ?, ?, 'owner', ?, ?, ?)`,
      ).run(
        createId(),
        workspaceId,
        ownerId,
        JSON.stringify(OWNER_PERMISSIONS),
        JSON.stringify(DEFAULT_MEMBER_NOTIFICATION_TYPES),
        JSON.stringify(DEFAULT_MEMBER_NOTIFICATION_CHANNELS),
      )
    })
    return { workspaceId: workspaceId.toString() }
  }

  async createWorkspaceToken(input: CreateWorkspaceTokenInput): Promise<{ tokenId: string; token: string }> {
    const tokenId = createId()
    const { token, tokenHash, tokenPrefix } = workspaceToken()
    this.open()
      .prepare(
        `INSERT INTO "WorkspaceApiToken"
           ("id", "workspaceId", "name", "permission", "tokenHash", "tokenPrefix", "isDefault", "scopes")
         VALUES (?, ?, ?, ?, ?, ?, 0, ?)`,
      )
      .run(
        tokenId,
        BigInt(input.workspaceId),
        input.name,
        input.permission,
        tokenHash,
        tokenPrefix,
        input.scopes === null ? null : JSON.stringify(input.scopes),
      )
    return { tokenId: tokenId.toString(), token }
  }

  async close(): Promise<void> {
    this.db?.close()
    this.db = undefined
  }
}

export const createFixtureDriver = (): FixtureDriver =>
  new SqliteFixtureDriver(process.env.PARITY_SQLITE_PATH ?? "/rust-data/chatbotx.db")
