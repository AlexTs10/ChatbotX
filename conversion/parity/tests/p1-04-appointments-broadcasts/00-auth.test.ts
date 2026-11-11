// Auth for every P1.4 operation: no token -> 401, a token without the
// operation's scope -> 403, a read-only token on a write -> 403, and a
// read-only token on a read passes auth. The auth middlewares run before input
// validation, so placeholder ids are enough here.

import { beforeAll, describe, expect, it } from "vitest"
import type { Tenant } from "../../src/fixtures/index.ts"
import { apiClient } from "../../src/http.ts"
import { comparableError } from "../../src/normalize.ts"
import { extraToken, newTenant } from "./helpers.ts"

type Op = {
  id: string
  method: "GET" | "POST" | "PUT" | "PATCH" | "DELETE"
  path: string
  scope: string
}

const op = (id: string, method: Op["method"], path: string, scope: string): Op => ({ id, method, path, scope })

/** Every operation in scope of P1.4 (public-spec-operations snapshot). */
export const P104_OPERATIONS: Op[] = [
  op("appointmentCalendars.create", "POST", "/v1/appointment-calendars", "appointments"),
  op("appointmentCalendars.delete", "DELETE", "/v1/appointment-calendars/1", "appointments"),
  op("appointmentCalendars.duplicate", "POST", "/v1/appointment-calendars/1/duplicate", "appointments"),
  op("appointmentCalendars.get", "GET", "/v1/appointment-calendars/1", "appointments"),
  op("appointmentCalendars.getAvailability", "GET", "/v1/appointment-calendars/1/availability", "appointments"),
  op("appointmentCalendars.list", "GET", "/v1/appointment-calendars", "appointments"),
  op("appointmentCalendars.setActive", "PATCH", "/v1/appointment-calendars/1/active", "appointments"),
  op("appointmentCalendars.update", "PUT", "/v1/appointment-calendars/1", "appointments"),
  op("appointmentExternalCalendars.disconnect", "DELETE", "/v1/appointment-external-calendars/1", "appointments"),
  op("appointmentExternalCalendars.list", "GET", "/v1/appointment-external-calendars", "appointments"),
  op("appointmentReminders.list", "GET", "/v1/appointment-reminders", "appointments"),
  op("appointments.book", "POST", "/v1/appointments", "appointments"),
  op("appointments.cancel", "POST", "/v1/appointments/1/cancel", "appointments"),
  op("appointments.createBookingLink", "POST", "/v1/conversations/1/appointment-booking-link", "appointments"),
  op("appointments.delete", "DELETE", "/v1/appointments/1", "appointments"),
  op("appointments.get", "GET", "/v1/appointments/1", "appointments"),
  op("appointments.list", "GET", "/v1/appointments", "appointments"),
  op("botFields.applyOperation", "POST", "/v1/bot-fields/x/operations", "automation"),
  op("botFields.bulkUpdate", "PUT", "/v1/bot-fields/bulk-update", "automation"),
  op("botFields.create", "POST", "/v1/bot-fields", "automation"),
  op("botFields.delete", "DELETE", "/v1/bot-fields/x", "automation"),
  op("botFields.get", "GET", "/v1/bot-fields/x", "automation"),
  op("botFields.list", "GET", "/v1/bot-fields", "automation"),
  op("botFields.reset", "POST", "/v1/bot-fields/x/reset", "automation"),
  op("botFields.resetMany", "POST", "/v1/bot-fields/bulk-reset", "automation"),
  op("botFields.set", "PUT", "/v1/bot-fields/x", "automation"),
  op("botFields.setMany", "PUT", "/v1/bot-fields", "automation"),
  op("botFields.update", "PATCH", "/v1/bot-fields/x", "automation"),
  op("botSimulator.getLink", "GET", "/v1/bot-simulator/link", "channels"),
  op("broadcasts.clone", "POST", "/v1/broadcasts/1/clone", "broadcasts"),
  op("broadcasts.create", "POST", "/v1/broadcasts", "broadcasts"),
  op("broadcasts.delete", "DELETE", "/v1/broadcasts/1", "broadcasts"),
  op("broadcasts.duplicate", "POST", "/v1/broadcasts/1/duplicate", "broadcasts"),
  op("broadcasts.get", "GET", "/v1/broadcasts/x", "broadcasts"),
  op("broadcasts.getAudience", "GET", "/v1/broadcasts/x/audience", "broadcasts"),
  op("broadcasts.list", "GET", "/v1/broadcasts", "broadcasts"),
  op("broadcasts.listContacts", "GET", "/v1/broadcasts/1/contacts", "broadcasts"),
  op("broadcasts.listOccurrences", "GET", "/v1/broadcasts/x/occurrences", "broadcasts"),
  op("broadcasts.listRunGroups", "GET", "/v1/broadcasts/x/run-groups", "broadcasts"),
  op("broadcasts.moveToDraft", "POST", "/v1/broadcasts/1/move-to-draft", "broadcasts"),
  op("broadcasts.previewAudience", "POST", "/v1/broadcasts/audience/preview", "broadcasts"),
  op("broadcasts.previewRecurrence", "POST", "/v1/broadcasts/preview-recurrence", "broadcasts"),
  op("broadcasts.resend", "POST", "/v1/broadcasts/1/resend", "broadcasts"),
  op("broadcasts.restoreOccurrence", "POST", "/v1/broadcasts/1/restore-occurrence", "broadcasts"),
  op("broadcasts.resume", "POST", "/v1/broadcasts/1/resume", "broadcasts"),
  op("broadcasts.schedule", "POST", "/v1/broadcasts/1/schedule", "broadcasts"),
  op("broadcasts.skipOccurrence", "POST", "/v1/broadcasts/1/skip-occurrence", "broadcasts"),
  op("broadcasts.stop", "POST", "/v1/broadcasts/1/stop", "broadcasts"),
  op("broadcasts.update", "PUT", "/v1/broadcasts/1", "broadcasts"),
  op("broadcasts.updateDraft", "PUT", "/v1/broadcasts/1/draft", "broadcasts"),
  op("capabilities.get", "GET", "/v1/capabilities", "contacts"),
  op("carts.get", "GET", "/v1/carts/id:1", "ecommerce"),
  op("channelIntegrations.list", "GET", "/v1/channel-integrations", "channels"),
  op("channelPosts.list", "GET", "/v1/channel-posts", "contacts"),
  op("channelPosts.listSelected", "GET", "/v1/channel-posts/options/by-ids", "contacts"),
]

