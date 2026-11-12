// products.getImportTemplate / createImportUpload / peekImportHeaders /
// startImport / listImports / getImport. The file goes to storage through
// the presigned PUT URL; the worker runs the import and creates the products.

import { describe, expect, it, vi } from "vitest"
import {
  type Api,
  expectError,
  expectInvalid,
  expectNotFound,
  ISO,
  keysOf,
  newTenant,
  SNOWFLAKE,
  UNKNOWN_ID,
  wait,
} from "./helpers.ts"

// Worker jobs and many writes per test: allow more than the 30 s default on a loaded runner.
vi.setConfig({ testTimeout: 120_000 })

type Upload = { fileId: string; presignedPostUrl: string; publicUrl: string; path: string }
type ImportJob = Record<string, unknown> & {
  id: string
  status: string
  type: string
  fileId: string
  successCount: number
  failedCount: number
  errorSample: Array<{ row?: number; reason: string }>
}

const IMPORT_KEYS = [
  "completedAt",
  "createdAt",
  "errorMessage",
  "errorSample",
  "failedCount",
  "fileId",
  "fileName",
  "id",
  "processedCount",
  "status",
  "successCount",
  "totalCount",
  "type",
  "updatedAt",
  "userId",
]

const upload = async (api: Api, csv: string, fileName = "products.csv"): Promise<Upload> => {
  const bytes = Buffer.from(csv, "utf8")
  const response = await api.post<Upload>("/v1/products/imports/upload-url", {
    fileName,
    mimeType: "text/csv",
    fileSize: bytes.length,
  })
  expect(response.status, response.text).toBe(201)
  const put = await fetch(response.json.presignedPostUrl, { method: "PUT", body: bytes, headers: { "content-type": "text/csv" } })
  expect(put.status, await put.text()).toBe(200)
  return response.json
}

const finished = (api: Api, importId: string): Promise<ImportJob> =>
  wait(async () => {
    const job = await api.get<ImportJob>(`/v1/products/imports/${importId}`)
    expect(job.status, job.text).toBe(200)
    expect(["completed", "failed"], job.text).toContain(job.json.status)
    return job.json
  })

describe("products.getImportTemplate", () => {
  it("returns the XLSX template in English or Vietnamese as base64", async () => {
    const { api } = await newTenant("p114-imp-template")
    const en = await api.get<{ fileName: string; mimeType: string; contentBase64: string }>("/v1/products/import-template")
    expect(en.status, en.text).toBe(200)
    expect(keysOf(en.json)).toEqual(["contentBase64", "fileName", "mimeType"])
    expect(en.json.mimeType).toBe("application/vnd.openxmlformats-officedocument.spreadsheetml.sheet")
    expect(en.json.fileName).toMatch(/\.xlsx$/)
    // An XLSX file is a ZIP archive.
    expect(Buffer.from(en.json.contentBase64, "base64").subarray(0, 2).toString("latin1")).toBe("PK")
    const vi = await api.get<{ fileName: string }>("/v1/products/import-template", { query: { language: "vi" } })
    expect(vi.status, vi.text).toBe(200)
    expect(vi.json.fileName).toMatch(/\.xlsx$/)
    expectInvalid(await api.get("/v1/products/import-template", { query: { language: "fr" } }), "language")
  })
})

describe("products.createImportUpload", () => {
  it("returns a presigned PUT URL under the workspace's product import folder; CSV or XLSX up to 10 MB", async () => {
    const { tenant, api } = await newTenant("p114-imp-upload")
    const response = await api.post<Upload>("/v1/products/imports/upload-url", {
      fileName: "Catalog.xlsx",
      mimeType: "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
      fileSize: 2048,
    })
    expect(response.status, response.text).toBe(201)
    expect(keysOf(response.json)).toEqual(["fileId", "path", "presignedPostUrl", "publicUrl"])
    const { fileId, path, presignedPostUrl, publicUrl } = response.json
    expect(fileId).toMatch(SNOWFLAKE)
    expect(path).toMatch(new RegExp(`^workspaces/${tenant.workspaceId}/imports/products/import_\\d+\\.xlsx$`))
    expect(new URL(presignedPostUrl).pathname.endsWith(`/${path}`)).toBe(true)
    expect(publicUrl.endsWith(`/storage/${path}`)).toBe(true)

    const post = (body: Record<string, unknown>) => api.post("/v1/products/imports/upload-url", body)
    const ok = { fileName: "a.csv", mimeType: "text/csv", fileSize: 10 }
    expectError(await post({ ...ok, mimeType: "application/pdf" }), 400, "importUnsupportedFileType")
    expectError(await post({ ...ok, fileName: "a.xlsx" }), 400, "importUnsupportedFileType")
    expectError(await post({ ...ok, fileSize: 10 * 1024 * 1024 + 1 }), 400, "importFileTooLarge")
    expect((await post({ ...ok, fileSize: 10 * 1024 * 1024 })).status).toBe(201)
    expectInvalid(await post({ ...ok, fileSize: 0 }), "fileSize")
    expectInvalid(await post({ ...ok, fileName: "" }), "fileName")
  })
})

