// mediaLibrary folders (listFolders / createFolder / renameFolder /
// deleteFolder) and files (createUploadUrl / createFile / getFile / listFiles /
// setFavourite / recordAccess / moveFiles / deleteFile), scope `media`.
// Upload: createUploadUrl returns a 5-minute presigned PUT URL and a storage
// path under `public/space/<workspaceId>/media-library/`; the test PUTs the
// bytes there, registers the path with createFile, then reads the bytes back
// through the public `url` (`http://app/storage/<path>`). Deletes remove the
// storage object too, checked through the same public URL.

import { describe, expect, it } from "vitest"
import { Normalizer } from "../../src/normalize.ts"
import {
  type Api,
  expectError,
  expectInvalid,
  expectNotFound,
  fetchPublic,
  ISO,
  keysOf,
  newTenant,
  PNG_1X1,
  SNOWFLAKE,
  UNKNOWN_ID,
  type UploadedFile,
  uniqueName,
  uploadMediaFile,
  wait,
} from "./helpers.ts"

type Folder = { id: string; name: string; createdAt: string; updatedAt: string; fileCount?: number }
type FileItem = UploadedFile & {
  isFavourite: boolean
  lastAccessedAt: string | null
  folderId: string | null
  createdAt: string
  updatedAt: string
}
type Page<T> = { data: T[]; pageCount: number }

const FOLDER_KEYS = ["createdAt", "id", "name", "updatedAt"]
const FILE_KEYS = [
  "createdAt",
  "folderId",
  "id",
  "isFavourite",
  "lastAccessedAt",
  "mimeType",
  "name",
  "path",
  "size",
  "updatedAt",
  "url",
]

const createFolder = async (api: Api, name: string): Promise<Folder> => {
  const response = await api.post<Folder>("/v1/media-library/folders", { name })
  expect(response.status, response.text).toBe(201)
  return response.json
}

const listFolders = async (api: Api, query: Record<string, number> = {}): Promise<Page<Folder>> => {
  const response = await api.get<Page<Folder>>("/v1/media-library/folders", { query })
  expect(response.status, response.text).toBe(200)
  return response.json
}

const listFiles = async (
  api: Api,
  query: Record<string, string | number | undefined> = {},
): Promise<Page<FileItem>> => {
  const response = await api.get<Page<FileItem>>("/v1/media-library/files", { query })
  expect(response.status, response.text).toBe(200)
  return response.json
}

const getFile = async (api: Api, id: string): Promise<FileItem> => {
  const response = await api.get<FileItem>(`/v1/media-library/files/${id}`)
  expect(response.status, response.text).toBe(200)
  return response.json
}

/** A file row on a workspace path that has no stored object (createFile only checks the prefix). */
const registerPath = async (
  api: Api,
  workspaceId: string,
  input: { name: string; mimeType?: string; folderId?: string | null },
): Promise<FileItem> => {
  const response = await api.post<FileItem>("/v1/media-library/files", {
    name: input.name,
    path: `public/space/${workspaceId}/media-library/${uniqueName("x")}`,
    mimeType: input.mimeType ?? "image/png",
    size: 10,
    ...(input.folderId !== undefined ? { folderId: input.folderId } : {}),
  })
  expect(response.status, response.text).toBe(201)
  return response.json
}

