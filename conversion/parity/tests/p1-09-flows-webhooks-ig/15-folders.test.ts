// folders.* (scope `contacts`): list / create / update / delete, restricted
// to the contacts folder types `tag` and `customField`. A folder of another
// type (here `flow`, written by the fixture extension) reads as 404 to
// update/delete, and the list/create enums refuse it (422).

import { describe, expect, it } from "vitest"
import { Normalizer } from "../../src/normalize.ts"
import { expectError, expectInvalid, ext, newTenant, uniqueName } from "./helpers.ts"

type Folder = {
  id: string
  name: string
  folderType: string
  parentId: string | null
  workspaceId: string
  isTrash: boolean
  paths: string[]
  createdAt: string
  updatedAt: string
}

const FOLDER_KEYS = ["createdAt", "folderType", "id", "isTrash", "name", "parentId", "paths", "updatedAt", "workspaceId"]

const UNKNOWN_ID = "123456789012345"

describe("folders.create", () => {
  it("creates a top-level folder and returns the full row (200)", async () => {
    const { tenant, api } = await newTenant("p109-folder-create")
    const normalizer = new Normalizer()
    const name = uniqueName("Tags")
    const response = await api.post<Folder>("/v1/folders", { name: `  ${name}  `, folderType: "tag" })
    expect(response.status, response.text).toBe(200)
    expect(Object.keys(response.json).sort()).toEqual(FOLDER_KEYS)
    expect(normalizer.normalize(response.json, { replace: { [tenant.workspaceId]: "<workspace>" } })).toEqual({
      id: "<id:1>",
      name,
      folderType: "tag",
      parentId: null,
      workspaceId: "<workspace>",
      isTrash: false,
      paths: [],
      createdAt: "<ts>",
      updatedAt: "<ts>",
    })
  })

  it("creates a sub-folder whose paths hold the parent id; parentId null or omitted = top level", async () => {
    const { api } = await newTenant("p109-folder-child")
    const parent = await api.post<Folder>("/v1/folders", { name: "Parent", folderType: "customField" })
    expect(parent.status).toBe(200)
    const child = await api.post<Folder>("/v1/folders", {
      name: "Child",
      folderType: "customField",
      parentId: parent.json.id,
    })
    expect(child.status, child.text).toBe(200)
    expect(child.json).toMatchObject({ parentId: parent.json.id, paths: [parent.json.id], folderType: "customField" })
    const grandChild = await api.post<Folder>("/v1/folders", {
      name: "Grandchild",
      folderType: "customField",
      parentId: child.json.id,
    })
    expect(grandChild.json).toMatchObject({ parentId: child.json.id, paths: [parent.json.id, child.json.id] })
    const explicitNull = await api.post<Folder>("/v1/folders", { name: "Top", folderType: "customField", parentId: null })
    expect(explicitNull.json).toMatchObject({ parentId: null, paths: [] })
  })

  it("parent of another type, of another workspace or unknown -> 404 notFound", async () => {
    const { api } = await newTenant("p109-folder-parent")
    const { api: otherApi } = await newTenant("p109-folder-parent-other")
    const tagFolder = await api.post<Folder>("/v1/folders", { name: "Tag parent", folderType: "tag" })
    const foreign = await otherApi.post<Folder>("/v1/folders", { name: "Foreign", folderType: "tag" })
    expectError(
      await api.post("/v1/folders", { name: "x", folderType: "customField", parentId: tagFolder.json.id }),
      404,
      "notFound",
      { message: "Parent folder does not exist!" },
    )
    expectError(await api.post("/v1/folders", { name: "x", folderType: "tag", parentId: foreign.json.id }), 404, "notFound")
    expectError(await api.post("/v1/folders", { name: "x", folderType: "tag", parentId: UNKNOWN_ID }), 404, "notFound")
  })

  it("validates name (trimmed 1-255) and folderType (tag | customField) -> 422", async () => {
    const { api } = await newTenant("p109-folder-bad")
    expectInvalid(await api.post("/v1/folders", { folderType: "tag" }), "name")
    expectInvalid(await api.post("/v1/folders", { name: "   ", folderType: "tag" }), "name")
    expectInvalid(await api.post("/v1/folders", { name: "x".repeat(256), folderType: "tag" }), "name")
    expectInvalid(await api.post("/v1/folders", { name: "x", folderType: "flow" }), "folderType")
    expectInvalid(await api.post("/v1/folders", { name: "x" }), "folderType")
  })
})

