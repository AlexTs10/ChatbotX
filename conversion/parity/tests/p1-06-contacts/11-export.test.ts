// contacts.export / getExportFile / listExportFiles. The worker writes the
// CSV to the storage; the test downloads it through the presigned URL.

import { beforeAll, describe, expect, it } from "vitest"
import { eventually } from "../../src/eventually.ts"
import type { Tenant } from "../../src/fixtures/index.ts"
import { expectNormalized, Normalizer } from "../../src/normalize.ts"
import {
  type Api,
  at,
  contactPath,
  createCustomField,
  createTag,
  expectError,
  expectInvalid,
  ext,
  filter,
  idOf,
  newTenant,
  qs,
} from "./helpers.ts"

type ExportFile = { status: string; fileName: string; downloadUrl: string | null; totalRecords: number | null }

let tenant: Tenant
let api: Api
const id: Record<string, string> = {}
let tagId: string
let fieldId: string

beforeAll(async () => {
  ;({ tenant, api } = await newTenant("p106-export"))
  const fx = await ext()
  const ws = tenant.workspaceId
  const { inboxId } = await fx.createInbox({ workspaceId: ws, channel: "telegram", name: "Export T" })
  tagId = await createTag(api, "Exported VIP")
  fieldId = await createCustomField(api, "Birthday", "date")
  // Contact ids grow in creation order; the export pages by id.
  id.ada = (
    await fx.createContact({
      workspaceId: ws,
      firstName: "Ada",
      lastName: 'Love "Lace"',
      email: "ada@parity.test",
      phoneNumber: "+14155550100",
      gender: "female",
      createdAt: at("2025-01-01T00:00:00Z"),
      blockedAt: at("2025-02-03T04:05:06Z"),
      inboxes: [{ inboxId, channel: "telegram", sourceId: "tg-ada", source: "ads" }],
    })
  ).contactId
  id.bob = (
    await fx.createContact({
      workspaceId: ws,
      firstName: "=cmd",
      lastName: "Multi\nLine",
      createdAt: at("2025-01-02T00:00:00Z"),
      inboxes: [{ inboxId, channel: "telegram", sourceId: "tg-bob" }],
    })
  ).contactId
  await fx.linkTag({ contactId: id.ada, tagId })
  await fx.setCustomFieldValue({ contactId: id.ada, customFieldId: fieldId, value: "1990-12-10T00:00:00Z" })
})

const FIELDS = () => [
  "sys:firstName",
  "sys:lastName",
  "sys:fullName",
  "sys:email",
  "sys:phoneNumber",
  "sys:gender",
  "sys:source",
  "sys:blockedAt",
  "sys:contactId",
  "sys:sourceUserId",
  "sys:adIds",
  "sys:noSuchColumn",
  `tag:${tagId}`,
  `cus:${fieldId}`,
  "cus:1",
  "tag:1",
]

const startExport = async (body: Record<string, unknown>) => {
  const response = await api.post<{ fileId: string }>("/v1/contacts/export", body)
  expect(response.status, response.text).toBe(202)
  return response.json.fileId
}
const finished = (fileId: string) =>
  eventually(
    async () => {
      const response = await api.get<ExportFile>(`/v1/contacts/export-files/${fileId}`)
      expect(response.status, response.text).toBe(200)
      expect(response.json.status).not.toBe("pending")
      return response.json
    },
    { timeoutMs: 30_000, intervalMs: 300, description: "export finished" },
  )
const download = async (url: string) => {
  const response = await fetch(url)
  expect(response.status).toBe(200)
  const bytes = new Uint8Array(await response.arrayBuffer())
  return { contentType: response.headers.get("content-type"), bytes, text: Buffer.from(bytes).toString("utf8") }
}

