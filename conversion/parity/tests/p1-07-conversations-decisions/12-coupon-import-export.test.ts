// coupons.createImportUploadUrl / import / export / getExport (scope
// `ecommerce`). The CSV goes to the storage through the presigned PUT URL;
// the worker runs the import (`runImport`) and the export (`exportCoupons`).
// An import has no public status route, so its result is read through
// `coupons.listCoupons`.

import { describe, expect, it } from "vitest"
import { eventually } from "../../src/eventually.ts"
import { expectError, expectInvalid, newTenant, qs, sortBy, UNKNOWN_ID, type Api } from "./helpers.ts"
import { bulkCreate, createTopic, type ListedCoupon, newContact } from "./helpers-ecommerce.ts"

type UploadUrl = { fileId: string; uploadUrl: string }
type ExportFile = { status: string; fileName: string; downloadUrl: string | null; totalRecords: number }

const uploadUrl = async (api: Api, fileName = "coupons.csv", size = 100): Promise<UploadUrl> => {
  const response = await api.post<UploadUrl>("/v1/coupon-imports/upload-url", { fileName, mimeType: "text/csv", size })
  expect(response.status, response.text).toBe(201)
  return response.json
}

const put = async (url: string, csv: string) => {
  const response = await fetch(url, { method: "PUT", body: Buffer.from(csv, "utf8"), headers: { "content-type": "text/csv" } })
  expect(response.status, await response.text()).toBe(200)
}

const codesOfTopic = async (api: Api, topicId: string): Promise<string[]> => {
  const response = await api.get<{ data: ListedCoupon[] }>(`/v1/coupons${qs({ topicId })}`)
  expect(response.status, response.text).toBe(200)
  return sortBy(response.json.data, (c) => c.code).map((c) => c.code)
}

describe("coupons.createImportUploadUrl", () => {
  it("201 with a file id and a 5-minute presigned PUT URL under the workspace's coupon import folder", async () => {
    const { tenant, api } = await newTenant("p107-coupon-upload-url")
    const created = await api.post<UploadUrl>("/v1/coupon-imports/upload-url", {
      fileName: "My Codes.CSV",
      mimeType: "text/csv",
      size: 1024,
    })
    expect(created.status, created.text).toBe(201)
    expect(Object.keys(created.json).sort()).toEqual(["fileId", "uploadUrl"])
    expect(created.json.fileId).toMatch(/^\d{15,20}$/)
    const url = new URL(created.json.uploadUrl)
    // The extension keeps its case; the server names the object.
    expect(url.pathname).toMatch(new RegExp(`/workspaces/${tenant.workspaceId}/imports/coupons/import_\\d+\\.CSV$`))
    expect(url.searchParams.get("X-Amz-Expires")).toBe("300")
    expect(url.searchParams.get("X-Amz-Signature")).toMatch(/^[0-9a-f]{64}$/)
  })

  it("422 for a non-.csv name, another MIME type, a size over 10 MiB or a missing field", async () => {
    const { api } = await newTenant("p107-coupon-upload-url-errors")
    const post = (body: Record<string, unknown>) => api.post("/v1/coupon-imports/upload-url", body)
    const ok = { fileName: "a.csv", mimeType: "text/csv", size: 10 }
    expectInvalid(await post({ ...ok, fileName: "a.xlsx" }), "fileName")
    expectInvalid(await post({ ...ok, fileName: "" }), "fileName")
    expectInvalid(await post({ ...ok, mimeType: "application/vnd.ms-excel" }), "mimeType")
    expectInvalid(await post({ ...ok, mimeType: "text/csv; charset=utf-8" }), "mimeType")
    expectInvalid(await post({ ...ok, size: 10 * 1024 * 1024 + 1 }), "size")
    expectInvalid(await post({ ...ok, size: 0 }), "size")
    expectInvalid(await post({ fileName: "a.csv", mimeType: "text/csv" }), "size")
    expect((await post({ ...ok, size: 10 * 1024 * 1024 })).status).toBe(201)
  })
})

