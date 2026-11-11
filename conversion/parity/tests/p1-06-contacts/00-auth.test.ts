// Auth matrix of every P1.6 operation (58): no token, unknown token, a token
// without the operation's scope, a token with only that scope, and a
// read-only token (writes refused, reads let through). Auth runs before input
// validation, so the requests carry no valid input.

import { beforeAll, describe, expect, it } from "vitest"
import { createTenant, type Tenant } from "../../src/fixtures/index.ts"
import { apiClient } from "../../src/http.ts"
import { expectError } from "./helpers.ts"

type Scope = "contacts" | "inbox" | "automation"
type Method = "GET" | "POST" | "PUT" | "PATCH" | "DELETE"
type Op = { id: string; method: Method; path: string; scope: Scope }

const C = "/v1/contacts"
const I = `${C}/id%3A1`
const op = (id: string, method: Method, path: string, scope: Scope = "contacts"): Op => ({ id, method, path, scope })

export const OPERATIONS: Op[] = [
  op("contacts.addTags", "POST", `${I}/tags`),
  op("contacts.addTagsByName", "POST", `${I}/tags/by-name`),
  op("contacts.applyCustomFieldOperations", "PATCH", `${I}/custom-fields`),
  op("contacts.block", "POST", `${I}/block`),
  op("contacts.bulkAddTags", "POST", `${C}/bulk/tags`),
  op("contacts.bulkDelete", "POST", `${C}/bulk/delete`),
  op("contacts.bulkRemoveTags", "POST", `${C}/bulk/tags/remove`),
  op("contacts.bulkSubscribeSequences", "POST", `${C}/bulk/sequences`),
  op("contacts.bulkTagByStats", "POST", `${C}/bulk/tags/by-stats`),
  op("contacts.bulkUnsubscribeSequences", "POST", `${C}/bulk/sequences/remove`),
  op("contacts.clearCustomField", "DELETE", `${I}/custom-fields/score`),
  op("contacts.clearCustomFields", "DELETE", `${I}/custom-fields`),
  op("contacts.count", "GET", `${C}/count`),
  op("contacts.create", "POST", C),
  op("contacts.createImportUpload", "POST", `${C}/imports/upload-url`),
  op("contacts.createNote", "POST", `${I}/notes`),
  op("contacts.delete", "DELETE", I),
  op("contacts.deleteNote", "DELETE", `${I}/notes/1`),
  op("contacts.export", "POST", `${C}/export`),
  op("contacts.findByCustomField", "GET", `${C}/find-by-custom-field`),
  op("contacts.get", "GET", I),
  op("contacts.getCustomField", "GET", `${I}/custom-fields/score`),
  op("contacts.getExportFile", "GET", `${C}/export-files/1`),
  op("contacts.getImport", "GET", `${C}/imports/1`),
  op("contacts.getImportTemplate", "GET", `${C}/import-template`),
  op("contacts.getMessage", "GET", `${I}/messages/1`, "inbox"),
  op("contacts.import", "POST", `${C}/import`),
  op("contacts.list", "GET", C),
  op("contacts.listAdReferrals", "GET", `${I}/ad-referrals`),
  op("contacts.listCustomFields", "GET", `${I}/custom-fields`),
  op("contacts.listExportFiles", "GET", `${C}/export-files`),
  op("contacts.listFilterFields", "GET", `${C}/filter-fields`),
  op("contacts.listImports", "GET", `${C}/imports`),
  op("contacts.listInboxes", "GET", `${I}/inboxes`),
  op("contacts.listMessages", "GET", `${I}/messages`, "inbox"),
  op("contacts.listNotes", "GET", `${I}/notes`),
  op("contacts.listSequences", "GET", `${I}/sequences`),
  op("contacts.listTags", "GET", `${I}/tags`),
  op("contacts.peekImportHeaders", "GET", `${C}/imports/files/1/headers`),
  op("contacts.refreshProfile", "POST", `${I}/refresh-profile`),
  op("contacts.removeTags", "DELETE", `${I}/tags`),
  op("contacts.removeTagsByName", "DELETE", `${I}/tags/by-name`),
  op("contacts.resolveFilterValueLabels", "POST", `${C}/filter-value-labels`),
  op("contacts.search", "POST", `${C}/search`),
  op("contacts.sendFlow", "POST", `${I}/flows`, "automation"),
  op("contacts.sendMessage", "POST", `${I}/messages`, "inbox"),
  op("contacts.setCustomField", "PUT", `${I}/custom-fields/score`),
  op("contacts.setCustomFieldLegacy", "POST", `${I}/custom-fields/1`),
  op("contacts.setSequences", "PUT", `${I}/sequences`),
  op("contacts.setTags", "PUT", `${I}/tags`),
  op("contacts.subscribeSequences", "POST", `${I}/sequences`),
  op("contacts.triggerAutoReply", "POST", `${I}/auto-replies`, "automation"),
  op("contacts.unblock", "POST", `${I}/unblock`),
  op("contacts.unsubscribeSequences", "DELETE", `${I}/sequences`),
  op("contacts.update", "PATCH", I),
  op("contacts.updateLegacy", "PUT", I),
  op("contacts.updateNote", "PUT", `${I}/notes/1`),
  op("contacts.upsert", "POST", `${I}/upsert`),
]