describe("folders.list", () => {
  it("lists every folder of one type (sub-folders too), oldest first; parentId narrows, \"0\" = top level", async () => {
    const { tenant, api } = await newTenant("p109-folder-list")
    const first = (await api.post<Folder>("/v1/folders", { name: "B first", folderType: "tag" })).json
    const second = (await api.post<Folder>("/v1/folders", { name: "A second", folderType: "tag" })).json
    const child = (await api.post<Folder>("/v1/folders", { name: "Child", folderType: "tag", parentId: first.id })).json
    await api.post<Folder>("/v1/folders", { name: "Other type", folderType: "customField" })
    await (await ext()).createFolder({ workspaceId: tenant.workspaceId, name: "Flow folder", folderType: "flow" })

    const top = await api.get<{ data: Folder[] }>("/v1/folders?folderType=tag")
    expect(top.status, top.text).toBe(200)
    expect(Object.keys(top.json)).toEqual(["data"])
    // Without parentId the list is NOT limited to the top level (the route
    // description says it is): sub-folders are listed too.
    expect(top.json.data.map((folder) => folder.id)).toEqual([first.id, second.id, child.id])
    expect(top.json.data[0]).toEqual(first)
    expect(top.json.data[2]).toEqual(child)

    const children = await api.get<{ data: Folder[] }>(`/v1/folders?folderType=tag&parentId=${first.id}`)
    expect(children.json.data.map((folder) => folder.id)).toEqual([child.id])

    // "0" is the root folder id: only the top-level folders.
    const root = await api.get<{ data: Folder[] }>("/v1/folders?folderType=tag&parentId=0")
    expect(root.json.data.map((folder) => folder.id)).toEqual([first.id, second.id])

    const fields = await api.get<{ data: Folder[] }>("/v1/folders?folderType=customField")
    expect(fields.json.data.map((folder) => folder.name)).toEqual(["Other type"])
  })

  it("an empty workspace lists nothing; another workspace's folders are not listed", async () => {
    const { api } = await newTenant("p109-folder-list-empty")
    const { api: otherApi } = await newTenant("p109-folder-list-other")
    await otherApi.post("/v1/folders", { name: "Not mine", folderType: "tag" })
    const response = await api.get("/v1/folders?folderType=tag")
    expect(response.status).toBe(200)
    expect(response.json).toEqual({ data: [] })
  })

  it("needs folderType tag | customField (422)", async () => {
    const { api } = await newTenant("p109-folder-list-bad")
    expectInvalid(await api.get("/v1/folders"), "folderType")
    expectInvalid(await api.get("/v1/folders?folderType=flow"), "folderType")
  })
})

describe("folders.update", () => {
  it("renames a folder and returns the updated row (200)", async () => {
    const { api } = await newTenant("p109-folder-update")
    const folder = (await api.post<Folder>("/v1/folders", { name: "Old", folderType: "tag" })).json
    const response = await api.put<Folder>(`/v1/folders/${folder.id}`, { name: " New name " })
    expect(response.status, response.text).toBe(200)
    expect(response.json).toEqual({ ...folder, name: "New name", updatedAt: expect.any(String) })
    const list = await api.get<{ data: Folder[] }>("/v1/folders?folderType=tag")
    expect(list.json.data.map((item) => item.name)).toEqual(["New name"])
  })

  it("a folder of a non-contacts type, of another workspace or unknown -> 404 notFound", async () => {
    const { tenant, api } = await newTenant("p109-folder-update-404")
    const { api: otherApi } = await newTenant("p109-folder-update-other")
    const { folderId } = await (await ext()).createFolder({
      workspaceId: tenant.workspaceId,
      name: "Flow folder",
      folderType: "flow",
    })
    const foreign = (await otherApi.post<Folder>("/v1/folders", { name: "Foreign", folderType: "tag" })).json
    expectError(await api.put(`/v1/folders/${folderId}`, { name: "x" }), 404, "notFound", { message: "Folder not found" })
    expectError(await api.put(`/v1/folders/${foreign.id}`, { name: "x" }), 404, "notFound")
    expectError(await api.put(`/v1/folders/${UNKNOWN_ID}`, { name: "x" }), 404, "notFound")
  })

  it("validates the id and name (422)", async () => {
    const { api } = await newTenant("p109-folder-update-bad")
    const folder = (await api.post<Folder>("/v1/folders", { name: "Old", folderType: "tag" })).json
    expectInvalid(await api.put(`/v1/folders/${folder.id}`, { name: "" }), "name")
    expectInvalid(await api.put(`/v1/folders/${folder.id}`, {}), "name")
    expectInvalid(await api.put("/v1/folders/abc", { name: "x" }), "id")
  })
})