describe("coupons.import", () => {
  it("201 {importId}; the worker adds the codes of the code column (trimmed, de-duplicated)", async () => {
    const { api } = await newTenant("p107-coupon-import")
    const topic = await createTopic(api)
    await bulkCreate(api, topic.id, ["IMP-OLD"])
    const file = await uploadUrl(api)
    await put(file.uploadUrl, "Name,Coupon Code\nfirst,IMP-1\nsecond, IMP-2 \nthird,IMP-1\nfourth,\nfifth,IMP-OLD\n")
    const started = await api.post<{ importId: string }>("/v1/coupon-imports", { topicId: topic.id, fileId: file.fileId })
    expect(started.status, started.text).toBe(201)
    expect(Object.keys(started.json)).toEqual(["importId"])
    expect(started.json.importId).toMatch(/^\d{15,20}$/)
    await eventually(
      async () => {
        expect(await codesOfTopic(api, topic.id)).toEqual(["IMP-1", "IMP-2", "IMP-OLD"])
      },
      { timeoutMs: 30_000, intervalMs: 300, description: "coupon import finished" },
    )
  })

  it("accepts a `code` or `coupon` header", async () => {
    const { api } = await newTenant("p107-coupon-import-headers")
    const topic = await createTopic(api)
    const a = await uploadUrl(api, "a.csv")
    await put(a.uploadUrl, "code\nHDR-A\n")
    const b = await uploadUrl(api, "b.csv")
    await put(b.uploadUrl, "COUPON\nHDR-B\n")
    for (const file of [a, b]) {
      expect((await api.post("/v1/coupon-imports", { topicId: topic.id, fileId: file.fileId })).status).toBe(201)
    }
    await eventually(
      async () => {
        expect(await codesOfTopic(api, topic.id)).toEqual(["HDR-A", "HDR-B"])
      },
      { timeoutMs: 30_000, intervalMs: 300, description: "both imports finished" },
    )
  })

  it("400 couponImportFileNotFound for a file that was never uploaded, an unknown or foreign file", async () => {
    const { api } = await newTenant("p107-coupon-import-file-errors")
    const other = await newTenant("p107-coupon-import-file-other")
    const topic = await createTopic(api)
    const notUploaded = await uploadUrl(api)
    expectError(await api.post("/v1/coupon-imports", { topicId: topic.id, fileId: notUploaded.fileId }), 400, "couponImportFileNotFound", {
      message: "Coupon import file was not uploaded",
    })
    expectError(await api.post("/v1/coupon-imports", { topicId: topic.id, fileId: UNKNOWN_ID }), 400, "couponImportFileNotFound", {
      message: "Coupon import file not found",
    })
    const foreign = await uploadUrl(other.api)
    await put(foreign.uploadUrl, "code\nFOREIGN-1\n")
    expectError(await api.post("/v1/coupon-imports", { topicId: topic.id, fileId: foreign.fileId }), 400, "couponImportFileNotFound")
    // A contact import file is not a coupon import file.
    const contactUpload = await api.post<{ fileId: string }>("/v1/contacts/imports/upload-url", {
      fileName: "contacts.csv",
      mimeType: "text/csv",
      fileSize: 10,
    })
    expect(contactUpload.status, contactUpload.text).toBe(201)
    expectError(await api.post("/v1/coupon-imports", { topicId: topic.id, fileId: contactUpload.json.fileId }), 400, "couponImportFileNotFound")
    expectInvalid(await api.post("/v1/coupon-imports", { topicId: topic.id }), "fileId")
    expectInvalid(await api.post("/v1/coupon-imports", { fileId: notUploaded.fileId }), "topicId")
  })

  it("400 couponTopicInactive for an archived or unknown topic (after the file checks)", async () => {
    const { api } = await newTenant("p107-coupon-import-topic-errors")
    const topic = await createTopic(api)
    await api.post(`/v1/coupon-topics/${topic.id}/archive`, {})
    const file = await uploadUrl(api)
    await put(file.uploadUrl, "code\nINACT-1\n")
    expectError(await api.post("/v1/coupon-imports", { topicId: topic.id, fileId: file.fileId }), 400, "couponTopicInactive", {
      message: "Coupon topic is not active",
    })
    expectError(await api.post("/v1/coupon-imports", { topicId: UNKNOWN_ID, fileId: file.fileId }), 400, "couponTopicInactive")
    // The refused file is still usable once the topic is active again.
    await api.post(`/v1/coupon-topics/${topic.id}/unarchive`, {})
    expect((await api.post("/v1/coupon-imports", { topicId: topic.id, fileId: file.fileId })).status).toBe(201)
    await eventually(
      async () => {
        expect(await codesOfTopic(api, topic.id)).toEqual(["INACT-1"])
      },
      { timeoutMs: 30_000, intervalMs: 300, description: "import after unarchive" },
    )
  })
})

const download = async (url: string) => {
  const response = await fetch(url)
  expect(response.status).toBe(200)
  const bytes = new Uint8Array(await response.arrayBuffer())
  return { contentType: response.headers.get("content-type"), bytes, text: Buffer.from(bytes).toString("utf8") }
}

const finished = async (api: Api, fileId: string): Promise<ExportFile> =>
  await eventually(
    async () => {
      const response = await api.get<ExportFile>(`/v1/coupon-exports/${fileId}`)
      expect(response.status, response.text).toBe(200)
      expect(["uploaded", "failed"], response.text).toContain(response.json.status)
      return response.json
    },
    { timeoutMs: 30_000, intervalMs: 300, description: "coupon export finished" },
  )

