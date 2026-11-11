// contacts.getImportTemplate / createImportUpload / peekImportHeaders /
// import / listImports / getImport. The file goes to the storage through
// the presigned PUT URL the API returns; the worker runs the import.

import { beforeAll, describe, expect, it } from "vitest"
import { eventually } from "../../src/eventually.ts"
import type { Tenant } from "../../src/fixtures/index.ts"
import { expectNormalized, Normalizer } from "../../src/normalize.ts"
import {
  type Api,
  contactPath,
  createCustomField,
  createTag,
  expectError,
  expectInvalid,
  ext,
  filter,
  idOf,
  listContacts,
  newTenant,
  qs,
} from "./helpers.ts"

type Upload = { fileId: string; presignedPostUrl: string; publicUrl: string; path: string }
type ImportJob = {
  id: string
  status: string
  totalCount: number | null
  processedCount: number | null
  successCount: number | null
  failedCount: number | null
  errorMessage: string | null
  errorSample: Array<{ row?: number; reason: string }>
  [key: string]: unknown
}

let tenant: Tenant
let api: Api
let messengerInbox: string

beforeAll(async () => {
  ;({ tenant, api } = await newTenant("p106-import"))
  messengerInbox = (await (await ext()).createInbox({ workspaceId: tenant.workspaceId, channel: "messenger", name: "Import M" })).inboxId
})

const upload = async (csv: string | Uint8Array, fileName = "contacts.csv"): Promise<Upload> => {
  const bytes = typeof csv === "string" ? Buffer.from(csv, "utf8") : Buffer.from(csv)
  const response = await api.post<Upload>("/v1/contacts/imports/upload-url", {
    fileName,
    mimeType: "text/csv",
    fileSize: bytes.length,
  })
  expect(response.status, response.text).toBe(201)
  const put = await fetch(response.json.presignedPostUrl, { method: "PUT", body: bytes, headers: { "content-type": "text/csv" } })
  expect(put.status, await put.text()).toBe(200)
  return response.json
}

const runImport = async (body: Record<string, unknown>): Promise<ImportJob> => {
  const started = await api.post<{ importId: string }>("/v1/contacts/import", body)
  expect(started.status, started.text).toBe(201)
  return await eventually(
    async () => {
      const job = await api.get<ImportJob>(`/v1/contacts/imports/${started.json.importId}`)
      expect(job.status, job.text).toBe(200)
      expect(["completed", "failed"], job.text).toContain(job.json.status)
      return job.json
    },
    { timeoutMs: 30_000, intervalMs: 300, description: "import finished" },
  )
}

describe("getImportTemplate", () => {
  it("returns the CSV template in the workspace language, or the asked one", async () => {
    const en = await api.get<{ fileName: string; mimeType: string; content: string }>("/v1/contacts/import-template")
    expect(en.status, en.text).toBe(200)
    expect(en.json).toEqual({
      fileName: "contacts-import-template.csv",
      mimeType: "text/csv",
      content:
        '﻿"Contact ID","Phone number","Email","First name","Last name","WhatsApp User ID"\n' +
        '"1234567890","+14155550100","john.doe@example.com","John","Doe","user.9187654321098765"\n',
    })
    const vi = await api.get<{ content: string }>(`/v1/contacts/import-template${qs({ language: "vi" })}`)
    expect(vi.status, vi.text).toBe(200)
    expect(vi.json.content.startsWith("﻿")).toBe(true)
    expect(vi.json.content).toMatchSnapshot()
    expectInvalid(await api.get(`/v1/contacts/import-template${qs({ language: "fr" })}`), "language")
  })
})

