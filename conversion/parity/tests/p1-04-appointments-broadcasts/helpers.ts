// Shared setup and comparison helpers of unit P1.4. Not a test file (vitest
// only runs `*.test.ts`). Setup goes through public HTTP routes where one
// exists (webchats, contacts, flows) and the P1.4 fixture extension where none
// does; every assertion stays black-box.

import { createHash } from "node:crypto"
import { expect } from "vitest"
import type { P104FixtureExt } from "../../src/fixtures/ext/p1-04.ts"
import { loadFixtureExt } from "../../src/fixtures/ext.ts"
import { createTenant, fixtures, type Tenant } from "../../src/fixtures/index.ts"
import { apiClient, type HttpResponse } from "../../src/http.ts"
import { comparableError } from "../../src/normalize.ts"

export type Api = ReturnType<typeof apiClient>

export const ext = (): Promise<P104FixtureExt> => loadFixtureExt<P104FixtureExt>("p1-04")

/** A new user + workspace + full token, and an API client for it. */
export const newTenant = async (label: string): Promise<{ tenant: Tenant; api: Api }> => {
  const tenant = await createTenant({ label })
  return { tenant, api: apiClient(tenant.token) }
}

/** An extra token on the tenant's workspace (read-only or scoped). */
export const extraToken = async (
  tenant: Tenant,
  options: { permission?: "full" | "read_only"; scopes?: string[] | null },
): Promise<string> => {
  const driver = await fixtures()
  const { token } = await driver.createWorkspaceToken({
    workspaceId: tenant.workspaceId,
    name: `parity-${options.permission ?? "full"}`,
    permission: options.permission ?? "full",
    scopes: options.scopes === undefined ? null : options.scopes,
  })
  return token
}

type ErrorBody = {
  defined?: boolean
  code?: string
  status?: number
  message?: string
  data?: { issues?: Array<{ path?: Array<string | number> }> } & Record<string, unknown>
}

/** Asserts status + error code; returns the parsed error body. */
export const expectError = (
  response: HttpResponse<unknown>,
  status: number,
  code: string,
): ErrorBody => {
  const body = (response.json ?? {}) as ErrorBody
  expect({ httpStatus: response.status, code: body.code, status: body.status }, response.text).toEqual({
    httpStatus: status,
    code,
    status,
  })
  return body
}

/** Status, code and `defined` of an error (P1.1 rule 9); for snapshots. */
export const errorShape = (response: HttpResponse<unknown>) => {
  const { data: _data, ...rest } = comparableError(response.json)
  return { httpStatus: response.status, ...rest }
}

/** Paths of the zod issues of a 422 `invalidRequestData`, e.g. ["maxPerDay"]. */
export const issuePaths = (response: HttpResponse<unknown>): string[] => {
  const body = (response.json ?? {}) as ErrorBody
  return (body.data?.issues ?? []).map((issue) => (issue.path ?? []).join(".")).sort()
}

/** Asserts a 422 `invalidRequestData` whose issues include `path`. */
export const expectInvalid = (response: HttpResponse<unknown>, path: string): void => {
  expectError(response, 422, "invalidRequestData")
  expect(issuePaths(response)).toContain(path)
}

/** Deep copy with every occurrence of each key replaced by its value in all strings. */
export const subst = <T>(value: T, pairs: Record<string, string>): T => {
  const entries = Object.entries(pairs)
    .filter(([from]) => from.length > 0)
    .sort((a, b) => b[0].length - a[0].length)
  const walk = (input: unknown): unknown => {
    if (typeof input === "string") {
      let output = input
      for (const [from, to] of entries) {
        output = output.split(from).join(to)
      }
      return output
    }
    if (Array.isArray(input)) {
      return input.map(walk)
    }
    if (input && typeof input === "object") {
      return Object.fromEntries(Object.entries(input).map(([key, item]) => [key, walk(item)]))
    }
    return input
  }
  return walk(value) as T
}

/** Query string in oRPC bracket notation for arrays: `ids[]=a&ids[]=b`. */
export const bracketArray = (name: string, values: string[]): string =>
  values.map((value) => `${encodeURIComponent(`${name}[]`)}=${encodeURIComponent(value)}`).join("&")

export const sha256Hex = (value: string): string => createHash("sha256").update(value).digest("hex")

// ---------------------------------------------------------------------------
// Setup through public routes of other namespaces (not parity assertions here)
// ---------------------------------------------------------------------------

export type Webchat = { id: string; inboxId: string }

export const createWebchat = async (
  api: Api,
  input: { name?: string; authorizedDomains?: string[]; enable?: boolean } = {},
): Promise<Webchat> => {
  const response = await api.post<Webchat>("/v1/webchats", {
    name: input.name ?? "Parity webchat",
    brandColor: "#007bff",
    conversationStarters: [],
    persistentMenus: [],
    authorizedDomains: input.authorizedDomains ?? ["example.com"],
    ...(input.enable === undefined ? {} : { enable: input.enable }),
  })
  expect(response.status, response.text).toBe(201)
  return { id: response.json.id, inboxId: response.json.inboxId }
}

/** Sets a webchat's allowed domains / enable flag with `PATCH /v1/webchats/{id}`. */
export const patchWebchat = async (
  api: Api,
  id: string,
  patch: { authorizedDomains?: string[]; enable?: boolean },
): Promise<Record<string, unknown>> => {
  const response = await api.patch<Record<string, unknown>>(`/v1/webchats/${id}`, patch)
  expect(response.status, response.text).toBe(200)
  return response.json
}

