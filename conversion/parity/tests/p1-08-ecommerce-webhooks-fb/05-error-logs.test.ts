// errorLogs.list — the workspace's third-party failures, newest first. The
// rows are fixture rows (the error-log event-bus writer inserts them after a
// failed provider call). The public route returns an allow-list of columns:
// never `sourceId` or `stackTrace`.

import { describe, expect, it } from "vitest"
import { expectInvalid, ext, keysOf, qs, sortQuery, tenantApi } from "./helpers.ts"

const ROW_KEYS = ["action", "contactId", "createdAt", "detail", "httpCode", "id", "updatedAt", "workspaceId"]

type ErrorLog = {
  id: string
  workspaceId: string
  contactId: string | null
  action: string
  detail: string
  httpCode: string | null
  createdAt: string
  updatedAt: string
}

type ErrorLogList = { data: ErrorLog[]; pageCount: number }

const minutesAgo = (minutes: number) => new Date(Date.now() - minutes * 60_000)

describe("errorLogs.list", () => {
  it("lists the workspace's error logs newest first, without the withheld columns", async () => {
    const { tenant, api } = await tenantApi("p108-el-list")
    const { tenant: other } = await tenantApi("p108-el-list-other")
    const fixtures = await ext()
    const oldest = minutesAgo(30)
    const middle = minutesAgo(20)
    const newest = minutesAgo(10)
    const { ids } = await fixtures.insertErrorLogs({
      workspaceId: tenant.workspaceId,
      rows: [
        { action: "messenger", detail: "(#10) Message outside the window", httpCode: "400", createdAt: middle, sourceId: "psid-1", stackTrace: "at send (/app/x.ts:1)" },
        { action: "smtp", detail: "Connection refused", httpCode: null, createdAt: oldest },
        { action: "openai", detail: "Rate limit reached", httpCode: "429", createdAt: newest },
      ],
    })
    await fixtures.insertErrorLogs({
      workspaceId: other.workspaceId,
      rows: [{ action: "smtp", detail: "Other workspace", httpCode: null, createdAt: newest }],
    })
    const [messengerId, smtpId, openaiId] = ids

    const response = await api.get<ErrorLogList>("/v1/error-logs")
    expect(response.status, response.text).toBe(200)
    expect(response.json.pageCount).toBe(1)
    expect(response.json.data.map((row) => row.id)).toEqual([openaiId, messengerId, smtpId])
    for (const row of response.json.data) {
      expect(keysOf(row)).toEqual(ROW_KEYS)
    }
    expect(response.json.data[1]).toEqual({
      id: messengerId,
      workspaceId: tenant.workspaceId,
      contactId: null,
      action: "messenger",
      detail: "(#10) Message outside the window",
      httpCode: "400",
      createdAt: middle.toISOString(),
      updatedAt: middle.toISOString(),
    })
    expect(response.json.data[2]).toMatchObject({ action: "smtp", httpCode: null, createdAt: oldest.toISOString() })
  })

  it("searches the provider slug, its display label and the detail (case-insensitive)", async () => {
    const { tenant, api } = await tenantApi("p108-el-keyword")
    const { ids } = await (await ext()).insertErrorLogs({
      workspaceId: tenant.workspaceId,
      rows: [
        { action: "smtp", detail: "Mailbox unavailable", httpCode: "550", createdAt: minutesAgo(3), sourceId: "psid-42" },
        { action: "google-sheets", detail: "Quota exceeded for SHEETS", httpCode: "429", createdAt: minutesAgo(2) },
        { action: "whatsapp", detail: "Template not approved", httpCode: "400", createdAt: minutesAgo(1) },
      ],
    })
    const [smtpId, sheetsId, whatsappId] = ids
    const search = async (keyword: string) => {
      const response = await api.get<ErrorLogList>(`/v1/error-logs${qs({ keyword })}`)
      expect(response.status, response.text).toBe(200)
      return response.json.data.map((row) => row.id)
    }
    // "Email" is the display label of `smtp`.
    expect(await search("email")).toEqual([smtpId])
    expect(await search("SMTP")).toEqual([smtpId])
    expect(await search("sheets")).toEqual([sheetsId])
    expect(await search("approved")).toEqual([whatsappId])
    expect(await search("WhatsApp")).toEqual([whatsappId])
    expect(await search("nothing-like-this")).toEqual([])
    // `sourceId` is not searched.
    expect(await search("psid")).toEqual([])
  })

  it("pages with page/perPage and ignores a sort parameter", async () => {
    const { tenant, api } = await tenantApi("p108-el-paging")
    const { ids } = await (await ext()).insertErrorLogs({
      workspaceId: tenant.workspaceId,
      rows: [
        { action: "api", detail: "a", httpCode: null, createdAt: minutesAgo(3) },
        { action: "api", detail: "b", httpCode: null, createdAt: minutesAgo(2) },
        { action: "api", detail: "c", httpCode: null, createdAt: minutesAgo(1) },
      ],
    })
    const [a, b, c] = ids
    const page1 = await api.get<ErrorLogList>(`/v1/error-logs${qs({ perPage: 2 })}`)
    expect(page1.json.pageCount).toBe(2)
    expect(page1.json.data.map((row) => row.id)).toEqual([c, b])
    const page2 = await api.get<ErrorLogList>(`/v1/error-logs${qs({ perPage: 2, page: 2 })}`)
    expect(page2.json.data.map((row) => row.id)).toEqual([a])
    // `sort` is not part of the public input; the order stays newest first.
    const sorted = await api.get<ErrorLogList>(`/v1/error-logs?${sortQuery([{ id: "createdAt", desc: false }])}`)
    expect(sorted.status, sorted.text).toBe(200)
    expect(sorted.json.data.map((row) => row.id)).toEqual([c, b, a])

    const empty = await (await tenantApi("p108-el-empty")).api.get("/v1/error-logs")
    expect(empty.json).toEqual({ data: [], pageCount: 0 })

    expectInvalid(await api.get(`/v1/error-logs${qs({ perPage: 51 })}`), "perPage")
    expectInvalid(await api.get(`/v1/error-logs${qs({ page: "x" })}`), "page")
  })
})