describe("folders.delete", () => {
  it("deletes the folder and its sub-folders (204); later reads are 404", async () => {
    const { api } = await newTenant("p109-folder-delete")
    const parent = (await api.post<Folder>("/v1/folders", { name: "Parent", folderType: "tag" })).json
    const child = (await api.post<Folder>("/v1/folders", { name: "Child", folderType: "tag", parentId: parent.id })).json
    const sibling = (await api.post<Folder>("/v1/folders", { name: "Sibling", folderType: "tag" })).json
    const response = await api.delete(`/v1/folders/${parent.id}`)
    expect(response.status, response.text).toBe(204)
    expect(response.text).toBe("")
    const list = await api.get<{ data: Folder[] }>("/v1/folders?folderType=tag")
    expect(list.json.data.map((folder) => folder.id)).toEqual([sibling.id])
    expectError(await api.put(`/v1/folders/${child.id}`, { name: "x" }), 404, "notFound")
    expectError(await api.delete(`/v1/folders/${parent.id}`), 404, "notFound")
  })

  it("keeps the folder's tags, moved to the root (folderId null)", async () => {
    const { api } = await newTenant("p109-folder-delete-tags")
    const folder = (await api.post<Folder>("/v1/folders", { name: "Tag folder", folderType: "tag" })).json
    const tag = await api.post<{ id: string; name: string; folderId: string | null }>("/v1/tags", {
      name: uniqueName("tag"),
      folderId: folder.id,
    })
    expect(tag.status, tag.text).toBe(201)
    expect(tag.json.folderId).toBe(folder.id)
    expect((await api.delete(`/v1/folders/${folder.id}`)).status).toBe(204)
    const after = await api.get<{ id: string; folderId: string | null }>(`/v1/tags/${tag.json.id}`)
    expect(after.status, after.text).toBe(200)
    expect(after.json).toMatchObject({ id: tag.json.id, name: tag.json.name, folderId: null })
  })

  it("a folder of a non-contacts type, of another workspace or unknown -> 404 notFound", async () => {
    const { tenant, api } = await newTenant("p109-folder-delete-404")
    const { api: otherApi } = await newTenant("p109-folder-delete-other")
    const { folderId } = await (await ext()).createFolder({
      workspaceId: tenant.workspaceId,
      name: "Flow folder",
      folderType: "flow",
    })
    const foreign = (await otherApi.post<Folder>("/v1/folders", { name: "Foreign", folderType: "tag" })).json
    expectError(await api.delete(`/v1/folders/${folderId}`), 404, "notFound")
    expectError(await api.delete(`/v1/folders/${foreign.id}`), 404, "notFound")
    expectError(await api.delete(`/v1/folders/${UNKNOWN_ID}`), 404, "notFound")
    // The foreign folder is still there for its owner.
    const list = await otherApi.get<{ data: Folder[] }>("/v1/folders?folderType=tag")
    expect(list.json.data.map((folder) => folder.id)).toEqual([foreign.id])
    expectInvalid(await api.delete("/v1/folders/abc"), "id")
  })
})
