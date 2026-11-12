// Auth matrix of every P1.13 operation (51): pipelines, placeCategories,
// placeLocations, polls.
//
// All are workspace-token routes (`workspaceTokenAuthAPIForScope`): no token
// or an unknown token -> 401 INVALID_CHATBOT_TOKEN, a token without the
// route's scope -> 403 FORBIDDEN, a read-only token on a write (every
// non-GET) -> 403 FORBIDDEN, and a token with only the route's scope passes
// auth. Auth runs before input validation, so the requests carry no valid
// input. Ids in paths are numeric ids no row has; a request that passes auth
// answers 4xx or 2xx, never 5xx.

import { beforeAll, describe, expect, it } from "vitest"
import { createTenant, type Tenant } from "../../src/fixtures/index.ts"
import { apiClient } from "../../src/http.ts"
import { expectError } from "./helpers.ts"

type Scope = "pipelines" | "places" | "polls"
type Method = "GET" | "POST" | "PUT" | "PATCH" | "DELETE"
type Op = { id: string; method: Method; path: string; scope: Scope; body?: unknown }

const op = (id: string, method: Method, path: string, scope: Scope, body?: unknown): Op => ({
  id,
  method,
  path,
  scope,
  body,
})

const ID = "999999999999999993"
const PL = `/v1/pipelines/${ID}`
const IT = `${PL}/items/${ID}`

/** Every operation of P1.13 (public-spec-operations snapshot) with its token scope. */
const OPERATIONS: Op[] = [
  op("pipelines.addAttachment", "POST", `${IT}/attachments`, "pipelines"),
  op("pipelines.addComment", "POST", `${IT}/comments`, "pipelines"),
  op("pipelines.create", "POST", "/v1/pipelines", "pipelines"),
  op("pipelines.createField", "POST", `${PL}/fields`, "pipelines"),
  op("pipelines.createItem", "POST", `${PL}/items`, "pipelines"),
  op("pipelines.createStage", "POST", `${PL}/stages`, "pipelines"),
  op("pipelines.delete", "DELETE", PL, "pipelines"),
  op("pipelines.deleteComment", "DELETE", `${IT}/comments/${ID}`, "pipelines"),
  op("pipelines.deleteField", "DELETE", `${PL}/fields/${ID}`, "pipelines"),
  op("pipelines.deleteItem", "DELETE", IT, "pipelines"),
  op("pipelines.deleteStage", "DELETE", `${PL}/stages/${ID}`, "pipelines"),
  op("pipelines.duplicate", "POST", `${PL}/duplicate`, "pipelines"),
  op("pipelines.get", "GET", PL, "pipelines"),
  op("pipelines.getItem", "GET", IT, "pipelines"),
  op("pipelines.getStats", "GET", `${PL}/stats`, "pipelines"),
  op("pipelines.list", "GET", "/v1/pipelines", "pipelines"),
  op("pipelines.listActivity", "GET", `${IT}/activity`, "pipelines"),
  op("pipelines.listComments", "GET", `${IT}/comments`, "pipelines"),
  op("pipelines.listContactItems", "GET", `/v1/contacts/id:${ID}/pipeline-items`, "pipelines"),
  op("pipelines.listFields", "GET", `${PL}/fields`, "pipelines"),
  op("pipelines.listItems", "GET", `${PL}/items`, "pipelines"),
  op("pipelines.listMembers", "GET", `${PL}/members`, "pipelines"),
  op("pipelines.moveItem", "POST", `${IT}/move`, "pipelines"),
  op("pipelines.removeAttachment", "DELETE", `${IT}/attachments/${ID}`, "pipelines"),
  op("pipelines.removeMember", "DELETE", `${PL}/members/${ID}`, "pipelines"),
  op("pipelines.reorderStages", "POST", `${PL}/stages/reorder`, "pipelines"),
  op("pipelines.setMember", "PUT", `${PL}/members/${ID}`, "pipelines"),
  op("pipelines.stageTotals", "GET", `${PL}/stage-totals`, "pipelines"),
  op("pipelines.transferItem", "POST", `${IT}/transfer`, "pipelines"),
  op("pipelines.update", "PATCH", PL, "pipelines", { name: "parity" }),
  op("pipelines.updateField", "PATCH", `${PL}/fields/${ID}`, "pipelines", { name: "parity" }),
  op("pipelines.updateItem", "PATCH", IT, "pipelines", { title: "parity" }),
  op("pipelines.updateStage", "PATCH", `${PL}/stages/${ID}`, "pipelines", { name: "parity" }),
  op("placeCategories.create", "POST", "/v1/place-categories", "places"),
  op("placeCategories.delete", "DELETE", `/v1/place-categories/${ID}`, "places"),
  op("placeCategories.get", "GET", `/v1/place-categories/${ID}`, "places"),
  op("placeCategories.list", "GET", "/v1/place-categories", "places"),
  op("placeCategories.update", "PATCH", `/v1/place-categories/${ID}`, "places"),
  op("placeLocations.create", "POST", "/v1/place-locations", "places"),
  op("placeLocations.delete", "DELETE", `/v1/place-locations/${ID}`, "places"),
  op("placeLocations.get", "GET", `/v1/place-locations/${ID}`, "places"),
  op("placeLocations.list", "GET", "/v1/place-locations", "places"),
  op("placeLocations.update", "PATCH", `/v1/place-locations/${ID}`, "places"),
  op("polls.bulkDelete", "POST", "/v1/polls/bulk-delete", "polls"),
  op("polls.create", "POST", "/v1/polls", "polls"),
  op("polls.delete", "DELETE", `/v1/polls/${ID}`, "polls"),
  op("polls.get", "GET", `/v1/polls/${ID}`, "polls"),
  op("polls.getAnalytics", "GET", `/v1/polls/${ID}/analytics`, "polls"),
  op("polls.getLinks", "GET", `/v1/polls/${ID}/links`, "polls"),
  op("polls.list", "GET", "/v1/polls", "polls"),
  op("polls.update", "PATCH", `/v1/polls/${ID}`, "polls"),
]

