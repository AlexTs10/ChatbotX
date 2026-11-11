// aiMcpServers.{list,get,create,update,delete}: CRUD of the workspace's
// remote MCP servers. The public resource never returns auth secrets, and no
// public operation calls the MCP server itself (no outbound HTTP).

import { describe, expect, it } from "vitest"
import { fake } from "../../src/fake.ts"
import { Normalizer } from "../../src/normalize.ts"
import { uniqueHost, uniqueId } from "../../src/unique.ts"
import { type Api, expectError, expectInvalid, tenantApi, uniqueName } from "./helpers.ts"

type McpServer = {
  id: string
  workspaceId: string
  name: string
  url: string
  auth: Record<string, unknown>
  availableTools: Record<string, unknown>
  selectedTools: string[]
  createdAt: string
  updatedAt: string
}

const TOOLS = {
  search: { description: "Search the catalog", inputSchema: { type: "object" } },
  lookup: { description: "Look up an order" },
}

const createServer = (api: Api, overrides: Record<string, unknown> = {}) =>
  api.post<McpServer>("/v1/ai-mcp-servers", {
    name: uniqueName("mcp"),
    url: `https://${uniqueHost("mcp")}/mcp`,
    auth: { type: "none" },
    availableTools: TOOLS,
    selectedTools: ["search"],
    ...overrides,
  })

describe("aiMcpServers.create", () => {
  it("creates a token-auth server and never returns the token", async () => {
    const normalizer = new Normalizer()
    const { tenant, api } = await tenantApi("mcp-create")
    const host = uniqueHost("mcp-token")
    const secret = `sk-mcp-${uniqueId()}`
    const name = uniqueName("Catalog")
    const response = await api.post<McpServer>("/v1/ai-mcp-servers", {
      name: `  ${name}  `,
      url: `https://${host}/mcp`,
      auth: { type: "token", token: `  ${secret}  ` },
      availableTools: TOOLS,
      selectedTools: ["search", "lookup"],
    })
    expect(response.status).toBe(201)
    expect(response.text).not.toContain(secret)
    expect(normalizer.normalize(response.json, { replace: { [tenant.workspaceId]: "<workspace>" } })).toEqual({
      id: "<id:1>",
      workspaceId: "<workspace>",
      name,
      url: `https://${host}/mcp`,
      auth: { type: "token" },
      availableTools: TOOLS,
      selectedTools: ["search", "lookup"],
      createdAt: "<ts>",
      updatedAt: "<ts>",
    })
    // Creating a server does not contact it.
    expect(await fake.requests({ host })).toHaveLength(0)
  })

  it("returns header names but not header values", async () => {
    const { api } = await tenantApi("mcp-header")
    const value = `hv-${uniqueId()}`
    const response = await createServer(api, {
      auth: { type: "header", headers: [{ header: "X-Api-Key", value }, { header: "X-Tenant", value: "acme" }] },
    })
    expect(response.status).toBe(201)
    expect(response.json.auth).toEqual({ type: "header", headers: [{ header: "X-Api-Key" }, { header: "X-Tenant" }] })
    expect(response.text).not.toContain(value)
  })

  it("rejects a name that is already taken in the workspace (422 validation)", async () => {
    const { api } = await tenantApi("mcp-dup")
    const name = uniqueName("dup")
    expect((await createServer(api, { name })).status).toBe(201)
    const second = await createServer(api, { name })
    expectError(second, 422, "validation", { defined: true })
    expect((second.json as { message?: string }).message).toBe("Name is already taken")
  })

  it("allows the same name in another workspace", async () => {
    const name = uniqueName("shared")
    const first = await tenantApi("mcp-ws-a")
    const second = await tenantApi("mcp-ws-b")
    expect((await createServer(first.api, { name })).status).toBe(201)
    expect((await createServer(second.api, { name })).status).toBe(201)
  })

  it.each([
    ["name missing", { name: undefined }, "name"],
    ["name blank after trim", { name: "   " }, "name"],
    ["url not a URL", { url: "not a url" }, "url"],
    ["url missing", { url: undefined }, "url"],
    ["auth type unknown", { auth: { type: "oauth" } }, "auth.type"],
    ["token auth without token", { auth: { type: "token" } }, "auth.token"],
    ["header auth with a blank value", { auth: { type: "header", headers: [{ header: "X", value: " " }] } }, "auth.headers.0.value"],
    ["availableTools a string", { availableTools: "search" }, "availableTools"],
    ["selectedTools not an array", { selectedTools: "search" }, "selectedTools"],
  ])("rejects invalid input: %s (422)", async (_label, overrides, path) => {
    const { api } = await tenantApi("mcp-invalid")
    expectInvalid(await createServer(api, overrides), path)
  })
})