describe("createImportUpload", () => {
  it("returns a presigned PUT URL, a public URL and the storage path", async () => {
    const n = new Normalizer()
    const response = await api.post<Upload>("/v1/contacts/imports/upload-url", {
      fileName: "My Contacts.CSV",
      mimeType: "text/csv",
      fileSize: 1024,
    })
    expect(response.status, response.text).toBe(201)
    const { fileId, path, presignedPostUrl, publicUrl } = response.json
    // The declared extension keeps its case.
    expect(path).toMatch(new RegExp(`^workspaces/${tenant.workspaceId}/imports/contacts/import_\\d+\\.CSV$`))
    expect(new URL(presignedPostUrl).pathname.endsWith(`/${path}`)).toBe(true)
    expect(new URL(presignedPostUrl).searchParams.get("X-Amz-Expires")).toBe("300")
    expect(publicUrl.endsWith(`/storage/${path}`)).toBe(true)
    const stablePath = path.replace(tenant.workspaceId, "<workspace>").replace(/import_\d+/, "import_<id>")
    expectNormalized(
      n,
      { fileId, path: stablePath, presignedPostUrl: "<url>", publicUrl: "<url>" },
      { replace: { [tenant.workspaceId]: "<workspace>" } },
    ).toMatchSnapshot()
  })

  it("only CSV up to 20 MB", async () => {
    const post = (body: Record<string, unknown>) => api.post("/v1/contacts/imports/upload-url", body)
    const ok = { fileName: "a.csv", mimeType: "text/csv", fileSize: 10 }
    expectError(await post({ ...ok, mimeType: "application/vnd.ms-excel" }), 400, "importUnsupportedFileType")
    expectError(await post({ ...ok, mimeType: "text/csv; charset=utf-8" }), 400, "importUnsupportedFileType")
    expectError(await post({ ...ok, fileName: "a.xlsx" }), 400, "importUnsupportedFileType")
    expectError(await post({ ...ok, fileSize: 20 * 1024 * 1024 + 1 }), 400, "importFileTooLarge")
    expect((await post({ ...ok, fileSize: 20 * 1024 * 1024 })).status).toBe(201)
    expectInvalid(await post({ ...ok, fileSize: 0 }), "fileSize")
    expectInvalid(await post({ ...ok, fileSize: 1.5 }), "fileSize")
    expectInvalid(await post({ ...ok, fileName: "" }), "fileName")
    expectInvalid(await post({ mimeType: "text/csv", fileSize: 10 }), "fileName")
  })
})

describe("peekImportHeaders", () => {
  it("reads the header row (BOM stripped, trimmed) and suggests a column map", async () => {
    const file = await upload("﻿ Số điện thoại ,PSID,E-mail,Tên,Họ,BSUID,Note\n0901,u1,a@b.co,An,Ng,US.1,x\n")
    const response = await api.get(`/v1/contacts/imports/files/${file.fileId}/headers`)
    expect(response.status, response.text).toBe(200)
    expect(response.json).toEqual({
      headers: ["Số điện thoại", "PSID", "E-mail", "Tên", "Họ", "BSUID", "Note"],
      suggestedColumnMap: {
        phoneNumber: "Số điện thoại",
        contactId: "PSID",
        email: "E-mail",
        firstName: "Tên",
        lastName: "Họ",
        sourceUserId: "BSUID",
      },
    })
  })

  it("unknown columns are not suggested; ; is not a delimiter", async () => {
    const file = await upload("a;b;c\n1;2;3\n")
    expect((await api.get(`/v1/contacts/imports/files/${file.fileId}/headers`)).json).toEqual({
      headers: ["a;b;c"],
      suggestedColumnMap: {},
    })
  })

  it("400 importUnableToReadHeaders: not uploaded yet, header row only, unknown file; 422 bad id", async () => {
    const declared = await api.post<Upload>("/v1/contacts/imports/upload-url", { fileName: "x.csv", mimeType: "text/csv", fileSize: 5 })
    expectError(await api.get(`/v1/contacts/imports/files/${declared.json.fileId}/headers`), 400, "importUnableToReadHeaders")
    const headerOnly = await upload("contactId,email\n")
    expectError(await api.get(`/v1/contacts/imports/files/${headerOnly.fileId}/headers`), 400, "importUnableToReadHeaders")
    expectError(await api.get("/v1/contacts/imports/files/1/headers"), 400, "importUnableToReadHeaders")
    expectInvalid(await api.get("/v1/contacts/imports/files/abc/headers"), "fileId")
  })
})