export type Contact = { contactId: string; conversationId: string; inboxId: string }

/** A webchat contact (contact + contact inbox + direct conversation). */
export const createContact = async (
  api: Api,
  tenant: Tenant,
  input: { inboxId: string; firstName: string; lastName?: string; email?: string },
): Promise<Contact> => {
  const response = await api.post<{ id: string }>("/v1/contacts", {
    channel: "webchat",
    inboxId: input.inboxId,
    firstName: input.firstName,
    lastName: input.lastName ?? "Parity",
    email: input.email ?? "",
    gender: "unknown",
  })
  expect(response.status, response.text).toBe(200)
  const contactId = response.json.id
  const conversationId = await (await ext()).conversationIdForContact({
    workspaceId: tenant.workspaceId,
    contactId,
  })
  return { contactId, conversationId, inboxId: input.inboxId }
}

/** A published flow with one text step. */
export const createPublishedFlow = async (api: Api, name: string, text = "Parity broadcast"): Promise<string> => {
  const response = await api.post<{ id: string }>("/v1/flows", {
    name,
    folderId: null,
    spec: { formatVersion: 1, name, steps: [{ type: "send", text }] },
    publish: true,
  })
  expect(response.status, response.text).toBe(201)
  return response.json.id
}

// ---------------------------------------------------------------------------
// Appointment calendars
// ---------------------------------------------------------------------------

/** 2030-01-07 is a Monday (weekday 1). Far in the future, so every slot is bookable. */
export const FUTURE_MONDAY = "2030-01-07"

export const createCalendar = async (api: Api, name = "Parity calendar"): Promise<string> => {
  const response = await api.post<{ id: string }>("/v1/appointment-calendars", { name })
  expect(response.status, response.text).toBe(201)
  return response.json.id
}

export type CalendarForEdit = {
  id: string
  name: string
  confirmationFlowId: string | null
  reminders: Array<{ flowId: string; timingValue: number; timingUnit: string }>
  [key: string]: unknown
}

export const getCalendar = async (api: Api, id: string): Promise<CalendarForEdit> => {
  const response = await api.get<CalendarForEdit>(`/v1/appointment-calendars/${id}`)
  expect(response.status, response.text).toBe(200)
  return response.json
}

/**
 * A full `appointmentCalendars.update` body: UTC, 30 min slots, Monday
 * 09:00-11:00, bookable on 2030-01-07..2030-01-08 only. Keeps the default
 * confirmation flow and reminders of `current` unless overridden.
 */
export const calendarUpdateBody = (
  current: CalendarForEdit,
  overrides: Record<string, unknown> = {},
): Record<string, unknown> => ({
  name: current.name,
  description: null,
  active: true,
  timezone: "UTC",
  durationMinutes: 30,
  bufferAfterMinutes: null,
  locationType: "onlineMeeting",
  locationDetail: null,
  scheduleWindowConfig: {
    scheduleWindowType: "dateRange",
    startDate: FUTURE_MONDAY,
    endDate: "2030-01-08",
    minAdvanceDays: 0,
  },
  maxAppointmentsPerUser: null,
  dailyLimitEnabled: false,
  maxPerDay: null,
  allowGroupMeeting: false,
  maxPerSlot: null,
  confirmationMessage: null,
  confirmationFlowId: current.confirmationFlowId,
  cancellationFlowId: null,
  externalConnectionId: null,
  externalEventTitleTemplate: null,
  externalEventDescriptionTemplate: null,
  externalEventAttendeesTemplate: null,
  availability: [{ weekday: 1, startMinute: 540, endMinute: 660 }],
  reminders: current.reminders.map(({ flowId, timingValue, timingUnit }) => ({
    flowId,
    timingValue,
    timingUnit,
  })),
  ...overrides,
})

/** Creates a calendar and configures it with `calendarUpdateBody`. */
export const createConfiguredCalendar = async (
  api: Api,
  overrides: Record<string, unknown> = {},
  name = "Parity calendar",
): Promise<CalendarForEdit> => {
  const id = await createCalendar(api, name)
  const current = await getCalendar(api, id)
  const updated = await api.put(`/v1/appointment-calendars/${id}`, calendarUpdateBody(current, overrides))
  expect(updated.status, updated.text).toBe(204)
  return await getCalendar(api, id)
}

export type Appointment = {
  id: string
  calendarId: string
  contactId: string
  conversationId: string | null
  startAt: string
  endAt: string
  status: string
  externalEventId: string | null
  [key: string]: unknown
}

export const bookAppointment = async (
  api: Api,
  input: Record<string, unknown>,
): Promise<Appointment> => {
  const response = await api.post<Appointment>("/v1/appointments", input)
  expect(response.status, response.text).toBe(201)
  return response.json
}

/** Keys whose values a test sets itself (future dates), kept as they are. */
export const FIXED_TIME_KEYS = ["startAt", "endAt", "sendAt", "startDate", "endDate"]

export type Broadcast = {
  id: string
  name: string
  status: string
  schedulesType: string
  schedulesAt: string
  flowId: string | null
  contactCount: number | null
  channel?: string
  subaction?: string
  targetMode?: string
  sendRatePerMinute: number | null
  recurrence?: Record<string, unknown> | null
  [key: string]: unknown
}
