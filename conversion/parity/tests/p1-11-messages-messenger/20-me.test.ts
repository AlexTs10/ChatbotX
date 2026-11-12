// me.get / listWorkspaces / registerDeviceToken / unregisterDeviceToken /
// changePassword: the user-level surface of the native app. Only a mobile
// access JWT (`POST /api/auth/mobile/token`, password grant) is accepted.
// A user whose `mustChangePassword` flag is set can only read `me.get` and
// call `me.changePassword`; every other route answers 403 "Password change
// required". changePassword verifies the current password, sets the new one,
// clears the flag, ends every other sign-in (web sessions, refresh tokens)
// and returns a fresh token pair.
//
// A mobile JWT with `x-workspace-id` also authenticates as a member on the
// allowlisted workspace routes of this unit (messages, media upload); other
// routes answer 403 to it.
//
// Device tokens have no read route; registering one is checked through its
// response only (no push is triggered here, the fake upstream has no Expo host).

import { describe, expect, it } from "vitest"
import { signInOrThrow } from "../../src/auth.ts"
import { DEFAULT_PASSWORD, type Tenant } from "../../src/fixtures/index.ts"
import { HttpClient } from "../../src/http.ts"
import { decodeJwt } from "../../src/normalize.ts"
import {
  expectError,
  expectInvalid,
  ext,
  keysOf,
  mobileClient,
  mobileToken,
  mobileTokenOrThrow,
  newTenant,
  uniqueName,
  webchatConversation,
} from "./helpers.ts"

const TOKEN_KEYS = [
  "accessToken",
  "deviceId",
  "expiresAt",
  "expiresIn",
  "mustChangePassword",
  "refreshToken",
  "refreshTokenExpiresAt",
  "tokenType",
]

/** get-session that skips the 5-minute session cookie cache (reads the session table). */
const freshSession = (web: HttpClient) =>
  web.get<{ user?: { id: string } } | null>("/api/auth/get-session", { query: { disableCookieCache: "true" } })

const credentials = (tenant: Tenant) => ({ email: tenant.email, password: tenant.password })

describe("me.get", () => {
  it("returns the signed-in user for a mobile access token", async () => {
    const { tenant } = await newTenant("p111-me-get")
    const tokens = await mobileTokenOrThrow({ ...credentials(tenant), deviceId: "parity-device-1" })
    expect(keysOf(tokens)).toEqual(TOKEN_KEYS)
    expect(tokens).toMatchObject({ tokenType: "Bearer", deviceId: "parity-device-1", mustChangePassword: false })
    const { payload } = decodeJwt(tokens.accessToken)
    expect(payload.sub).toBe(tenant.userId)

    const me = await mobileClient(tokens.accessToken).get("/v1/me")
    expect(me.status, me.text).toBe(200)
    expect(me.json).toEqual({
      id: tenant.userId,
      name: "Parity p111-me-get",
      email: tenant.email,
      image: null,
      mustChangePassword: false,
    })
  })
})

describe("me.listWorkspaces", () => {
  it("lists the user's workspaces (no token field)", async () => {
    const { tenant } = await newTenant("p111-me-ws")
    const tokens = await mobileTokenOrThrow(credentials(tenant))
    const response = await mobileClient(tokens.accessToken).get<{ workspaces: Array<Record<string, unknown>> }>(
      "/v1/me/workspaces",
    )
    expect(response.status, response.text).toBe(200)
    expect(keysOf(response.json)).toEqual(["workspaces"])
    expect(response.json.workspaces).toHaveLength(1)
    expect(response.json.workspaces[0]).toMatchObject({
      id: tenant.workspaceId,
      name: tenant.workspaceName,
      ownerId: tenant.userId,
      timezone: "UTC",
    })
    expect(Object.keys(response.json.workspaces[0] ?? {})).not.toContain("token")
  })
})

