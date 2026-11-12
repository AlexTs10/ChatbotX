// Auth matrix of every P1.11 operation (52): magicLinks, me, mediaLibrary,
// messages, messengerChannels, messengerPersonas, messengerTemplates.
//
// Workspace-token routes (`workspaceTokenAuthAPIForScope`, 47 operations): no
// token or an unknown token -> 401 INVALID_CHATBOT_TOKEN, a token without the
// route's scope -> 403 FORBIDDEN, a read-only token on a write (every non-GET)
// -> 403 FORBIDDEN, and a token with only the route's scope passes auth. Auth
// runs before input validation, so the requests carry no valid input. Ids in
// paths are numeric ids no row has; a request that passes auth answers 4xx or
// 2xx, never 5xx.
//
// `me` routes (5) take only a mobile access JWT: no token, a workspace API
// token or a forged JWT -> 401 INVALID_CHATBOT_TOKEN "Invalid or missing
// access token". They have no scope and no read-only check (see 20-me).

import { beforeAll, describe, expect, it } from "vitest"
import { createTenant, type Tenant } from "../../src/fixtures/index.ts"
import { apiClient } from "../../src/http.ts"
import { expectError } from "./helpers.ts"

type Scope = "automation" | "media" | "inbox" | "channels" | "broadcasts"
type Method = "GET" | "POST" | "PUT" | "PATCH" | "DELETE"
type Op = { id: string; method: Method; path: string; scope: Scope }

const op = (id: string, method: Method, path: string, scope: Scope): Op => ({ id, method, path, scope })

const ID = "999999999999999991"
const MSG = `/v1/conversations/${ID}/messages/${ID}`
const MC = `/v1/messenger-channels/${ID}`

/** Every workspace-token operation of P1.11 (public-spec-operations snapshot) with its token scope. */
const OPERATIONS: Op[] = [
  op("magicLinks.create", "POST", "/v1/magic-links", "automation"),
  op("magicLinks.delete", "DELETE", `/v1/magic-links/${ID}`, "automation"),
  op("magicLinks.get", "GET", `/v1/magic-links/${ID}`, "automation"),
  op("magicLinks.list", "GET", "/v1/magic-links", "automation"),
  op("magicLinks.update", "PATCH", `/v1/magic-links/${ID}`, "automation"),
  op("mediaLibrary.createFile", "POST", "/v1/media-library/files", "media"),
  op("mediaLibrary.createFolder", "POST", "/v1/media-library/folders", "media"),
  op("mediaLibrary.createUploadUrl", "POST", "/v1/media-library/files/upload-url", "media"),
  op("mediaLibrary.deleteFile", "DELETE", `/v1/media-library/files/${ID}`, "media"),
  op("mediaLibrary.deleteFolder", "DELETE", `/v1/media-library/folders/${ID}`, "media"),
  op("mediaLibrary.getFile", "GET", `/v1/media-library/files/${ID}`, "media"),
  op("mediaLibrary.listFiles", "GET", "/v1/media-library/files", "media"),
  op("mediaLibrary.listFolders", "GET", "/v1/media-library/folders", "media"),
  op("mediaLibrary.moveFiles", "PATCH", "/v1/media-library/files/move", "media"),
  op("mediaLibrary.recordAccess", "POST", `/v1/media-library/files/${ID}/access`, "media"),
  op("mediaLibrary.renameFolder", "PUT", `/v1/media-library/folders/${ID}`, "media"),
  op("mediaLibrary.setFavourite", "PUT", `/v1/media-library/files/${ID}/favourite`, "media"),
  op("messages.changeAttributes", "POST", `${MSG}/attributes`, "inbox"),
  op("messages.create", "POST", `/v1/conversations/${ID}/messages`, "inbox"),
  op("messages.delete", "DELETE", MSG, "inbox"),
  op("messages.get", "GET", MSG, "inbox"),
  op("messages.getFlowResponse", "GET", `${MSG}/flow-response`, "inbox"),
  op("messages.list", "GET", `/v1/conversations/${ID}/messages`, "inbox"),
  op("messages.requestWhatsappCallPermission", "POST", `/v1/conversations/${ID}/whatsapp-call-permission`, "inbox"),
  op("messages.sendWhatsappTemplate", "POST", `/v1/conversations/${ID}/whatsapp-template`, "inbox"),
  op("messages.update", "PUT", MSG, "inbox"),
  op("messengerChannels.disconnect", "DELETE", MC, "channels"),
  op("messengerChannels.disconnectCapi", "DELETE", `${MC}/capi`, "channels"),
  op("messengerChannels.get", "GET", MC, "channels"),
  op("messengerChannels.getCoexistStatus", "GET", `${MC}/coexist`, "channels"),
  op("messengerChannels.getSettings", "GET", `${MC}/settings`, "channels"),
  op("messengerChannels.list", "GET", "/v1/messenger-channels", "channels"),
  op("messengerChannels.patchSettings", "PATCH", `${MC}/settings`, "channels"),
  op("messengerChannels.sendCapiTestEvent", "POST", `${MC}/capi/test-event`, "channels"),
  op("messengerChannels.setCapiDataset", "PUT", `${MC}/capi/dataset`, "channels"),
  op("messengerChannels.setCapiTestEventCode", "PUT", `${MC}/capi/test-event-code`, "channels"),
  op("messengerChannels.setCoexist", "PUT", `${MC}/coexist`, "channels"),
  op("messengerChannels.updateHandoverResumeFlow", "PATCH", `${MC}/handover-resume-flow`, "channels"),
  op("messengerChannels.updateSettings", "PUT", `${MC}/settings`, "channels"),
  op("messengerChannels.updateTagSync", "PATCH", `${MC}/tag-sync`, "channels"),
  op("messengerPersonas.list", "GET", "/v1/messenger-personas", "channels"),
  op("messengerTemplates.clone", "POST", `/v1/messenger/templates/${ID}/clone`, "broadcasts"),
  op("messengerTemplates.create", "POST", `${MC}/templates`, "broadcasts"),
  op("messengerTemplates.delete", "DELETE", `/v1/messenger/templates/${ID}`, "broadcasts"),
  op("messengerTemplates.get", "GET", `/v1/messenger/templates/${ID}`, "broadcasts"),
  op("messengerTemplates.list", "GET", "/v1/messenger/templates", "broadcasts"),
  op("messengerTemplates.sync", "POST", `${MC}/templates/sync`, "broadcasts"),
]

