// Loads the fixture driver of the stack under test and offers the common
// setup steps on top of it.

import path from "node:path"
import { fileURLToPath } from "node:url"
import { signInOrThrow } from "../auth.ts"
import { config } from "../config.ts"
import { HttpClient } from "../http.ts"
import { uniqueEmail, uniqueId } from "../unique.ts"
import type { FixtureDriver, FixtureDriverModule, WorkspaceApiTokenPermission } from "./driver.ts"

export type { FixtureDriver } from "./driver.ts"

const here = path.dirname(fileURLToPath(import.meta.url))
let driverPromise: Promise<FixtureDriver> | undefined

/**
 * The driver for `STACK`, one per test file. Drivers must not keep the
 * process alive when idle (the worker exits after the file).
 */
export const fixtures = (): Promise<FixtureDriver> => {
  if (!driverPromise) {
    const modulePath = path.resolve(here, "../../stacks", config.stack, "fixtures.ts")
    driverPromise = (import(modulePath) as Promise<FixtureDriverModule>).then((module) =>
      module.createFixtureDriver(),
    )
  }
  return driverPromise
}

export const DEFAULT_PASSWORD = "Parity#12345"

export type Tenant = {
  email: string
  password: string
  userId: string
  workspaceId: string
  workspaceName: string
  /** Full-permission, all-scope workspace API token. */
  token: string
  tokenId: string
}

/** New user + workspace + full API token. One per test that needs a workspace. */
export const createTenant = async (
  options: { label?: string; permission?: WorkspaceApiTokenPermission; scopes?: string[] | null } = {},
): Promise<Tenant> => {
  const driver = await fixtures()
  const label = options.label ?? "tenant"
  const email = uniqueEmail(label)
  const { userId } = await driver.createUser({
    email,
    password: DEFAULT_PASSWORD,
    name: `Parity ${label}`,
  })
  const workspaceName = `Parity WS ${uniqueId()}`
  const { workspaceId } = await driver.createWorkspace({ ownerUserId: userId, name: workspaceName })
  const { token, tokenId } = await driver.createWorkspaceToken({
    workspaceId,
    name: "parity",
    permission: options.permission ?? "full",
    scopes: options.scopes === undefined ? null : options.scopes,
  })
  return { email, password: DEFAULT_PASSWORD, userId, workspaceId, workspaceName, token, tokenId }
}

/** HTTP client with a session cookie of the tenant owner. */
export const signedInClient = async (tenant: Pick<Tenant, "email" | "password">) => {
  const http = new HttpClient()
  await signInOrThrow(http, { email: tenant.email, password: tenant.password })
  return http
}