describe("me.registerDeviceToken / unregisterDeviceToken", () => {
  it("registers for the account or a workspace the user belongs to; unregister ignores unknown tokens", async () => {
    const { tenant } = await newTenant("p111-me-device")
    const tokens = await mobileTokenOrThrow(credentials(tenant))
    const api = mobileClient(tokens.accessToken)
    const push = `ExponentPushToken[${uniqueName("parity")}]`

    const account = await api.put("/v1/me/device-tokens", { platform: "ios", token: push })
    expect(account.status, account.text).toBe(200)
    expect(account.json).toEqual({ success: true })
    const bound = await api.put("/v1/me/device-tokens", { platform: "android", token: push }, {
      headers: { "x-workspace-id": tenant.workspaceId },
    })
    expect(bound.status, bound.text).toBe(200)
    expect(bound.json).toEqual({ success: true })

    const removed = await api.delete("/v1/me/device-tokens", { json: { token: push } })
    expect(removed.status, removed.text).toBe(200)
    expect(removed.json).toEqual({ success: true })
    const again = await api.delete("/v1/me/device-tokens", { json: { token: push } })
    expect(again.json).toEqual({ success: true })
  })

  it("refuses a workspace the user is not a member of (403) and a bad header (400); validates the body", async () => {
    const { tenant } = await newTenant("p111-me-device-err")
    const { tenant: other } = await newTenant("p111-me-device-err-o")
    const api = mobileClient((await mobileTokenOrThrow(credentials(tenant))).accessToken)
    const push = `ExponentPushToken[${uniqueName("parity")}]`
    expectError(
      await api.put("/v1/me/device-tokens", { platform: "ios", token: push }, { headers: { "x-workspace-id": other.workspaceId } }),
      403,
      "forbidden",
      { message: "Not a member of this workspace" },
    )
    expectError(
      await api.put("/v1/me/device-tokens", { platform: "ios", token: push }, { headers: { "x-workspace-id": "abc" } }),
      400,
      "BAD_REQUEST",
      { message: "x-workspace-id must be a numeric workspace identifier" },
    )
    expectInvalid(await api.put("/v1/me/device-tokens", { platform: "web", token: push }), "platform")
    expectInvalid(await api.put("/v1/me/device-tokens", { platform: "ios", token: "" }), "token")
    expectInvalid(await api.delete("/v1/me/device-tokens", { json: {} }), "token")
  })
})