// A read-only token may GET, plus the POST-for-read allowlist; of this unit
// only `resolveFilterValueLabels` is on it (`contacts.search` is not).
const READ_ONLY_ALLOWED = (o: Op) => o.method === "GET" || o.id === "contacts.resolveFilterValueLabels"
const WRITES = OPERATIONS.filter((o) => !READ_ONLY_ALLOWED(o))
const READS = OPERATIONS.filter(READ_ONLY_ALLOWED)

const call = (token: string, o: Op) => {
  const api = apiClient(token)
  const body = o.method === "GET" ? undefined : {}
  return api.http.request(o.method, `/api${o.path}`, {
    ...(token ? { bearer: token } : {}),
    noCookies: true,
    ...(body ? { json: body } : {}),
  })
}

describe("P1.6 auth matrix", () => {
  it("covers the 58 operations of the unit", () => {
    expect(OPERATIONS).toHaveLength(58)
    expect(new Set(OPERATIONS.map((o) => o.id)).size).toBe(58)
    expect(new Set(OPERATIONS.map((o) => `${o.method} ${o.path}`)).size).toBe(58)
    expect(OPERATIONS.filter((o) => o.method === "GET")).toHaveLength(20)
    expect(READS).toHaveLength(21)
    expect(WRITES).toHaveLength(37)
    expect(OPERATIONS.filter((o) => o.scope === "inbox").map((o) => o.id)).toEqual([
      "contacts.getMessage",
      "contacts.listMessages",
      "contacts.sendMessage",
    ])
    expect(OPERATIONS.filter((o) => o.scope === "automation").map((o) => o.id)).toEqual([
      "contacts.sendFlow",
      "contacts.triggerAutoReply",
    ])
  })

  describe("no token -> 401 INVALID_CHATBOT_TOKEN", () => {
    it.each(OPERATIONS.map((o) => [o.id, o] as const))("%s", async (_id, o) => {
      const response = await call("", o)
      expectError(response, 401, "INVALID_CHATBOT_TOKEN", {
        defined: true,
        message: "Invalid or missing workspace API token",
      })
    })
  })

  describe("unknown workspace token -> 401 INVALID_CHATBOT_TOKEN", () => {
    it.each(
      ["contacts.list", "contacts.create", "contacts.sendMessage", "contacts.sendFlow", "contacts.import"].map(
        (id) => [id, OPERATIONS.find((o) => o.id === id) as Op] as const,
      ),
    )("%s", async (_id, o) => {
      const response = await call("cbx_ws_parityUnknownTokenValue000000000000000", o)
      expectError(response, 401, "INVALID_CHATBOT_TOKEN", { defined: true })
    })
  })

  describe("token without the operation's scope -> 403 FORBIDDEN", () => {
    let tenant: Tenant
    beforeAll(async () => {
      // `analytics` is none of the three scopes this unit's operations need.
      tenant = await createTenant({ label: "p106-wrong-scope", scopes: ["analytics"] })
    })
    it.each(OPERATIONS.map((o) => [o.id, o] as const))("%s", async (_id, o) => {
      const response = await call(tenant.token, o)
      expectError(response, 403, "FORBIDDEN", {
        defined: true,
        message: `Token is not authorized for the '${o.scope}' scope`,
      })
    })
  })

  describe("a `contacts` token does not open the inbox / automation routes", () => {
    let tenant: Tenant
    beforeAll(async () => {
      tenant = await createTenant({ label: "p106-contacts-scope", scopes: ["contacts"] })
    })
    it.each(OPERATIONS.filter((o) => o.scope !== "contacts").map((o) => [o.id, o] as const))("%s", async (_id, o) => {
      const response = await call(tenant.token, o)
      expectError(response, 403, "FORBIDDEN", {
        message: `Token is not authorized for the '${o.scope}' scope`,
      })
    })
  })

  describe("token with only the operation's scope passes auth", () => {
    const tenants = new Map<Scope, Tenant>()
    beforeAll(async () => {
      for (const scope of ["contacts", "inbox", "automation"] as const) {
        tenants.set(scope, await createTenant({ label: `p106-scope-${scope}`, scopes: [scope] }))
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
      tenant = await createTenant({ label: "p106-read-only", permission: "read_only" })
    })
    it.each(WRITES.map((o) => [o.id, o] as const))("write %s -> 403 FORBIDDEN", async (_id, o) => {
      const response = await call(tenant.token, o)
      expectError(response, 403, "FORBIDDEN", {
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
})