describe("mediaLibrary folders", () => {
  it("creates (201), lists by name with file counts, renames (204) and deletes (204)", async () => {
    const { api } = await newTenant("p111-ml-folders")
    const n = new Normalizer()
    expect(await listFolders(api)).toEqual({ data: [], pageCount: 1 })
    const beta = await createFolder(api, "beta")
    expect(keysOf(beta)).toEqual(FOLDER_KEYS)
    expect(n.normalize(beta)).toEqual({ id: "<id:1>", name: "beta", createdAt: "<ts>", updatedAt: "<ts>" })
    const alpha = await createFolder(api, "alpha")
    const gamma = await createFolder(api, "gamma")
    await uploadMediaFile(api, { folderId: beta.id })

    const listed = await listFolders(api)
    expect(listed.pageCount).toBe(1)
    expect(listed.data.map((folder) => [folder.name, folder.fileCount])).toEqual([
      ["alpha", 0],
      ["beta", 1],
      ["gamma", 0],
    ])
    expect(keysOf(listed.data[0])).toEqual([...FOLDER_KEYS, "fileCount"].sort())
    expect(listed.data[1]).toEqual({ ...beta, fileCount: 1 })

    const renamed = await api.put(`/v1/media-library/folders/${alpha.id}`, { name: "zeta" })
    expect(renamed.status, renamed.text).toBe(204)
    expect(renamed.text).toBe("")
    expect((await listFolders(api)).data.map((folder) => folder.name)).toEqual(["beta", "gamma", "zeta"])

    const deleted = await api.delete(`/v1/media-library/folders/${gamma.id}`)
    expect(deleted.status, deleted.text).toBe(204)
    expect((await listFolders(api)).data.map((folder) => folder.id)).toEqual([beta.id, alpha.id])

    // Another workspace sees none of them.
    const { api: otherApi } = await newTenant("p111-ml-folders-o")
    expect((await listFolders(otherApi)).data).toEqual([])
  })

  it("pages 50 folders by default (51 -> 2 pages); perPage above 50 is 422", async () => {
    const { api } = await newTenant("p111-ml-folders-paging")
    for (let index = 0; index < 51; index += 1) {
      await createFolder(api, `F${String(index).padStart(2, "0")}`)
    }
    const first = await listFolders(api)
    expect(first.pageCount).toBe(2)
    expect(first.data).toHaveLength(50)
    expect(first.data[0]?.name).toBe("F00")
    const second = await listFolders(api, { page: 2 })
    expect(second.data.map((folder) => folder.name)).toEqual(["F50"])
    const small = await listFolders(api, { perPage: 20, page: 3 })
    expect(small).toMatchObject({ pageCount: 3 })
    expect(small.data.map((folder) => folder.name)).toEqual(Array.from({ length: 11 }, (_, i) => `F${40 + i}`))
    expect((await listFolders(api, { page: 9 })).data).toEqual([])
    expectInvalid(await api.get("/v1/media-library/folders", { query: { perPage: 51 } }), "perPage")
    expectInvalid(await api.get("/v1/media-library/folders", { query: { page: 0 } }), "page")
  })

  it("deleting a folder deletes its files and their storage objects", async () => {
    const { api } = await newTenant("p111-ml-folder-delete")
    const folder = await createFolder(api, "To delete")
    const file = await uploadMediaFile(api, { folderId: folder.id })
    const root = await uploadMediaFile(api)
    expect((await fetchPublic(file.url)).status).toBe(200)
    expect((await api.delete(`/v1/media-library/folders/${folder.id}`)).status).toBe(204)
    expectNotFound(await api.get(`/v1/media-library/files/${file.id}`), `MediaLibraryFile ${file.id} not found`)
    expect((await fetchPublic(file.url)).status).not.toBe(200)
    // Root files stay.
    expect((await getFile(api, root.id)).id).toBe(root.id)
    expect((await fetchPublic(root.url)).status).toBe(200)
  })

  it("rename / delete of an unknown or foreign folder is 204 and changes nothing; input is validated", async () => {
    const { api } = await newTenant("p111-ml-folder-noop")
    const folder = await createFolder(api, "Mine")
    const { api: otherApi } = await newTenant("p111-ml-folder-noop-o")
    expect((await otherApi.put(`/v1/media-library/folders/${folder.id}`, { name: "Stolen" })).status).toBe(204)
    expect((await otherApi.delete(`/v1/media-library/folders/${folder.id}`)).status).toBe(204)
    expect((await api.put(`/v1/media-library/folders/${UNKNOWN_ID}`, { name: "X" })).status).toBe(204)
    expect((await api.delete(`/v1/media-library/folders/${UNKNOWN_ID}`)).status).toBe(204)
    expect((await listFolders(api)).data.map((item) => item.name)).toEqual(["Mine"])
    expectInvalid(await api.post("/v1/media-library/folders", { name: "" }), "name")
    expectInvalid(await api.post("/v1/media-library/folders", {}), "name")
    expectInvalid(await api.put(`/v1/media-library/folders/${folder.id}`, { name: "" }), "name")
    expectInvalid(await api.put("/v1/media-library/folders/abc", { name: "x" }), "folderId")
    expectInvalid(await api.delete("/v1/media-library/folders/abc"), "folderId")
  })
})