describe("import", () => {
  it("imports new contacts with tag and custom field; existing ids and invalid rows fail", async () => {
    const tagId = await createTag(api, "imported-tag")
    const fieldId = await createCustomField(api, "import_score", "number")
    const fx = await ext()
    await fx.createContact({
      workspaceId: tenant.workspaceId,
      firstName: "Already",
      inboxes: [{ inboxId: messengerInbox, channel: "messenger", sourceId: "dup-1" }],
    })
    const file = await upload(
      [
        "uid,mail,first,last,score",
        "imp-1,ONE@Parity.Test,Émile,Zola,7",
        "imp-2,not-an-email,Zoë,,x",
        ",,,,",
        "dup-1,dup@parity.test,Dup,Licate,1",
        "imp-1,again@parity.test,Same,Id,2",
      ].join("\n"),
    )
    const job = await runImport({
      fileId: file.fileId,
      channel: "messenger",
      inboxId: messengerInbox,
      contactId: "uid",
      email: "mail",
      firstName: "first",
      lastName: "last",
      tagId,
      fieldMapping: [{ column: "score", customFieldId: fieldId }],
    })
    expect(job).toMatchObject({
      status: "completed",
      type: "contacts",
      fileId: file.fileId,
      fileName: "contacts.csv",
      userId: null,
      totalCount: 5,
      processedCount: 5,
      successCount: 2,
      failedCount: 3,
    })
    expect(job.errorSample).toEqual([{ row: 4, reason: "Invalid row" }])
    expect(job.errorMessage).toBe("3 rows failed; showing the first 1")

    const imported = await listContacts(api, { contactFilter: filter([{ field: "source", operator: "in", value: ["imported"] }]) })
    expect(imported.status, imported.text).toBe(200)
    const rows = imported.json.data as Array<{ id: string; firstName: string | null; lastName: string | null; email: string | null }>
    expect(rows.map((row) => [row.firstName, row.lastName, row.email]).sort()).toEqual([
      ["Zoë", null, null],
      ["Émile", "Zola", "one@parity.test"],
    ].sort())
    const emile = rows.find((row) => row.firstName === "Émile") as { id: string }
    const tags = await api.get<{ data: Array<{ id: string }> }>(contactPath(idOf(emile.id), "/tags"))
    expect(tags.json.data.map((tag) => tag.id)).toEqual([tagId])
    const fields = await api.get<{ data: Array<{ id: string; value: string }> }>(contactPath(idOf(emile.id), "/custom-fields"))
    expect(fields.json.data.map((row) => [row.id, row.value])).toEqual([[fieldId, "7"]])
    const zoe = rows.find((row) => row.firstName === "Zoë") as { id: string }
    // An invalid number is skipped, the contact is still imported.
    expect((await api.get(contactPath(idOf(zoe.id), "/custom-fields"))).json).toEqual({ data: [] })
    const inboxes = await api.get<{ data: Array<{ source: string; sourceId: string; channel: string }> }>(
      contactPath(idOf(emile.id), "/inboxes"),
    )
    expect(inboxes.json.data).toMatchObject([{ source: "imported", sourceId: "imp-1", channel: "messenger" }])
  })

  it("whatsapp: phones are normalized with countryCode; the phone digits are the channel id", async () => {
    const fx = await ext()
    const whatsapp = (await fx.createInbox({ workspaceId: tenant.workspaceId, channel: "whatsapp", name: "Import W" })).inboxId
    const file = await upload("phone,name\n0912 345 678,Lan\n+1 202 456 1111,Sam\n12,Bad\n")
    const job = await runImport({
      fileId: file.fileId,
      channel: "whatsapp",
      inboxId: whatsapp,
      phoneNumber: "phone",
      firstName: "name",
      countryCode: "+84",
    })
    expect(job).toMatchObject({ status: "completed", totalCount: 3, successCount: 2, failedCount: 1 })
    expect(job.errorSample).toEqual([{ row: 4, reason: "Invalid row" }])
    const lan = await api.get<{ phoneNumber: string; contactInboxes: Array<{ sourceId: string }> }>(contactPath("phone:+84912345678"))
    expect(lan.status, lan.text).toBe(200)
    expect(lan.json.contactInboxes[0]?.sourceId).toBe("84912345678")
    expect((await api.get(contactPath("phone:+12024561111"))).status).toBe(200)
  })

  it("start errors", async () => {
    const file = await upload("uid\nx1\n")
    const base = { fileId: file.fileId, channel: "messenger", inboxId: messengerInbox, contactId: "uid" }
    expectError(await api.post("/v1/contacts/import", { ...base, fileId: "1" }), 404, "contactImportFileNotFound", {
      message: "File not found",
    })
    expectError(await api.post("/v1/contacts/import", { ...base, inboxId: "1" }), 404, "contactImportInboxNotFound", {
      message: "Inbox not found",
    })
    expectInvalid(await api.post("/v1/contacts/import", { ...base, contactId: undefined }), "contactId")
    expectInvalid(
      await api.post("/v1/contacts/import", { ...base, channel: "whatsapp", contactId: undefined }),
      "phoneNumber",
    )
    expectInvalid(await api.post("/v1/contacts/import", { ...base, countryCode: "84" }), "countryCode")
    expectInvalid(await api.post("/v1/contacts/import", { ...base, channel: "fax" }), "channel")
    expectInvalid(await api.post("/v1/contacts/import", { ...base, fileId: "abc" }), "fileId")
    expectInvalid(
      await api.post("/v1/contacts/import", {
        ...base,
        fieldMapping: Array.from({ length: 11 }, (_, i) => ({ column: `c${i}`, customFieldId: "1" })),
      }),
      "fieldMapping",
    )
    // An export file is not an import file.
    const exported = await api.post<{ fileId: string }>("/v1/contacts/export", { fields: ["sys:firstName"], exportAll: true })
    expect(exported.status, exported.text).toBe(202)
    expectError(await api.post("/v1/contacts/import", { ...base, fileId: exported.json.fileId }), 400, "contactImportFileTypeInvalid", {
      message: "File is not a contacts import",
    })
  })
})