/** The `me` operations: mobile access JWT only. */
const ME_OPERATIONS: Array<{ id: string; method: Method; path: string }> = [
  { id: "me.changePassword", method: "POST", path: "/v1/me/password" },
  { id: "me.get", method: "GET", path: "/v1/me" },
  { id: "me.listWorkspaces", method: "GET", path: "/v1/me/workspaces" },
  { id: "me.registerDeviceToken", method: "PUT", path: "/v1/me/device-tokens" },
  { id: "me.unregisterDeviceToken", method: "DELETE", path: "/v1/me/device-tokens" },
]

const WRITES = OPERATIONS.filter((o) => o.method !== "GET")
const READS = OPERATIONS.filter((o) => o.method === "GET")
const SCOPES: Scope[] = ["automation", "media", "inbox", "channels", "broadcasts"]

const call = (token: string, o: { method: string; path: string }) => {
  const api = apiClient(token)
  const body = o.method === "GET" ? undefined : {}
  return api.http.request(o.method, `/api${o.path}`, {
    ...(token ? { bearer: token } : {}),
    noCookies: true,
    ...(body ? { json: body } : {}),
  })
}

/** A JWT-shaped bearer no issuer signed. */
const FORGED_JWT = [
  Buffer.from(JSON.stringify({ alg: "EdDSA", kid: "parity", typ: "JWT" })).toString("base64url"),
  Buffer.from(JSON.stringify({ sub: "1", aud: "http://app/api", iss: "http://app", exp: 4102444800 })).toString(
    "base64url",
  ),
  Buffer.from("not-a-signature").toString("base64url"),
].join(".")

describe("P1.11 auth matrix", () => {
  it("covers the 52 operations of the unit", () => {
    const all = [...OPERATIONS, ...ME_OPERATIONS]
    expect(all).toHaveLength(52)
    expect(new Set(all.map((o) => o.id)).size).toBe(52)
    expect(new Set(all.map((o) => `${o.method} ${o.path}`)).size).toBe(52)
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
        "magicLinks.list",
        "magicLinks.create",
        "mediaLibrary.listFiles",
        "mediaLibrary.createFolder",
        "messages.list",
        "messages.create",
        "messengerChannels.list",
        "messengerChannels.setCapiDataset",
        "messengerPersonas.list",
        "messengerTemplates.list",
        "messengerTemplates.sync",
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
      tenant = await createTenant({ label: "p111-wrong-scope", scopes: ["analytics"] })
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
          await createTenant({ label: `p111-not-${scope}`, scopes: SCOPES.filter((other) => other !== scope) }),
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
        tenants.set(scope, await createTenant({ label: `p111-scope-${scope}`, scopes: [scope] }))
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
      tenant = await createTenant({ label: "p111-read-only", permission: "read_only" })
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
    const tenant = await createTenant({ label: "p111-ro-scoped", permission: "read_only", scopes: ["analytics"] })
    const response = await call(tenant.token, { method: "POST", path: "/v1/media-library/folders" })
    expectError(response, 403, "FORBIDDEN", { message: "Read-only token cannot perform this operation" })
  })

  describe("me routes take only a mobile access token", () => {
    let tenant: Tenant
    beforeAll(async () => {
      tenant = await createTenant({ label: "p111-me-auth" })
    })
    it.each(ME_OPERATIONS.map((o) => [o.id, o] as const))("%s: none / workspace token / forged JWT -> 401", async (_id, o) => {
      for (const token of ["", tenant.token, FORGED_JWT]) {
        expectError(await call(token, o), 401, "INVALID_CHATBOT_TOKEN", {
          defined: true,
          message: "Invalid or missing access token",
        })
      }
    })
  })
})
