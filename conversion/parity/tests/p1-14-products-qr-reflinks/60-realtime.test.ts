// realtime.getConnectToken — a short-lived member connect token for the
// workspace realtime socket. It needs a member: a mobile access JWT with
// `x-workspace-id` (route on the mobile member allowlist). A workspace API
// token passes auth but has no member -> 403.

import { describe, expect, it } from "vitest"
import { decodeJwt } from "../../src/normalize.ts"
import { connectRealtime, UpgradeRefused, workspaceSocketUrl } from "../../src/realtime.ts"
import { expectError, keysOf, mobileClient, mobileTokenOrThrow, newTenant } from "./helpers.ts"

describe("realtime.getConnectToken", () => {
  it("a workspace token has no member: 403 FORBIDDEN", async () => {
    const { api } = await newTenant("p114-rt-token")
    expectError(await api.get("/v1/realtime/connect-token"), 403, "FORBIDDEN", {
      message: "A signed-in member session is required",
    })
  })

  it("a member gets a 60-second token, the realtime URL and a socket URL that accepts it", async () => {
    const { tenant } = await newTenant("p114-rt-member")
    const tokens = await mobileTokenOrThrow({ email: tenant.email, password: tenant.password })
    const member = mobileClient(tokens.accessToken, tenant.workspaceId)
    const response = await member.get<{ token: string; url: string; socketUrl: string }>("/v1/realtime/connect-token")
    expect(response.status, response.text).toBe(200)
    expect(keysOf(response.json)).toEqual(["socketUrl", "token", "url"])
    const { token, url, socketUrl } = response.json
    expect(url).toMatch(/^http:\/\/app\//)
    const expectedSocket = new URL(`rt/workspaces/${tenant.workspaceId}`, url)
    expectedSocket.protocol = "ws:"
    expect(socketUrl).toBe(expectedSocket.toString())
    const { payload } = decodeJwt(token)
    expect(payload).toMatchObject({
      aud: `workspace:${tenant.workspaceId}`,
      purpose: "member-connect",
      userId: tenant.userId,
      teamIds: [],
    })
    expect(typeof payload.iatMs).toBe("number")
    expect((payload.exp as number) - (payload.iat as number)).toBe(60)

    // The token opens the workspace socket (through the parity edge).
    const socket = connectRealtime(workspaceSocketUrl(tenant.workspaceId, token))
    try {
      await socket.opened
    } finally {
      await socket.close()
    }
    // A forged token is refused.
    const forged = connectRealtime(workspaceSocketUrl(tenant.workspaceId, `${token.slice(0, -4)}AAAA`))
    await expect(forged.opened).rejects.toBeInstanceOf(UpgradeRefused)
  })

  it("a mobile token without x-workspace-id must pick a workspace; another workspace is refused", async () => {
    const { tenant } = await newTenant("p114-rt-select")
    const { tenant: other } = await newTenant("p114-rt-select-other")
    const tokens = await mobileTokenOrThrow({ email: tenant.email, password: tenant.password })
    expectError(await mobileClient(tokens.accessToken).get("/v1/realtime/connect-token"), 400, "WORKSPACE_SELECTION_REQUIRED")
    expectError(
      await mobileClient(tokens.accessToken, other.workspaceId).get("/v1/realtime/connect-token"),
      403,
      "FORBIDDEN",
    )
  })
})
