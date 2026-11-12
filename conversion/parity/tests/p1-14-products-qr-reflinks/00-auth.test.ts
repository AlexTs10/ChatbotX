// Auth matrix of every P1.14 operation (54): productCategories,
// productVendors, products, qrCodes, questionnaires, realtime, reflinks.
//
// All are workspace-token routes (`workspaceTokenAuthAPIForScope`): no token
// or an unknown token -> 401 INVALID_CHATBOT_TOKEN, a token without the
// route's scope -> 403 FORBIDDEN, a read-only token on a write (every
// non-GET) -> 403 FORBIDDEN, and a token with only the route's scope passes
// auth. Auth runs before input validation, so the requests carry no valid
// input. Ids in paths are numeric ids no row has; a request that passes auth
// answers 4xx or 2xx, never 5xx, and makes no third-party call.
// `realtime.getConnectToken` needs a member (mobile JWT): with a workspace
// token it is a 403 after auth (see 70-realtime), so it is left out of the
// "passes auth" rows.

import { beforeAll, describe, expect, it } from "vitest"
import { createTenant, type Tenant } from "../../src/fixtures/index.ts"
import { apiClient } from "../../src/http.ts"
import { expectError } from "./helpers.ts"

type Scope = "ecommerce" | "automation" | "inbox"
type Method = "GET" | "POST" | "PUT" | "PATCH" | "DELETE"
type Op = { id: string; method: Method; path: string; scope: Scope; body?: unknown }

const op = (id: string, method: Method, path: string, scope: Scope, body?: unknown): Op => ({
  id,
  method,
  path,
  scope,
  body,
})

const ID = "999999999999999994"
const PR = "/v1/products"
const QN = `/v1/questionnaires/${ID}`
const RL = `/v1/ref-links/${ID}`

/** Every operation of P1.14 (public-spec-operations snapshot) with its token scope. */
const OPERATIONS: Op[] = [
  op("productCategories.create", "POST", "/v1/product-categories", "ecommerce"),
  op("productCategories.delete", "DELETE", `/v1/product-categories/${ID}`, "ecommerce"),
  op("productCategories.list", "GET", "/v1/product-categories", "ecommerce"),
  op("productCategories.setVisible", "PATCH", `/v1/product-categories/${ID}/visible`, "ecommerce"),
  op("productCategories.update", "PUT", `/v1/product-categories/${ID}`, "ecommerce"),
  op("products.create", "POST", PR, "ecommerce"),
  op("products.createImportUpload", "POST", `${PR}/imports/upload-url`, "ecommerce"),
  op("products.createMetaCatalog", "POST", `${PR}/meta-catalog`, "ecommerce"),
  op("products.delete", "DELETE", `${PR}/${ID}`, "ecommerce"),
  op("products.deleteMany", "POST", `${PR}/bulk-delete`, "ecommerce"),
  op("products.disconnectMetaCatalog", "DELETE", `${PR}/meta-catalog`, "ecommerce"),
  op("products.get", "GET", `${PR}/${ID}`, "ecommerce"),
  op("products.getImport", "GET", `${PR}/imports/${ID}`, "ecommerce"),
  op("products.getImportTemplate", "GET", `${PR}/import-template`, "ecommerce"),
  op("products.getMetaCatalog", "GET", `${PR}/meta-catalog`, "ecommerce"),
  op("products.list", "GET", PR, "ecommerce"),
  op("products.listImports", "GET", `${PR}/imports`, "ecommerce"),
  op("products.listMetaCatalogBusinesses", "GET", `${PR}/meta-catalog/businesses`, "ecommerce"),
  op("products.listSendable", "GET", `${PR}/sendable`, "ecommerce"),
  op("products.peekImportHeaders", "GET", `${PR}/imports/files/${ID}/headers`, "ecommerce"),
  op("products.searchMetaCatalogItems", "GET", `${PR}/meta-catalog/items`, "ecommerce"),
  op("products.selectMetaCatalog", "POST", `${PR}/meta-catalog/select`, "ecommerce"),
  op("products.setActive", "PATCH", `${PR}/${ID}/active`, "ecommerce"),
  op("products.startImport", "POST", `${PR}/imports`, "ecommerce"),
  op("products.syncMetaCatalog", "POST", `${PR}/meta-catalog/sync`, "ecommerce"),
  op("products.update", "PUT", `${PR}/${ID}`, "ecommerce"),
  op("productVendors.create", "POST", "/v1/product-vendors", "ecommerce"),
  op("productVendors.delete", "DELETE", `/v1/product-vendors/${ID}`, "ecommerce"),
  op("productVendors.list", "GET", "/v1/product-vendors", "ecommerce"),
  // An empty PATCH body would reach the update with no column to set; send one field.
  op("productVendors.update", "PATCH", `/v1/product-vendors/${ID}`, "ecommerce", { name: "parity" }),
  op("qrCodes.create", "POST", "/v1/qr-codes", "automation"),
  op("qrCodes.delete", "DELETE", `/v1/qr-codes/${ID}`, "automation"),
  op("qrCodes.get", "GET", `/v1/qr-codes/${ID}`, "automation"),
  op("qrCodes.list", "GET", "/v1/qr-codes", "automation"),
  op("qrCodes.update", "PATCH", `/v1/qr-codes/${ID}`, "automation", { size: 100 }),
  op("questionnaires.create", "POST", "/v1/questionnaires", "automation"),
  op("questionnaires.delete", "DELETE", QN, "automation"),
  op("questionnaires.deleteSubmission", "DELETE", `${QN}/submissions/${ID}`, "automation"),
  op("questionnaires.duplicate", "POST", `${QN}/duplicate`, "automation"),
  op("questionnaires.get", "GET", QN, "automation"),
  op("questionnaires.getSubmission", "GET", `${QN}/submissions/${ID}`, "automation"),
  op("questionnaires.getSubmissionStats", "GET", `${QN}/submissions/stats`, "automation"),
  op("questionnaires.list", "GET", "/v1/questionnaires", "automation"),
  op("questionnaires.listSubmissions", "GET", `${QN}/submissions`, "automation"),
  op("questionnaires.rename", "PATCH", `${QN}/rename`, "automation"),
  op("questionnaires.update", "PUT", QN, "automation"),
  op("realtime.getConnectToken", "GET", "/v1/realtime/connect-token", "inbox"),
  op("reflinks.create", "POST", "/v1/ref-links", "automation"),
  op("reflinks.delete", "DELETE", RL, "automation"),
  op("reflinks.get", "GET", RL, "automation"),
  op("reflinks.getChatWidget", "GET", `${RL}/chat-widget`, "automation"),
  op("reflinks.list", "GET", "/v1/ref-links", "automation"),
  op("reflinks.update", "PATCH", RL, "automation", { name: "parity" }),
  op("reflinks.updateChatWidget", "PUT", `${RL}/chat-widget`, "automation"),
]