describe("aiMcpServers.get", () => {
  it("returns the stored server", async () => {
    const { api } = await tenantApi("mcp-get")
    const created = await createServer(api, { auth: { type: "token", token: "secret-token" } })
    const response = await api.get<McpServer>(`/v1/ai-mcp-servers/${created.json.id}`)
    expect(response.status).toBe(200)
    expect(response.json).toEqual(created.json)
    expect(response.text).not.toContain("secret-token")
  })

  it("is 404 notFound for an unknown id and for another workspace's server", async () => {
    const owner = await tenantApi("mcp-get-owner")
    const other = await tenantApi("mcp-get-other")
    const created = await createServer(owner.api)
    const foreign = await other.api.get(`/v1/ai-mcp-servers/${created.json.id}`)
    expectError(foreign, 404, "notFound", { defined: true })
    expect((foreign.json as { message?: string }).message).toBe("AI MCP server not found")
    expectError(await owner.api.get("/v1/ai-mcp-servers/1"), 404, "notFound", { defined: true })
  })

  it("rejects a non-numeric id (422)", async () => {
    const { api } = await tenantApi("mcp-get-bad")
    expectInvalid(await api.get("/v1/ai-mcp-servers/abc"), "id")
  })
})

describe("aiMcpServers.list", () => {
  it("lists the workspace's servers newest first with pageCount", async () => {
    const { api } = await tenantApi("mcp-list")
    const empty = await api.get("/v1/ai-mcp-servers")
    expect(empty.status).toBe(200)
    expect(empty.json).toEqual({ data: [], pageCount: 0 })

    const first = await createServer(api, { name: uniqueName("first") })
    const second = await createServer(api, { name: uniqueName("second") })
    const all = await api.get<{ data: McpServer[]; pageCount: number }>("/v1/ai-mcp-servers")
    expect(all.status).toBe(200)
    expect(all.json).toEqual({ data: [second.json, first.json], pageCount: 1 })

    const page2 = await api.get("/v1/ai-mcp-servers", { query: { page: 2, perPage: 1 } })
    expect(page2.json).toEqual({ data: [first.json], pageCount: 2 })
    const page3 = await api.get("/v1/ai-mcp-servers", { query: { page: 3, perPage: 1 } })
    expect(page3.json).toEqual({ data: [], pageCount: 2 })
  })

  it("does not list another workspace's servers", async () => {
    const owner = await tenantApi("mcp-list-owner")
    const other = await tenantApi("mcp-list-other")
    await createServer(owner.api)
    expect((await other.api.get("/v1/ai-mcp-servers")).json).toEqual({ data: [], pageCount: 0 })
  })

  it.each([
    ["perPage above 50", { perPage: 51 }, "perPage"],
    ["perPage 0", { perPage: 0 }, "perPage"],
    ["page 0", { page: 0 }, "page"],
    ["page not a number", { page: "x" }, "page"],
  ])("rejects %s (422)", async (_label, query, path) => {
    const { api } = await tenantApi("mcp-list-bad")
    expectInvalid(await api.get("/v1/ai-mcp-servers", { query }), path)
  })
})