describe("export", () => {
  it("exportAll writes a BOM-prefixed, always-quoted CSV with formula guards", async () => {
    const n = new Normalizer()
    const fileId = await startExport({ fields: FIELDS(), exportAll: true })
    const file = await finished(fileId)
    const today = new Date().toISOString().slice(0, 10)
    expect(file).toMatchObject({ status: "uploaded", fileName: `contacts-${today}.csv`, totalRecords: 2 })
    expect(file.downloadUrl).toMatch(new RegExp(`/workspaces/${tenant.workspaceId}/exports/contacts/contact_${fileId}\\.csv\\?`))
    expectNormalized(n, { ...file, fileName: "contacts-<date>.csv", downloadUrl: "<presigned GET url>" }).toMatchSnapshot()
    const csv = await download(file.downloadUrl as string)
    expect(csv.contentType).toBe("text/csv; charset=utf-8")
    expect([...csv.bytes.slice(0, 3)]).toEqual([0xef, 0xbb, 0xbf])
    expect(csv.text.slice(1)).toBe(
      [
        '"First Name","Last Name","Full Name","Email","Phone Number","Gender","Source","Blocked At","Contact ID","WhatsApp User ID","Ad IDs","noSuchColumn","Exported VIP","Birthday","1","1"',
        // `sys:source` is always empty; an unknown tag column is "No", an unknown custom field "".
        '"Ada","Love ""Lace""","Ada Love ""Lace""","ada@parity.test","\'+14155550100","female","","2025-02-03T04:05:06.000Z","tg-ada","","","","Yes","1990-12-10","","No"',
        '"\'=cmd","Multi Line","\'=cmd Multi Line","","","","","","tg-bob","","","","No","","","No"',
        "",
      ].join("\n"),
    )
  })

  it("contactIds exports only those; a filter narrows exportAll", async () => {
    const byIds = await finished(await startExport({ fields: ["sys:firstName"], contactIds: [id.bob] }))
    expect(byIds).toMatchObject({ status: "uploaded", totalRecords: 1 })
    expect((await download(byIds.downloadUrl as string)).text.slice(1)).toBe('"First Name"\n"\'=cmd"\n')

    const filtered = await finished(
      await startExport({
        fields: ["sys:email"],
        exportAll: true,
        filter: { contactFilter: filter([{ field: "tags", operator: "in", value: [tagId] }]) },
      }),
    )
    expect(filtered).toMatchObject({ status: "uploaded", totalRecords: 1 })
    expect((await download(filtered.downloadUrl as string)).text.slice(1)).toBe('"Email"\n"ada@parity.test"\n')

    const byKeyword = await finished(await startExport({ fields: ["sys:firstName"], exportAll: true, filter: { keyword: "ada" } }))
    expect(byKeyword.totalRecords).toBe(1)
    // exportAll wins over contactIds.
    const both = await finished(await startExport({ fields: ["sys:firstName"], exportAll: true, contactIds: [id.bob] }))
    expect(both.totalRecords).toBe(2)
  })

  it("an export that matches nobody ends failed with 0 records and no download", async () => {
    const file = await finished(await startExport({ fields: ["sys:firstName"], contactIds: ["1"] }))
    expect(file).toMatchObject({ status: "failed", downloadUrl: null, totalRecords: 0 })
  })

  it("validation", async () => {
    expectInvalid(await api.post("/v1/contacts/export", { fields: [], exportAll: true }), "fields")
    expectInvalid(await api.post("/v1/contacts/export", { fields: ["firstName"], exportAll: true }), "fields.0")
    expectInvalid(await api.post("/v1/contacts/export", { fields: ["cf:1"], exportAll: true }), "fields.0")
    expectInvalid(await api.post("/v1/contacts/export", { fields: ["sys:firstName"] }), "contactIds")
    expectInvalid(await api.post("/v1/contacts/export", { fields: ["sys:firstName"], contactIds: [] }), "contactIds")
    expectInvalid(
      await api.post("/v1/contacts/export", { fields: ["sys:firstName"], exportAll: true, filter: { contactFilter: { operator: "x" } } }),
      "filter.contactFilter.operator",
    )
  })
})

describe("getExportFile / listExportFiles", () => {
  it("lists exports newest first with cursor pages; unknown file 404", async () => {
    const n = new Normalizer()
    const { tenant: t, api: a } = await newTenant("p106-export-list")
    const fx = await ext()
    await fx.createContact({ workspaceId: t.workspaceId, firstName: "One" })
    const start = async () => {
      const response = await a.post<{ fileId: string }>("/v1/contacts/export", { fields: ["sys:firstName"], exportAll: true })
      expect(response.status, response.text).toBe(202)
      return response.json.fileId
    }
    const first = await start()
    const second = await start()
    await eventually(async () => {
      const list = await a.get<{ data: Array<{ id: string; status: string }> }>("/v1/contacts/export-files")
      expect(list.json.data.map((row) => row.status)).toEqual(["uploaded", "uploaded"])
    }, { timeoutMs: 30_000, intervalMs: 300 })
    const list = await a.get<{ data: Array<{ id: string; createdAt: string; expiresAt: string }>; nextCursor: string | null }>(
      "/v1/contacts/export-files",
    )
    expect(list.json.data.map((row) => row.id)).toEqual([second, first])
    expect(list.json.nextCursor).toBeNull()
    for (const row of list.json.data) {
      expect(Date.parse(row.expiresAt) - Date.parse(row.createdAt)).toBe(7 * 86_400_000)
    }
    expect(list.json.data[0]).toMatchObject({ requestedBy: null, totalRecords: 1, fileName: expect.stringMatching(/^contacts-\d{4}-\d{2}-\d{2}\.csv$/) })
    expectNormalized(n, list.json, { replace: { [list.json.data[0]?.fileName ?? "-"]: "contacts-<date>.csv" } }).toMatchSnapshot()

    const page1 = await a.get<{ data: Array<{ id: string }>; nextCursor: string | null }>(`/v1/contacts/export-files${qs({ perPage: 1 })}`)
    expect(page1.json.data.map((row) => row.id)).toEqual([second])
    expect(typeof page1.json.nextCursor).toBe("string")
    const page2 = await a.get<{ data: Array<{ id: string }>; nextCursor: string | null }>(
      `/v1/contacts/export-files${qs({ perPage: 1, cursor: page1.json.nextCursor as string })}`,
    )
    expect(page2.json.data.map((row) => row.id)).toEqual([first])
    expect(page2.json.nextCursor).toBeNull()

    expectError(await a.get(`/v1/contacts/export-files${qs({ cursor: "garbage" })}`), 400, "invalidCursor")
    expectInvalid(await a.get(`/v1/contacts/export-files${qs({ perPage: 101 })}`), "perPage")
    expectError(await a.get("/v1/contacts/export-files/1"), 404, "notFound", { message: "Export file not found" })
    expectError(await api.get(`/v1/contacts/export-files/${first}`), 404, "notFound")
    expectInvalid(await a.get("/v1/contacts/export-files/abc"), "fileId")
    expect((await a.get(contactPath(idOf("1")))).status).toBe(404)
  })
})