describe("mediaLibrary upload + files", () => {
  it("createUploadUrl -> PUT -> createFile; getFile returns the file; the public url serves the bytes", async () => {
    const { tenant, api } = await newTenant("p111-ml-upload")
    const n = new Normalizer()
    const presigned = await api.post<{ path: string; uploadUrl: string; publicUrl: string }>(
      "/v1/media-library/files/upload-url",
      { fileName: "menu.png", mimeType: "image/png" },
    )
    expect(presigned.status, presigned.text).toBe(201)
    expect(keysOf(presigned.json)).toEqual(["path", "publicUrl", "uploadUrl"])
    const prefix = `public/space/${tenant.workspaceId}/media-library/`
    expect(presigned.json.path).toMatch(new RegExp(`^${prefix}\\d{15,20}$`))
    expect(presigned.json.publicUrl).toBe(`http://app/storage/${presigned.json.path}`)
    expect(new URL(presigned.json.uploadUrl).searchParams.get("X-Amz-Expires")).toBe("300")
    expect((await fetch(presigned.json.uploadUrl, { method: "PUT", body: PNG_1X1 })).status).toBe(200)

    const created = await api.post<FileItem>("/v1/media-library/files", {
      name: "menu.png",
      path: presigned.json.path,
      mimeType: "image/png",
      size: PNG_1X1.length,
    })
    expect(created.status, created.text).toBe(201)
    expect(keysOf(created.json)).toEqual(FILE_KEYS)
    expect(n.normalize(created.json, { keep: ["path", "url"] })).toEqual({
      id: "<id:1>",
      name: "menu.png",
      path: presigned.json.path,
      mimeType: "image/png",
      size: PNG_1X1.length,
      isFavourite: false,
      lastAccessedAt: null,
      folderId: null,
      url: presigned.json.publicUrl,
      createdAt: "<ts>",
      updatedAt: "<ts>",
    })
    expect(await getFile(api, created.json.id)).toEqual(created.json)

    const stored = await fetchPublic(created.json.url)
    expect(stored.status).toBe(200)
    expect(stored.bytes.equals(PNG_1X1)).toBe(true)

    // Another workspace cannot read it.
    const { api: otherApi } = await newTenant("p111-ml-upload-o")
    expectNotFound(await otherApi.get(`/v1/media-library/files/${created.json.id}`), `MediaLibraryFile ${created.json.id} not found`)
  })

  it("createFile only accepts a path under the workspace's own prefix (400 invalidPath)", async () => {
    const { tenant, api } = await newTenant("p111-ml-path")
    const { tenant: other } = await newTenant("p111-ml-path-o")
    const base = { name: "a.png", mimeType: "image/png", size: 1 }
    for (const path of [
      `public/space/${other.workspaceId}/media-library/1`,
      `workspaces/${other.workspaceId}/x`,
      "public/space/1/x",
      "media/x.png",
    ]) {
      expectError(await api.post("/v1/media-library/files", { ...base, path }), 400, "invalidPath", {
        message: "Invalid file path",
      })
    }
    const legacy = await api.post<FileItem>("/v1/media-library/files", { ...base, path: `workspaces/${tenant.workspaceId}/a.png` })
    expect(legacy.status, legacy.text).toBe(201)
    expect(legacy.json.url).toBe(`http://app/storage/workspaces/${tenant.workspaceId}/a.png`)
    expectInvalid(await api.post("/v1/media-library/files", { name: "a", path: "x", mimeType: "image/png" }), "size")
    expectInvalid(await api.post("/v1/media-library/files/upload-url", { fileName: "", mimeType: "image/png" }), "fileName")
    expectInvalid(await api.post("/v1/media-library/files/upload-url", { fileName: "a.png" }), "mimeType")
    expectInvalid(await api.get("/v1/media-library/files/abc"), "fileId")
    expectNotFound(await api.get(`/v1/media-library/files/${UNKNOWN_ID}`), `MediaLibraryFile ${UNKNOWN_ID} not found`)
  })

  it("listFiles: root only by default, folderId narrows, all spans folders; search, fileType, newest first", async () => {
    const { tenant, api } = await newTenant("p111-ml-list")
    expect(await listFiles(api)).toEqual({ data: [], pageCount: 0 })
    const folder = await createFolder(api, "Photos")
    const rootImage = await registerPath(api, tenant.workspaceId, { name: "Root-Cat.png" })
    const rootPdf = await registerPath(api, tenant.workspaceId, { name: "menu.pdf", mimeType: "application/pdf" })
    const nested = await registerPath(api, tenant.workspaceId, { name: "nested-cat.jpg", mimeType: "image/jpeg", folderId: folder.id })

    const root = await listFiles(api)
    expect(root.pageCount).toBe(1)
    expect(root.data.map((file) => file.id)).toEqual([rootPdf.id, rootImage.id])
    expect(keysOf(root.data[0])).toEqual(FILE_KEYS)
    expect((await listFiles(api, { folderId: folder.id })).data.map((file) => file.id)).toEqual([nested.id])
    expect((await listFiles(api, { filter: "all" })).data.map((file) => file.id)).toEqual([
      nested.id,
      rootPdf.id,
      rootImage.id,
    ])
    // `search` is a case-insensitive substring of the name, inside the same scope.
    expect((await listFiles(api, { search: "CAT" })).data.map((file) => file.id)).toEqual([rootImage.id])
    expect((await listFiles(api, { search: "cat", filter: "all" })).data.map((file) => file.id)).toEqual([
      nested.id,
      rootImage.id,
    ])
    expect((await listFiles(api, { fileType: "image", filter: "all" })).data.map((file) => file.id)).toEqual([
      nested.id,
      rootImage.id,
    ])
    expect((await listFiles(api, { search: "no-such" })).pageCount).toBe(0)

    const paged = await listFiles(api, { filter: "all", perPage: 2 })
    expect(paged.pageCount).toBe(2)
    expect(paged.data).toHaveLength(2)
    expect((await listFiles(api, { filter: "all", perPage: 2, page: 2 })).data.map((file) => file.id)).toEqual([
      rootImage.id,
    ])
    expectInvalid(await api.get("/v1/media-library/files", { query: { filter: "nope" } }), "filter")
    expectInvalid(await api.get("/v1/media-library/files", { query: { fileType: "video" } }), "fileType")
    expectInvalid(await api.get("/v1/media-library/files", { query: { perPage: 51 } }), "perPage")
    const { api: otherApi } = await newTenant("p111-ml-list-o")
    expect((await listFiles(otherApi, { filter: "all" })).data).toEqual([])
  })

  it("listFiles pages 50 by default: 51 files -> 2 pages", async () => {
    const { tenant, api } = await newTenant("p111-ml-list-paging")
    for (let index = 0; index < 51; index += 1) {
      await registerPath(api, tenant.workspaceId, { name: `f${index}.png` })
    }
    const first = await listFiles(api)
    expect(first.pageCount).toBe(2)
    expect(first.data).toHaveLength(50)
    expect(first.data[0]?.name).toBe("f50.png")
    expect((await listFiles(api, { page: 2 })).data.map((file) => file.name)).toEqual(["f0.png"])
  })

  it("setFavourite + filter favourite (ignores folderId); recordAccess + filter recent (never-accessed first)", async () => {
    const { tenant, api } = await newTenant("p111-ml-fav")
    const folder = await createFolder(api, "Folder")
    const a = await registerPath(api, tenant.workspaceId, { name: "a.png" })
    const b = await registerPath(api, tenant.workspaceId, { name: "b.png", folderId: folder.id })
    const c = await registerPath(api, tenant.workspaceId, { name: "c.png" })

    const fav = await api.put<FileItem>(`/v1/media-library/files/${b.id}/favourite`, { isFavourite: true })
    expect(fav.status, fav.text).toBe(200)
    expect(fav.json).toEqual({ ...b, isFavourite: true })
    expect((await getFile(api, b.id)).isFavourite).toBe(true)
    expect((await listFiles(api, { filter: "favourite" })).data.map((file) => file.id)).toEqual([b.id])
    expect((await listFiles(api, { filter: "favourite", folderId: UNKNOWN_ID })).data.map((file) => file.id)).toEqual([
      b.id,
    ])
    const unfav = await api.put<FileItem>(`/v1/media-library/files/${b.id}/favourite`, { isFavourite: false })
    expect(unfav.json.isFavourite).toBe(false)
    expect((await listFiles(api, { filter: "favourite" })).data).toEqual([])

    const before = Date.now()
    const touched = await api.post(`/v1/media-library/files/${a.id}/access`)
    expect(touched.status, touched.text).toBe(204)
    const accessed = await getFile(api, a.id)
    expect(accessed.lastAccessedAt).toMatch(/^\d{4}-\d{2}-\d{2}T/)
    expect(Date.parse(accessed.lastAccessedAt as string)).toBeGreaterThanOrEqual(before - 5_000)
    // `recent` sorts by lastAccessedAt descending, so files never accessed come first.
    const recent = (await listFiles(api, { filter: "recent" })).data.map((file) => file.id)
    expect(recent).toHaveLength(3)
    expect(recent.at(-1)).toBe(a.id)
    expect(new Set(recent.slice(0, 2))).toEqual(new Set([b.id, c.id]))

    expectNotFound(
      await api.put(`/v1/media-library/files/${UNKNOWN_ID}/favourite`, { isFavourite: true }),
      `MediaLibraryFile ${UNKNOWN_ID} not found`,
    )
    expectInvalid(await api.put(`/v1/media-library/files/${a.id}/favourite`, {}), "isFavourite")
    // recordAccess on an unknown file is a silent 204.
    expect((await api.post(`/v1/media-library/files/${UNKNOWN_ID}/access`)).status).toBe(204)
  })

  it("moveFiles moves into a folder and back to root (null); unknown ids are ignored", async () => {
    const { tenant, api } = await newTenant("p111-ml-move")
    const folder = await createFolder(api, "Target")
    const a = await registerPath(api, tenant.workspaceId, { name: "a.png" })
    const b = await registerPath(api, tenant.workspaceId, { name: "b.png" })
    const moved = await api.patch("/v1/media-library/files/move", { fileIds: [a.id, b.id, UNKNOWN_ID], folderId: folder.id })
    expect(moved.status, moved.text).toBe(204)
    expect((await listFiles(api)).data).toEqual([])
    expect((await listFiles(api, { folderId: folder.id })).data.map((file) => file.id).sort()).toEqual([a.id, b.id].sort())
    expect((await getFile(api, a.id)).folderId).toBe(folder.id)
    expect((await listFolders(api)).data[0]?.fileCount).toBe(2)

    expect((await api.patch("/v1/media-library/files/move", { fileIds: [a.id], folderId: null })).status).toBe(204)
    expect((await listFiles(api)).data.map((file) => file.id)).toEqual([a.id])
    expect((await api.patch("/v1/media-library/files/move", { fileIds: [b.id] })).status).toBe(204)
    expect((await getFile(api, b.id)).folderId).toBeNull()

    // Another workspace's move does not touch these files.
    const { api: otherApi } = await newTenant("p111-ml-move-o")
    expect((await otherApi.patch("/v1/media-library/files/move", { fileIds: [a.id], folderId: null })).status).toBe(204)
    expectInvalid(await api.patch("/v1/media-library/files/move", { fileIds: [] }), "fileIds")
    expectInvalid(await api.patch("/v1/media-library/files/move", { fileIds: ["abc"] }), "fileIds.0")
  })

  it("deleteFile removes the row and the storage object; a second delete is 404", async () => {
    const { api } = await newTenant("p111-ml-delete")
    const file = await uploadMediaFile(api)
    expect((await fetchPublic(file.url)).status).toBe(200)
    const response = await api.delete(`/v1/media-library/files/${file.id}`)
    expect(response.status, response.text).toBe(204)
    expect(response.text).toBe("")
    expectNotFound(await api.get(`/v1/media-library/files/${file.id}`), `MediaLibraryFile ${file.id} not found`)
    await wait(async () => expect((await fetchPublic(file.url)).status).not.toBe(200), 10_000)
    expectNotFound(await api.delete(`/v1/media-library/files/${file.id}`), `MediaLibraryFile ${file.id} not found`)
    const { api: otherApi } = await newTenant("p111-ml-delete-o")
    const kept = await uploadMediaFile(api)
    expectNotFound(await otherApi.delete(`/v1/media-library/files/${kept.id}`), `MediaLibraryFile ${kept.id} not found`)
    expect((await fetchPublic(kept.url)).status).toBe(200)
    expect(String(kept.id)).toMatch(SNOWFLAKE)
    expect(new Date().toISOString()).toMatch(ISO)
  })
})