describe("listImports / getImport", () => {
  it("lists newest first with pageCount; get by id; another workspace's import is 404", async () => {
    const { tenant: t, api: a } = await newTenant("p106-import-list")
    const fx = await ext()
    const inbox = (await fx.createInbox({ workspaceId: t.workspaceId, channel: "telegram", name: "L" })).inboxId
    const startOne = async (csv: string) => {
      const declared = await a.post<Upload>("/v1/contacts/imports/upload-url", {
        fileName: "list.csv",
        mimeType: "text/csv",
        fileSize: Buffer.byteLength(csv),
      })
      await fetch(declared.json.presignedPostUrl, { method: "PUT", body: Buffer.from(csv), headers: { "content-type": "text/csv" } })
      const started = await a.post<{ importId: string }>("/v1/contacts/import", {
        fileId: declared.json.fileId,
        channel: "telegram",
        inboxId: inbox,
        contactId: "id",
      })
      expect(started.status, started.text).toBe(201)
      await eventually(async () => {
        const job = await a.get<ImportJob>(`/v1/contacts/imports/${started.json.importId}`)
        expect(job.json.status).toBe("completed")
      }, { timeoutMs: 30_000, intervalMs: 300 })
      return started.json.importId
    }
    const first = await startOne("id\nt-1\n")
    const second = await startOne("id\nt-2\nt-3\n")

    const n = new Normalizer()
    const list = await a.get<{ data: ImportJob[]; pageCount: number }>("/v1/contacts/imports")
    expect(list.status, list.text).toBe(200)
    expect(list.json.data.map((row) => row.id)).toEqual([second, first])
    expect(list.json.pageCount).toBe(1)
    expectNormalized(n, list.json).toMatchSnapshot()
    const paged = await a.get<{ data: ImportJob[]; pageCount: number }>(`/v1/contacts/imports${qs({ perPage: 1, page: 2 })}`)
    expect(paged.json.data.map((row) => row.id)).toEqual([first])
    expect(paged.json.pageCount).toBe(2)
    const byStatus = await a.get<{ data: ImportJob[] }>(`/v1/contacts/imports${qs({ status: "failed" })}`)
    expect(byStatus.json.data).toEqual([])

    expectError(await api.get(`/v1/contacts/imports/${first}`), 404, "notFound", { message: "Import not found" })
    expectError(await a.get("/v1/contacts/imports/1"), 404, "notFound", { message: "Import not found" })
  })
})