describe("me.changePassword", () => {
  it("sets the new password, returns a fresh token pair and ends every other sign-in", async () => {
    const { tenant } = await newTenant("p111-me-pw")
    const web = new HttpClient()
    await signInOrThrow(web, credentials(tenant))
    expect((await freshSession(web)).json?.user?.id).toBe(tenant.userId)
    const first = await mobileTokenOrThrow({ ...credentials(tenant), deviceId: "parity-old" })

    const newPassword = "Parity#New-67890"
    const response = await mobileClient(first.accessToken).post("/v1/me/password", {
      currentPassword: tenant.password,
      newPassword,
      deviceId: "parity-new",
    })
    expect(response.status, response.text).toBe(200)
    expect(keysOf(response.json)).toEqual(TOKEN_KEYS)
    expect(response.json).toMatchObject({ tokenType: "Bearer", deviceId: "parity-new", mustChangePassword: false })
    expect(response.headers.get("cache-control")).toBe("no-store")
    const fresh = response.json as { accessToken: string; refreshToken: string }

    // The fresh pair works; the old refresh token and the web session do not.
    expect((await mobileClient(fresh.accessToken).get("/v1/me")).status).toBe(200)
    const reuse = await new HttpClient().post("/api/auth/mobile/token", {
      grantType: "refresh_token",
      refreshToken: first.refreshToken,
    })
    expect(reuse.status, reuse.text).toBe(401)
    expect((await freshSession(web)).json).toBeNull()

    // Only the new password signs in.
    expect((await mobileToken(credentials(tenant))).status).toBe(401)
    expect((await mobileToken({ email: tenant.email, password: newPassword })).status).toBe(200)
  })

  it("a wrong current password, a too-short or unchanged new password are 400 changePasswordFailed", async () => {
    const { tenant } = await newTenant("p111-me-pw-err")
    const api = mobileClient((await mobileTokenOrThrow(credentials(tenant))).accessToken)
    expectError(
      await api.post("/v1/me/password", { currentPassword: "Wrong#12345", newPassword: "Parity#Other-1" }),
      400,
      "changePasswordFailed",
      { message: "Invalid password" },
    )
    expectError(
      await api.post("/v1/me/password", { currentPassword: tenant.password, newPassword: tenant.password }),
      400,
      "changePasswordFailed",
      { message: "New password must be different from the current one" },
    )
    expectInvalid(await api.post("/v1/me/password", { currentPassword: tenant.password, newPassword: "short" }), "newPassword")
    expectInvalid(await api.post("/v1/me/password", { newPassword: "Parity#Other-1" }), "currentPassword")
    expectInvalid(
      await api.post("/v1/me/password", { currentPassword: tenant.password, newPassword: "x".repeat(101) }),
      "newPassword",
    )
    // Still the old password.
    expect((await api.get("/v1/me")).status).toBe(200)
  })

  it("a provisioned temporary password: only me.get and changePassword answer until it is changed", async () => {
    const { tenant } = await newTenant("p111-me-must")
    await (await ext()).setMustChangePassword({ userId: tenant.userId, value: true })
    const tokens = await mobileTokenOrThrow(credentials(tenant))
    expect(tokens.mustChangePassword).toBe(true)
    const api = mobileClient(tokens.accessToken)
    expect((await api.get("/v1/me")).json).toMatchObject({ id: tenant.userId, mustChangePassword: true })
    expectError(await api.get("/v1/me/workspaces"), 403, "FORBIDDEN", { message: "Password change required" })
    expectError(
      await api.put("/v1/me/device-tokens", { platform: "ios", token: `ExponentPushToken[${uniqueName("x")}]` }),
      403,
      "FORBIDDEN",
      { message: "Password change required" },
    )
    expectError(
      await mobileClient(tokens.accessToken, tenant.workspaceId).post("/v1/media-library/files/upload-url", {
        fileName: "a.png",
        mimeType: "image/png",
      }),
      403,
      "FORBIDDEN",
      { message: "Password change required" },
    )

    const changed = await api.post<{ accessToken: string; mustChangePassword: boolean }>("/v1/me/password", {
      currentPassword: tenant.password,
      newPassword: "Parity#Changed-1",
    })
    expect(changed.status, changed.text).toBe(200)
    expect(changed.json.mustChangePassword).toBe(false)
    const after = mobileClient(changed.json.accessToken)
    expect((await after.get("/v1/me")).json).toMatchObject({ mustChangePassword: false })
    expect((await after.get("/v1/me/workspaces")).status).toBe(200)
  })
})

describe("a mobile token on workspace routes of this unit", () => {
  it("acts as the member on allowlisted routes; other routes and a missing header are refused", async () => {
    const { tenant, api } = await newTenant("p111-me-member")
    const conversation = await webchatConversation(tenant, api)
    const tokens = await mobileTokenOrThrow(credentials(tenant))
    const member = mobileClient(tokens.accessToken, tenant.workspaceId)

    // messages.create as a member is attributed to the user.
    const sent = await member.post<{ senderType: string; senderId: string | null; text: string }>(
      `/v1/conversations/${conversation.conversationId}/messages`,
      { text: "From the phone" },
    )
    expect(sent.status, sent.text).toBe(201)
    expect(sent.json).toMatchObject({ text: "From the phone", senderType: "user", senderId: tenant.userId })

    const upload = await member.post("/v1/media-library/files/upload-url", { fileName: "a.png", mimeType: "image/png" })
    expect(upload.status, upload.text).toBe(201)

    expectError(await member.get("/v1/media-library/files"), 403, "FORBIDDEN", {
      message: "This operation is not available to mobile sessions",
    })
    expectError(await member.get("/v1/magic-links"), 403, "FORBIDDEN", {
      message: "This operation is not available to mobile sessions",
    })
    expectError(
      await mobileClient(tokens.accessToken).get(`/v1/conversations/${conversation.conversationId}/messages`),
      400,
      "WORKSPACE_SELECTION_REQUIRED",
      { message: "A numeric x-workspace-id header is required for this request" },
    )
    const { tenant: other } = await newTenant("p111-me-member-o")
    expectError(
      await mobileClient(tokens.accessToken, other.workspaceId).get(
        `/v1/conversations/${conversation.conversationId}/messages`,
      ),
      403,
      "FORBIDDEN",
      { message: "Account has no access to this workspace" },
    )
    expect(DEFAULT_PASSWORD).toBe(tenant.password)
  })
})
