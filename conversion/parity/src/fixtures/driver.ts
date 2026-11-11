// Fixture driver: setup that has no public HTTP path (DECISIONS 2026-11-11,
// "P1.1 harness decisions" #1). Each stack writes the same logical rows
// straight to its own database: `stacks/<name>/fixtures.ts` exports
// `createFixtureDriver()`. Setup is never a parity assertion; assertions use
// HTTP, WebSocket, fake upstream records and files only.

export type WorkspaceApiTokenPermission = "full" | "read_only"

export type CreateUserInput = {
  email: string
  password: string
  name: string
  /** Default true: the user can sign in at once. */
  emailVerified?: boolean
}

export type CreateWorkspaceInput = {
  ownerUserId: string
  name: string
  /** IANA zone, default "UTC" (same default as the app's create action). */
  timezone?: string
}

export type CreateWorkspaceTokenInput = {
  workspaceId: string
  name: string
  permission: WorkspaceApiTokenPermission
  /** null = all scopes (unrestricted). */
  scopes: string[] | null
}

export interface FixtureDriver {
  /** User with an email + password credential, in the root tenant. */
  createUser(input: CreateUserInput): Promise<{ userId: string }>
  /**
   * Workspace with the owner as member (role owner, all permissions), the
   * same rows the app's workspace create service writes. Create it before the
   * first HTTP call that lists the owner's workspaces: the app caches that
   * list, and a direct insert cannot clear the cache.
   */
  createWorkspace(input: CreateWorkspaceInput): Promise<{ workspaceId: string }>
  /** Workspace API token; returns the plaintext `cbx_ws_...` once. */
  createWorkspaceToken(input: CreateWorkspaceTokenInput): Promise<{ tokenId: string; token: string }>
  close(): Promise<void>
}

export type FixtureDriverModule = {
  createFixtureDriver: () => FixtureDriver | Promise<FixtureDriver>
}
