import { describe, expect, it } from "vitest"
import { createTenant, signedInClient } from "../../src/fixtures/index.ts"
import { apiClient, HttpClient } from "../../src/http.ts"
import { comparableError } from "../../src/normalize.ts"
import { rpc } from "../../src/rpc.ts"

describe("smoke: fixture driver", () => {
  it("creates a workspace that its owner sees in listMyWorkspacesAPI", async () => {
    const tenant = await createTenant({ label: "list" })
    const http = await signedInClient(tenant)

    const result = await rpc<{ workspaces: Array<{ id: string; name: string }> }>(
      http,
      "workspacesAPI/listMyWorkspacesAPI",
      {},
    )
    expect(result.status).toBe(200)
    expect(result.json.workspaces.map((workspace) => workspace.id)).toEqual([tenant.workspaceId])
    expect(result.json.workspaces[0]).toMatchObject({
      id: tenant.workspaceId,
      name: tenant.workspaceName,
    })
  })

  it("rejects listMyWorkspacesAPI without a session", async () => {
    const result = await rpc(new HttpClient(), "workspacesAPI/listMyWorkspacesAPI", {})
    expect(result.status).toBe(401)
    expect(comparableError(result.json)).toMatchObject({ code: "UNAUTHORIZED", status: 401 })
  })

  it("creates a full workspace API token that GET /api/v1/token reports", async () => {
    const tenant = await createTenant({ label: "token" })
    const response = await apiClient(tenant.token).get("/v1/token")
    expect(response.status).toBe(200)
    expect(response.json).toEqual({
      workspaceId: tenant.workspaceId,
      permission: "full",
      scopes: null,
    })
  })

  it("creates a scoped read-only token", async () => {
    const tenant = await createTenant({
      label: "scoped",
      permission: "read_only",
      scopes: ["contacts"],
    })
    const response = await apiClient(tenant.token).get("/v1/token")
    expect(response.status).toBe(200)
    expect(response.json).toEqual({
      workspaceId: tenant.workspaceId,
      permission: "read_only",
      scopes: ["contacts"],
    })
  })

  it("rejects an unknown token with 401", async () => {
    const response = await apiClient("cbx_ws_parity-unknown-token-0000000000000000000000").get(
      "/v1/token",
    )
    expect(response.status).toBe(401)
    expect(comparableError(response.json)).toEqual({
      defined: true,
      code: "INVALID_CHATBOT_TOKEN",
      status: 401,
    })
  })
})