/** Operations whose "passes auth" check applies (all but the member-only route). */
const PASSING = OPERATIONS.filter((o) => o.id !== "realtime.getConnectToken")

const WRITES = OPERATIONS.filter((o) => o.method !== "GET")
const READS = PASSING.filter((o) => o.method === "GET")
const SCOPES: Scope[] = ["ecommerce", "automation", "inbox"]

const call = (token: string, o: { method: string; path: string; body?: unknown }) => {
  const api = apiClient(token)
  const body = o.method === "GET" ? undefined : (o.body ?? {})
  return api.http.request(o.method, `/api${o.path}`, {
    ...(token ? { bearer: token } : {}),
    noCookies: true,
    ...(body ? { json: body } : {}),
  })
}

describe("P1.14 auth matrix", () => {
  it("covers the 54 operations of the unit", () => {
    expect(OPERATIONS).toHaveLength(54)
    expect(new Set(OPERATIONS.map((o) => o.id)).size).toBe(54)
    expect(new Set(OPERATIONS.map((o) => `${o.method} ${o.path}`)).size).toBe(54)
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
        "productCategories.list",
        "products.create",
        "products.listMetaCatalogBusinesses",
        "productVendors.list",
        "qrCodes.create",
        "questionnaires.list",
        "realtime.getConnectToken",
        "reflinks.updateChatWidget",
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
      tenant = await createTenant({ label: "p114-wrong-scope", scopes: ["analytics"] })
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
          await createTenant({ label: `p114-not-${scope}`, scopes: SCOPES.filter((other) => other !== scope) }),
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
        tenants.set(scope, await createTenant({ label: `p114-scope-${scope}`, scopes: [scope] }))
      }
    })
    it.each(PASSING.map((o) => [o.id, o] as const))("%s", async (_id, o) => {
      const response = await call((tenants.get(o.scope) as Tenant).token, o)
      expect([401, 403], response.text).not.toContain(response.status)
      expect(response.status, response.text).toBeLessThan(500)
    })
  })

  describe("read-only token", () => {
    let tenant: Tenant
    beforeAll(async () => {
      tenant = await createTenant({ label: "p114-read-only", permission: "read_only" })
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
    const tenant = await createTenant({ label: "p114-ro-scoped", permission: "read_only", scopes: ["analytics"] })
    const response = await call(tenant.token, { method: "POST", path: "/v1/products" })
    expectError(response, 403, "FORBIDDEN", { message: "Read-only token cannot perform this operation" })
  })
})