describe("aiMcpServers.update", () => {
  it("replaces the server's settings and keeps the id", async () => {
    const { api } = await tenantApi("mcp-update")
    const created = await createServer(api)
    const host = uniqueHost("mcp-updated")
    const name = uniqueName("renamed")
    const response = await api.put<McpServer>(`/v1/ai-mcp-servers/${created.json.id}`, {
      name,
      url: `https://${host}/v2/mcp`,
      auth: { type: "header", headers: [{ header: "Authorization", value: "Bearer abc" }] },
      availableTools: { only: { description: "Only tool" } },
      selectedTools: ["only"],
    })
    expect(response.status).toBe(200)
    expect(response.text).not.toContain("Bearer abc")
    expect(response.json).toEqual({
      ...created.json,
      name,
      url: `https://${host}/v2/mcp`,
      auth: { type: "header", headers: [{ header: "Authorization" }] },
      availableTools: { only: { description: "Only tool" } },
      selectedTools: ["only"],
      updatedAt: expect.any(String),
    })
    expect(Date.parse(response.json.updatedAt)).toBeGreaterThanOrEqual(Date.parse(created.json.updatedAt))
    expect((await api.get(`/v1/ai-mcp-servers/${created.json.id}`)).json).toEqual(response.json)
    expect(await fake.requests({ host })).toHaveLength(0)
  })

  it("keeps the own name, refuses another server's name (422 validation)", async () => {
    const { api } = await tenantApi("mcp-update-name")
    const a = await createServer(api)
    const b = await createServer(api)
    const body = {
      url: a.json.url,
      auth: { type: "none" },
      availableTools: TOOLS,
      selectedTools: ["search"],
    }
    expect((await api.put(`/v1/ai-mcp-servers/${a.json.id}`, { ...body, name: a.json.name })).status).toBe(200)
    const clash = await api.put(`/v1/ai-mcp-servers/${a.json.id}`, { ...body, name: b.json.name })
    expectError(clash, 422, "validation", { defined: true })
    expect((clash.json as { message?: string }).message).toBe("Name is already taken")
  })

  it("is 404 notFound for an unknown or foreign id", async () => {
    const owner = await tenantApi("mcp-update-owner")
    const other = await tenantApi("mcp-update-other")
    const created = await createServer(owner.api)
    const body = { name: uniqueName("x"), url: "https://example.parity.test/mcp", auth: { type: "none" }, availableTools: {}, selectedTools: [] }
    expectError(await other.api.put(`/v1/ai-mcp-servers/${created.json.id}`, body), 404, "notFound", { defined: true })
    expectError(await owner.api.put("/v1/ai-mcp-servers/1", body), 404, "notFound", { defined: true })
    // The foreign update changed nothing.
    expect((await owner.api.get(`/v1/ai-mcp-servers/${created.json.id}`)).json).toEqual(created.json)
  })

  it("requires the full body (422)", async () => {
    const { api } = await tenantApi("mcp-update-bad")
    const created = await createServer(api)
    expectInvalid(await api.put(`/v1/ai-mcp-servers/${created.json.id}`, { name: "only-name" }), "url")
  })
})

describe("aiMcpServers.delete", () => {
  it("deletes the server (204) and a second delete is 404", async () => {
    const { api } = await tenantApi("mcp-delete")
    const created = await createServer(api)
    const deleted = await api.delete(`/v1/ai-mcp-servers/${created.json.id}`)
    expect(deleted.status).toBe(204)
    expect(deleted.text).toBe("")
    expectError(await api.get(`/v1/ai-mcp-servers/${created.json.id}`), 404, "notFound")
    expectError(await api.delete(`/v1/ai-mcp-servers/${created.json.id}`), 404, "notFound", { defined: true })
    expect((await api.get("/v1/ai-mcp-servers")).json).toEqual({ data: [], pageCount: 0 })
  })

  it("cannot delete another workspace's server", async () => {
    const owner = await tenantApi("mcp-delete-owner")
    const other = await tenantApi("mcp-delete-other")
    const created = await createServer(owner.api)
    expectError(await other.api.delete(`/v1/ai-mcp-servers/${created.json.id}`), 404, "notFound")
    expect((await owner.api.get(`/v1/ai-mcp-servers/${created.json.id}`)).status).toBe(200)
  })

  it("rejects a non-numeric id (422)", async () => {
    const { api } = await tenantApi("mcp-delete-bad")
    expectInvalid(await api.delete("/v1/ai-mcp-servers/abc"), "id")
  })
})