describe("coupons.export / getExport", () => {
  it("201 {fileId}; the worker writes a BOM-prefixed, always-quoted CSV of active-topic coupons", async () => {
    const { tenant, api } = await newTenant("p107-coupon-export")
    const topic = await createTopic(api, { name: "Export, topic" })
    await bulkCreate(api, topic.id, ["EXP-A", "EXP-B", "=EXP-C"])
    const hidden = await createTopic(api)
    await bulkCreate(api, hidden.id, ["HIDDEN-1"])
    await api.post(`/v1/coupon-topics/${hidden.id}/archive`, {})
    const contactId = await newContact(api)
    await api.post(`/v1/coupon-topics/${topic.id}/issue`, { contactId })
    await api.post(`/v1/coupon-topics/${topic.id}/mark-used`, { contactId })
    const listed = await api.get<{ data: ListedCoupon[] }>(`/v1/coupons${qs({ topicId: topic.id })}`)
    const createdAt = Object.fromEntries(listed.json.data.map((c) => [c.code, c.createdAt]))

    const started = await api.post<{ fileId: string }>("/v1/coupon-exports", {})
    expect(started.status, started.text).toBe(201)
    expect(Object.keys(started.json)).toEqual(["fileId"])
    const file = await finished(api, started.json.fileId)
    const today = new Date().toISOString().slice(0, 10)
    expect(file).toMatchObject({ status: "uploaded", fileName: `coupons-${today}.csv`, totalRecords: 3 })
    expect(Object.keys(file).sort()).toEqual(["downloadUrl", "fileName", "status", "totalRecords"])
    const url = new URL(file.downloadUrl as string)
    expect(url.pathname).toMatch(
      new RegExp(`/workspaces/${tenant.workspaceId}/exports/coupons/coupon_\\d+\\.csv$`),
    )
    expect(url.searchParams.get("X-Amz-Expires")).toBe("300")
    const csv = await download(file.downloadUrl as string)
    expect(csv.contentType).toBe("text/csv; charset=utf-8")
    expect([...csv.bytes.slice(0, 3)]).toEqual([0xef, 0xbb, 0xbf])
    // Rows by coupon id; the "Used" column holds the ISSUE status; a leading
    // "=" is guarded with a quote.
    expect(csv.text.slice(1)).toBe(
      [
        '"Name","Code","Used","Date"',
        `"Export, topic","EXP-A","Published","${createdAt["EXP-A"]}"`,
        `"Export, topic","EXP-B","Unpublished","${createdAt["EXP-B"]}"`,
        `"Export, topic","'=EXP-C","Unpublished","${createdAt["=EXP-C"]}"`,
        "",
      ].join("\n"),
    )
  })

  it("applies the topic / issue / usage / search filters; an empty result is a failed file", async () => {
    const { api } = await newTenant("p107-coupon-export-filters")
    const one = await createTopic(api)
    const two = await createTopic(api)
    await bulkCreate(api, one.id, ["F-ONE-1", "F-ONE-2"])
    await bulkCreate(api, two.id, ["F-TWO-1"])
    const contactId = await newContact(api)
    await api.post(`/v1/coupon-topics/${one.id}/issue`, { contactId })

    const codes = async (filter: Record<string, string>) => {
      const started = await api.post<{ fileId: string }>("/v1/coupon-exports", filter)
      expect(started.status, started.text).toBe(201)
      const file = await finished(api, started.json.fileId)
      if (file.status !== "uploaded") {
        return { file, codes: [] as string[] }
      }
      const text = (await download(file.downloadUrl as string)).text.slice(1)
      return {
        file,
        codes: text
          .trim()
          .split("\n")
          .slice(1)
          .map((line) => line.split(",")[1]?.replace(/"/g, "")),
      }
    }
    expect((await codes({ topicId: two.id })).codes).toEqual(["F-TWO-1"])
    expect((await codes({ issueStatus: "published" })).codes).toEqual(["F-ONE-1"])
    expect((await codes({ usageStatus: "notUsed", search: "one" })).codes).toEqual(["F-ONE-1", "F-ONE-2"])
    const empty = await codes({ usageStatus: "used" })
    expect(empty.file).toMatchObject({ status: "failed", downloadUrl: null, totalRecords: 0 })
  })

  it("getExport 404 for an unknown or foreign file; export / getExport 422 for bad input", async () => {
    const { api } = await newTenant("p107-coupon-export-errors")
    const other = await newTenant("p107-coupon-export-other")
    const foreign = await other.api.post<{ fileId: string }>("/v1/coupon-exports", {})
    expect(foreign.status, foreign.text).toBe(201)
    expectError(await api.get(`/v1/coupon-exports/${UNKNOWN_ID}`), 404, "notFound", { message: "Export file not found" })
    expectError(await api.get(`/v1/coupon-exports/${foreign.json.fileId}`), 404, "notFound")
    // A coupon import file is not an export file.
    const upload = await uploadUrl(api)
    expectError(await api.get(`/v1/coupon-exports/${upload.fileId}`), 404, "notFound")
    expectInvalid(await api.get("/v1/coupon-exports/abc"), "fileId")
    expectInvalid(await api.post("/v1/coupon-exports", { search: "  " }), "search")
    expectInvalid(await api.post("/v1/coupon-exports", { issueStatus: "issued" }), "issueStatus")
    expectInvalid(await api.post("/v1/coupon-exports", { usageStatus: "yes" }), "usageStatus")
    expectInvalid(await api.post("/v1/coupon-exports", { topicId: "abc" }), "topicId")
  })
})