const WRITES = OPERATIONS.filter((o) => o.method !== "GET")
const READS = OPERATIONS.filter((o) => o.method === "GET")
const SCOPES: Scope[] = ["pipelines", "places", "polls"]

const call = (token: string, o: { method: string; path: string; body?: unknown }) => {
  const api = apiClient(token)
  const body = o.method === "GET" ? undefined : (o.body ?? {})
  return api.http.request(o.method, `/api${o.path}`, {
    ...(token ? { bearer: token } : {}),
    noCookies: true,
    ...(body ? { json: body } : {}),
  })
}

describe("P1.13 auth matrix", () => {
  it("covers the 51 operations of the unit", () => {
    expect(OPERATIONS).toHaveLength(51)
    expect(new Set(OPERATIONS.map((o) => o.id)).size).toBe(51)
    expect(new Set(OPERATIONS.map((o) => `${o.method} ${o.path}`)).size).toBe(51)
  })

  describe("no token -> 401 INVALID_CHATBOT_TOKEN", () => {
    it.each(OPERATIONS.map((o) => [o.id, o] as const))("%s", async (_id, o) => {
      expectError(await call("", o), 401, "INVALID_CHATBOT_TOKEN", {
        defined: true,
        message: "Invalid or missing workspace API token",
      })
    })
  })

  describe("unknown workspace token -> 401 INVALID_CHATBOT_TOKEN", () => {
    it.each(
      [
        "pipelines.list",
        "pipelines.create",
        "pipelines.createItem",
        "pipelines.listContactItems",
        "placeCategories.list",
        "placeLocations.create",
        "polls.list",
        "polls.update",
      ].map((id) => [id, OPERATIONS.find((o) => o.id === id) as Op] as const),
    )("%s", async (_id, o) => {
      expectError(await call("cbx_ws_parityUnknownTokenValue000000000000000", o), 401, "INVALID_CHATBOT_TOKEN", {
        defined: true,
        message: "Invalid or missing workspace API token",
      })
    })
  })

  describe("token without the operation's scope -> 403 FORBIDDEN", () => {
    let tenant: Tenant
    beforeAll(async () => {
      // `analytics` is none of the scopes this unit's operations need.
      tenant = await createTenant({ label: "p113-wrong-scope", scopes: ["analytics"] })
    })
    it.each(OPERATIONS.map((o) => [o.id, o] as const))("%s", async (_id, o) => {
      expectError(await call(tenant.token, o), 403, "FORBIDDEN", {
        defined: true,
        message: `Token is not authorized for the '${o.scope}' scope`,
      })
    })
  })

  describe("a token with all other scopes but this one -> 403 FORBIDDEN", () => {
    const tenants = new Map<Scope, Tenant>()
    beforeAll(async () => {
      for (const scope of SCOPES) {
        tenants.set(
          scope,
          await createTenant({ label: `p113-not-${scope}`, scopes: SCOPES.filter((other) => other !== scope) }),
        )
      }
    })
    it.each(OPERATIONS.map((o) => [o.id, o] as const))("%s", async (_id, o) => {
      expectError(await call((tenants.get(o.scope) as Tenant).token, o), 403, "FORBIDDEN", { defined: true })
    })
  })

  describe("a token with only the operation's scope passes auth", () => {
    const tenants = new Map<Scope, Tenant>()
    beforeAll(async () => {
      for (const scope of SCOPES) {
        tenants.set(scope, await createTenant({ label: `p113-scope-${scope}`, scopes: [scope] }))
      }
    })
    it.each(OPERATIONS.map((o) => [o.id, o] as const))("%s", async (_id, o) => {
      const response = await call((tenants.get(o.scope) as Tenant).token, o)
      expect([401, 403], response.text).not.toContain(response.status)
      expect(response.status, response.text).toBeLessThan(500)
    })
  })

  describe("read-only token", () => {
    let tenant: Tenant
    beforeAll(async () => {
      tenant = await createTenant({ label: "p113-read-only", permission: "read_only" })
    })
    it.each(WRITES.map((o) => [o.id, o] as const))("write %s -> 403 FORBIDDEN", async (_id, o) => {
      expectError(await call(tenant.token, o), 403, "FORBIDDEN", {
        defined: true,
        message: "Read-only token cannot perform this operation",
      })
    })
    it.each(READS.map((o) => [o.id, o] as const))("read %s passes auth", async (_id, o) => {
      const response = await call(tenant.token, o)
      expect([401, 403], response.text).not.toContain(response.status)
      expect(response.status, response.text).toBeLessThan(500)
    })
  })

  it("the read-only check runs before the scope check", async () => {
    const tenant = await createTenant({ label: "p113-ro-scoped", permission: "read_only", scopes: ["analytics"] })
    const response = await call(tenant.token, { method: "POST", path: "/v1/pipelines" })
    expectError(response, 403, "FORBIDDEN", { message: "Read-only token cannot perform this operation" })
  })
})
