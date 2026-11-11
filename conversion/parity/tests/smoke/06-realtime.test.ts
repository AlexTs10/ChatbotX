import { describe, expect, it } from "vitest"
import { createTenant, signedInClient } from "../../src/fixtures/index.ts"
import { decodeJwt } from "../../src/normalize.ts"
import { connectRealtime, UpgradeRefused, workspaceSocketUrl } from "../../src/realtime.ts"
import { rpc } from "../../src/rpc.ts"

describe("smoke: realtime socket through the edge", () => {
  it("mints a member connect token and opens the workspace socket", async () => {
    const tenant = await createTenant({ label: "rt" })
    const http = await signedInClient(tenant)

    const minted = await rpc<{ token: string }>(
      http,
      "realtimeAPI/mintWorkspaceConnectTokenAuthenticatedAPI",
      { workspaceId: tenant.workspaceId },
      { method: "GET" },
    )
    expect(minted.status).toBe(200)
    const { payload } = decodeJwt(minted.json.token)
    expect(payload).toMatchObject({
      aud: `workspace:${tenant.workspaceId}`,
      purpose: "member-connect",
    })
    expect(Number(payload.exp) - Number(payload.iat)).toBe(60)

    const socket = connectRealtime(workspaceSocketUrl(tenant.workspaceId, minted.json.token))
    await socket.opened
    await socket.close()
    expect((await socket.closed).code).toBe(1000)
  })

  it("refuses the upgrade with a bad token", async () => {
    const tenant = await createTenant({ label: "rt-bad" })
    const socket = connectRealtime(workspaceSocketUrl(tenant.workspaceId, "not-a-jwt"))
    const error = await socket.opened.then(
      () => null,
      (reason: unknown) => reason,
    )
    expect(error).toBeInstanceOf(UpgradeRefused)
    expect((error as UpgradeRefused).status).toBe(401)
  })
})