const call = (token: string, entry: Op) => {
  const api = apiClient(token)
  switch (entry.method) {
    case "GET":
      return api.get(entry.path)
    case "DELETE":
      return api.delete(entry.path)
    case "POST":
      return api.post(entry.path, {})
    case "PUT":
      return api.put(entry.path, {})
    case "PATCH":
      return api.patch(entry.path, {})
  }
}

describe("P1.4 auth: every operation", () => {
  // One workspace per token kind: the API allows 120 calls per 10 s per
  // workspace, and each table below makes about 56.
  let tenant: Tenant
  let readOnlyToken: string
  let wrongScopeToken: string

  beforeAll(async () => {
    ;({ tenant } = await newTenant("p104-auth"))
    const { tenant: readOnlyTenant } = await newTenant("p104-auth-ro")
    readOnlyToken = await extraToken(readOnlyTenant, { permission: "read_only" })
    const { tenant: scopedTenant } = await newTenant("p104-auth-scope")
    // `minigames` is not the scope of any P1.4 operation.
    wrongScopeToken = await extraToken(scopedTenant, { scopes: ["minigames"] })
  })

  it("lists 55 operations", () => {
    expect(P104_OPERATIONS).toHaveLength(55)
    expect(new Set(P104_OPERATIONS.map((entry) => entry.id)).size).toBe(55)
  })

  it.each(P104_OPERATIONS)("$id without a token is 401 INVALID_CHATBOT_TOKEN", async (entry) => {
    const response = await call("", entry)
    expect(response.status).toBe(401)
    expect(comparableError(response.json, { keepMessage: true })).toEqual({
      defined: true,
      code: "INVALID_CHATBOT_TOKEN",
      status: 401,
      message: "Invalid or missing workspace API token",
    })
  })

  it.each(P104_OPERATIONS)("$id with an unknown token is 401", async (entry) => {
    const response = await call("cbx_ws_parity-unknown-token-000000000000000000000000", entry)
    expect(response.status).toBe(401)
    expect((response.json as { code?: string }).code).toBe("INVALID_CHATBOT_TOKEN")
  })

  it.each(P104_OPERATIONS)("$id with a token without its scope is 403", async (entry) => {
    const response = await call(wrongScopeToken, entry)
    expect(response.status).toBe(403)
    expect(comparableError(response.json, { keepMessage: true })).toEqual({
      defined: true,
      code: "FORBIDDEN",
      status: 403,
      message: `Token is not authorized for the '${entry.scope}' scope`,
    })
  })

  const writes = P104_OPERATIONS.filter((entry) => entry.method !== "GET")
  const reads = P104_OPERATIONS.filter((entry) => entry.method === "GET")

  it.each(writes)("$id with a read-only token is 403", async (entry) => {
    const response = await call(readOnlyToken, entry)
    expect(response.status).toBe(403)
    expect(comparableError(response.json, { keepMessage: true })).toEqual({
      defined: true,
      code: "FORBIDDEN",
      status: 403,
      message: "Read-only token cannot perform this operation",
    })
  })

  it.each(reads)("$id with a read-only token passes auth", async (entry) => {
    const response = await call(readOnlyToken, entry)
    expect([401, 403]).not.toContain(response.status)
  })

  it.each([
    ["appointments", "/v1/appointment-calendars"],
    ["automation", "/v1/bot-fields"],
    ["broadcasts", "/v1/broadcasts"],
    ["contacts", "/v1/channel-posts"],
    ["channels", "/v1/channel-integrations"],
  ])("a token with only the %s scope may read %s", async (scope, path) => {
    const token = await extraToken(tenant, { scopes: [scope] })
    const response = await apiClient(token).get(path)
    expect(response.status, response.text).toBe(200)
  })

  it("accepts the deprecated ?token= query parameter", async () => {
    const response = await apiClient("").get("/v1/appointment-calendars", {
      query: { token: tenant.token },
    })
    expect(response.status, response.text).toBe(200)
    expect(response.json).toEqual({ data: [], pageCount: 0 })
  })
})