describe("products.peekImportHeaders", () => {
  it("reads the header row and suggests a column map", async () => {
    const { api } = await newTenant("p114-imp-headers")
    const file = await upload(api, "Product name,Code,Selling price,Brand,Notes\nMug,M1,3,Acme,x\n")
    const response = await api.get(`/v1/products/imports/files/${file.fileId}/headers`)
    expect(response.status, response.text).toBe(200)
    expect(response.json).toEqual({
      headers: ["Product name", "Code", "Selling price", "Brand", "Notes"],
      suggestedColumnMap: { name: "Product name", sku: "Code", price: "Selling price", vendor: "Brand" },
    })
    const declared = await api.post<Upload>("/v1/products/imports/upload-url", { fileName: "x.csv", mimeType: "text/csv", fileSize: 5 })
    expectError(await api.get(`/v1/products/imports/files/${declared.json.fileId}/headers`), 400, "importUnableToReadHeaders")
    expectInvalid(await api.get("/v1/products/imports/files/abc/headers"), "fileId")
  })
})

describe("products.startImport / getImport / listImports", () => {
  it("imports valid rows, creates missing categories and reports the bad rows", async () => {
    const { api } = await newTenant("p114-imp-run")
    const file = await upload(
      api,
      [
        "name,sku,price,category,inventory",
        "Blue mug,BM-1,12.5,Kitchen,4",
        ",NONAME,1,Kitchen,1",
        "Red mug,RM-1,abc,Kitchen,1",
        "Green mug,GM-1,3,Garden,0",
      ].join("\n"),
    )
    const started = await api.post<{ importId: string }>("/v1/products/imports", {
      fileId: file.fileId,
      columnMap: { name: "name", sku: "sku", price: "price", category: "category", inventoryQuantity: "inventory" },
    })
    expect(started.status, started.text).toBe(201)
    expect(keysOf(started.json)).toEqual(["importId"])
    const job = await finished(api, started.json.importId)
    expect(keysOf(job)).toEqual(IMPORT_KEYS)
    expect(job).toMatchObject({
      id: started.json.importId,
      status: "completed",
      type: "products",
      fileId: file.fileId,
      fileName: "products.csv",
      userId: null,
      successCount: 2,
      failedCount: 2,
      completedAt: expect.stringMatching(ISO),
    })
    expect(job.errorSample.map((error) => error.reason).sort()).toEqual(["Price must be a number", "Product name is required"])

    const products = await api.get<{ data: Array<{ name: string; sku: string; price: number; inventoryQuantity: number }> }>(
      "/v1/products",
    )
    expect(
      products.json.data
        .map((product) => [product.name, product.sku, product.price, product.inventoryQuantity])
        .sort((a, b) => String(a[0]).localeCompare(String(b[0]))),
    ).toEqual([
      ["Blue mug", "BM-1", 12.5, 4],
      ["Green mug", "GM-1", 3, 0],
    ])
    const categories = await api.get<{ data: Array<{ name: string; productCount: number }> }>("/v1/product-categories")
    expect(categories.json.data.map((category) => [category.name, category.productCount]).sort()).toEqual([
      ["Garden", 1],
      ["Kitchen", 1],
    ])

    const listed = await api.get<{ data: ImportJob[]; pageCount: number }>("/v1/products/imports")
    expect(listed.status, listed.text).toBe(200)
    expect(listed.json.pageCount).toBe(1)
    expect(listed.json.data.map((row) => row.id)).toEqual([started.json.importId])
    expect(listed.json.data[0]).toEqual(job)
  })

  it("guesses the column map from the headers when none is sent; unknown categories fail without createMissingCategories", async () => {
    const { api } = await newTenant("p114-imp-guess")
    const file = await upload(api, "Name,Price,Category\nTea,2,Drinks\nCake,5,\n")
    const started = await api.post<{ importId: string }>("/v1/products/imports", {
      fileId: file.fileId,
      createMissingCategories: false,
    })
    expect(started.status, started.text).toBe(201)
    const job = await finished(api, started.json.importId)
    expect(job).toMatchObject({ status: "completed", successCount: 1, failedCount: 1 })
    expect(job.errorSample).toMatchObject([{ reason: "Unknown category: Drinks" }])
    expect((await api.get<{ data: unknown[] }>("/v1/product-categories")).json.data).toEqual([])
  })

  it("404 for an unknown import or file; 422 for a column map without name", async () => {
    const { api } = await newTenant("p114-imp-errors")
    const { api: other } = await newTenant("p114-imp-errors-other")
    expectNotFound(await api.get(`/v1/products/imports/${UNKNOWN_ID}`), "Import not found")
    const foreignFile = await upload(other, "name\nx\n")
    const foreignImport = await other.post<{ importId: string }>("/v1/products/imports", { fileId: foreignFile.fileId })
    expect(foreignImport.status, foreignImport.text).toBe(201)
    expectNotFound(await api.get(`/v1/products/imports/${foreignImport.json.importId}`), "Import not found")
    const file = await upload(api, "name\nx\n")
    expectInvalid(await api.post("/v1/products/imports", { fileId: file.fileId, columnMap: { sku: "name" } }), "columnMap.name")
    expectInvalid(await api.post("/v1/products/imports", {}), "fileId")
    const unknown = await api.post("/v1/products/imports", { fileId: UNKNOWN_ID, columnMap: { name: "name" } })
    expect([400, 404], unknown.text).toContain(unknown.status)
    expect((await api.get<{ data: unknown[] }>("/v1/products/imports")).json.data).toEqual([])
    await finished(other, foreignImport.json.importId)
  })
})
