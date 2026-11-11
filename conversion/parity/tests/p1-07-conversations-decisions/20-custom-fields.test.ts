// customFields.create / list / get / update / delete (scope `contacts`).
// Side effects are read back through the same namespace (get by id and by
// name, list filters) and the folders API.

import { describe, expect, it } from "vitest"
import { Normalizer } from "../../src/normalize.ts"
import { uniqueId } from "../../src/unique.ts"
import { type CustomField, createField, createFolder, expectInvalidAt, fieldPath } from "./helpers-decisions.ts"
import { expectError, newTenant, UNKNOWN_ID, uniqueName } from "./helpers.ts"

type FieldList = { data: CustomField[]; pageCount: number }

describe("customFields.create", () => {
  it("creates a root field (201) with the full definition shape", async () => {
    const n = new Normalizer()
    const { api } = await newTenant("p107-cf-create")
    const name = uniqueName("Score")
    const created = await api.post<CustomField>("/v1/custom-fields", {
      name: `  ${name}  `,
      type: "number",
      description: "Lead score",
    })
    expect(created.status, created.text).toBe(201)
    expect(n.normalize(created.json)).toEqual({
      id: "<id:1>",
      name,
      type: "number",
      description: "Lead score",
      folderId: null,
      showInInbox: true,
      createdAt: "<ts>",
      updatedAt: "<ts>",
    })
    expect(Object.keys(created.json).sort()).toEqual(
      ["createdAt", "description", "folderId", "id", "name", "showInInbox", "type", "updatedAt"].sort(),
    )
  })

  it("accepts every field type; description defaults to null; folderId \"0\" is the root", async () => {
    const { api } = await newTenant("p107-cf-types")
    for (const type of ["shortText", "email", "phoneNumber", "number", "date", "datetime", "boolean", "longText"]) {
      const field = await createField(api, { name: uniqueName(type), type, folderId: "0" })
      expect(field).toMatchObject({ type, description: null, folderId: null })
    }
  })

  it("places the field in a customField folder", async () => {
    const { api } = await newTenant("p107-cf-folder")
    const folderId = await createFolder(api, uniqueName("Folder"))
    const field = await createField(api, { name: uniqueName("InFolder"), type: "shortText", folderId })
    expect(field.folderId).toBe(folderId)
  })

  it("rejects an unknown folder, a tag folder and a foreign folder with 404 Folder not found", async () => {
    const { api } = await newTenant("p107-cf-folder-bad")
    const other = await newTenant("p107-cf-folder-other")
    const tagFolder = await createFolder(api, uniqueName("Tags"), "tag")
    const foreign = await createFolder(other.api, uniqueName("Foreign"))
    for (const folderId of [UNKNOWN_ID, tagFolder, foreign]) {
      const response = await api.post("/v1/custom-fields", { name: uniqueName("F"), type: "shortText", folderId })
      expectError(response, 404, "notFound", { defined: true, message: "Folder not found" })
    }
  })

  it("rejects the same name and type twice with 422 validation; another type may reuse the name", async () => {
    const { api } = await newTenant("p107-cf-dup")
    const name = uniqueName("Dup")
    await createField(api, { name, type: "shortText" })
    const again = await api.post("/v1/custom-fields", { name, type: "shortText" })
    expectError(again, 422, "validation", { defined: true, message: "Name is already taken" })
    const otherType = await api.post<CustomField>("/v1/custom-fields", { name, type: "number" })
    expect(otherType.status, otherType.text).toBe(201)
    expect(otherType.json).toMatchObject({ name, type: "number" })
  })

  it("validates the input (422 invalidRequestData)", async () => {
    const { api } = await newTenant("p107-cf-invalid")
    expectInvalidAt(await api.post("/v1/custom-fields", { type: "shortText" }), "name")
    expectInvalidAt(await api.post("/v1/custom-fields", { name: "   ", type: "shortText" }), "name")
    expectInvalidAt(await api.post("/v1/custom-fields", { name: "x".repeat(256), type: "shortText" }), "name")
    expectInvalidAt(await api.post("/v1/custom-fields", { name: "bot_field:7", type: "shortText" }), "name")
    expectInvalidAt(await api.post("/v1/custom-fields", { name: uniqueName("T"), type: "select" }), "type")
    expectInvalidAt(await api.post("/v1/custom-fields", { name: uniqueName("T") }), "type")
    expectInvalidAt(
      await api.post("/v1/custom-fields", { name: uniqueName("T"), type: "shortText", folderId: "abc" }),
      "folderId",
    )
  })
})

describe("customFields.get", () => {
  it("finds a field by id and by exact name (URL-encoded); unknown and foreign keys are 404", async () => {
    const { api } = await newTenant("p107-cf-get")
    const other = await newTenant("p107-cf-get-other")
    const name = `Favourite colour ${uniqueId()}`
    const field = await createField(api, { name, type: "shortText", description: "Colour" })
    const byId = await api.get<CustomField>(fieldPath(field.id))
    expect(byId.status, byId.text).toBe(200)
    expect(byId.json).toEqual(field)
    const byName = await api.get<CustomField>(fieldPath(name))
    expect(byName.status, byName.text).toBe(200)
    expect(byName.json).toEqual(field)

    // Name match is exact (case-sensitive).
    expectError(await api.get(fieldPath(name.toUpperCase())), 404, "notFound", {
      defined: true,
      message: "Custom field not found",
    })
    expectError(await api.get(fieldPath(UNKNOWN_ID)), 404, "notFound", { message: "Custom field not found" })
    expectError(await other.api.get(fieldPath(field.id)), 404, "notFound", { message: "Custom field not found" })
    expectError(await other.api.get(fieldPath(name)), 404, "notFound", { message: "Custom field not found" })
  })

  it("a numeric key that is no field id falls back to a name match", async () => {
    const { api } = await newTenant("p107-cf-numeric")
    const name = String(1000 + Math.floor(Math.random() * 1_000_000))
    const field = await createField(api, { name, type: "number" })
    const got = await api.get<CustomField>(fieldPath(name))
    expect(got.status, got.text).toBe(200)
    expect(got.json.id).toBe(field.id)
  })
})

describe("customFields.list", () => {
  it("lists newest first with pageCount, filters by name substring (case-insensitive) and folder", async () => {
    const n = new Normalizer()
    const { api } = await newTenant("p107-cf-list")
    const empty = await api.get<FieldList>("/v1/custom-fields")
    expect(empty.status, empty.text).toBe(200)
    expect(empty.json).toEqual({ data: [], pageCount: 0 })

    const folderId = await createFolder(api, uniqueName("Folder"))
    const first = await createField(api, { name: "Alpha Parity", type: "shortText" })
    const second = await createField(api, { name: "Beta Parity", type: "number", folderId })
    const third = await createField(api, { name: "Gamma other", type: "longText" })

    const all = await api.get<FieldList>("/v1/custom-fields")
    expect(all.status, all.text).toBe(200)
    expect(all.json.pageCount).toBe(1)
    expect(all.json.data.map((field) => field.id)).toEqual([third.id, second.id, first.id])
    expect(n.normalize(all.json.data[1])).toEqual({
      id: "<id:2>",
      name: "Beta Parity",
      type: "number",
      description: null,
      folderId: "<id:1>",
      showInInbox: true,
      createdAt: "<ts>",
      updatedAt: "<ts>",
    })

    const byName = await api.get<FieldList>("/v1/custom-fields", { query: { name: "parity" } })
    expect(byName.json.data.map((field) => field.id)).toEqual([second.id, first.id])
    const inFolder = await api.get<FieldList>("/v1/custom-fields", { query: { folderId } })
    expect(inFolder.json.data.map((field) => field.id)).toEqual([second.id])
    const atRoot = await api.get<FieldList>("/v1/custom-fields", { query: { folderId: "0" } })
    expect(atRoot.json.data.map((field) => field.id)).toEqual([third.id, first.id])
  })

  it("pages with page/perPage and sorts by name", async () => {
    const { api } = await newTenant("p107-cf-page")
    const names = ["Charlie", "Alpha", "Bravo"]
    for (const name of names) {
      await createField(api, { name, type: "shortText" })
    }
    const page1 = await api.get<FieldList>("/v1/custom-fields", { query: { page: 1, perPage: 2 } })
    expect(page1.status, page1.text).toBe(200)
    expect(page1.json.pageCount).toBe(2)
    expect(page1.json.data.map((field) => field.name)).toEqual(["Bravo", "Alpha"])
    const page2 = await api.get<FieldList>("/v1/custom-fields", { query: { page: 2, perPage: 2 } })
    expect(page2.json.data.map((field) => field.name)).toEqual(["Charlie"])

    const sorted = await api.get<FieldList>("/v1/custom-fields?sort[0][id]=name&sort[0][desc]=false")
    expect(sorted.status, sorted.text).toBe(200)
    expect(sorted.json.data.map((field) => field.name)).toEqual(["Alpha", "Bravo", "Charlie"])
    const desc = await api.get<FieldList>("/v1/custom-fields?sort[0][id]=name&sort[0][desc]=true")
    expect(desc.json.data.map((field) => field.name)).toEqual(["Charlie", "Bravo", "Alpha"])
  })

  it("validates paging and sort (422)", async () => {
    const { api } = await newTenant("p107-cf-list-invalid")
    expectInvalidAt(await api.get("/v1/custom-fields", { query: { perPage: 51 } }), "perPage")
    expectInvalidAt(await api.get("/v1/custom-fields", { query: { page: 0 } }), "page")
    expectError(await api.get("/v1/custom-fields?sort[0][id]=type&sort[0][desc]=false"), 422, "invalidRequestData")
    expectInvalidAt(await api.get("/v1/custom-fields", { query: { folderId: "abc" } }), "folderId")
  })
})

describe("customFields.update", () => {
  it("renames and redescribes; the old name stops resolving, the new one resolves", async () => {
    const n = new Normalizer()
    const { api } = await newTenant("p107-cf-update")
    const oldName = uniqueName("Old")
    const field = await createField(api, { name: oldName, type: "shortText", description: "before" })
    // Warm the by-name lookup, so the rename must invalidate it.
    expect((await api.get(fieldPath(oldName))).status).toBe(200)
    const newName = uniqueName("New")
    const updated = await api.put<CustomField>(fieldPath(field.id), { name: newName, description: "after" })
    expect(updated.status, updated.text).toBe(200)
    expect(n.normalize(updated.json)).toEqual({
      id: "<id:1>",
      name: newName,
      type: "shortText",
      description: "after",
      folderId: null,
      showInInbox: true,
      createdAt: "<ts>",
      updatedAt: "<ts>",
    })
    expect(updated.json.createdAt).toBe(field.createdAt)
    expectError(await api.get(fieldPath(oldName)), 404, "notFound")
    expect((await api.get<CustomField>(fieldPath(newName))).json.id).toBe(field.id)
    expect((await api.get<CustomField>(fieldPath(field.id))).json).toEqual(updated.json)
  })

  it("folderId: omitted keeps the folder, a folder moves it, null moves it to the root", async () => {
    const { api } = await newTenant("p107-cf-move")
    const folderA = await createFolder(api, uniqueName("A"))
    const folderB = await createFolder(api, uniqueName("B"))
    const field = await createField(api, { name: uniqueName("Mover"), type: "shortText", folderId: folderA })
    const kept = await api.put<CustomField>(fieldPath(field.id), { name: field.name })
    expect(kept.json.folderId).toBe(folderA)
    const moved = await api.put<CustomField>(fieldPath(field.id), { name: field.name, folderId: folderB })
    expect(moved.json.folderId).toBe(folderB)
    const root = await api.put<CustomField>(fieldPath(field.id), { name: field.name, folderId: null })
    expect(root.json.folderId).toBeNull()
    const unknown = await api.put(fieldPath(field.id), { name: field.name, folderId: UNKNOWN_ID })
    expectError(unknown, 404, "notFound", { message: "Folder not found" })
  })

  it("a description left out keeps the stored one", async () => {
    const { api } = await newTenant("p107-cf-desc")
    const field = await createField(api, { name: uniqueName("Desc"), type: "shortText", description: "keep me" })
    const updated = await api.put<CustomField>(fieldPath(field.id), { name: field.name })
    expect(updated.status, updated.text).toBe(200)
    expect(updated.json.description).toBe("keep me")
  })

  it("refuses a name taken by a field of the same type (422 validation)", async () => {
    const { api } = await newTenant("p107-cf-clash")
    const taken = await createField(api, { name: uniqueName("Taken"), type: "shortText" })
    const field = await createField(api, { name: uniqueName("Mine"), type: "shortText" })
    const clash = await api.put(fieldPath(field.id), { name: taken.name })
    expectError(clash, 422, "validation", { defined: true, message: "Name is already taken" })
  })

  it("is 404 for an unknown or foreign id and 422 for bad input", async () => {
    const { api } = await newTenant("p107-cf-update-bad")
    const other = await newTenant("p107-cf-update-other")
    const field = await createField(api, { name: uniqueName("Owned"), type: "shortText" })
    expectError(await api.put(fieldPath(UNKNOWN_ID), { name: "x" }), 404, "notFound", {
      defined: true,
      message: "Custom field not found",
    })
    expectError(await other.api.put(fieldPath(field.id), { name: "x" }), 404, "notFound")
    expectInvalidAt(await api.put(fieldPath(field.id), {}), "name")
    expectInvalidAt(await api.put(fieldPath(field.id), { name: "bot_field:1" }), "name")
    expectInvalidAt(await api.put(fieldPath("abc"), { name: "x" }), "id")
    // The type cannot change: an extra `type` key is ignored.
    const typed = await api.put<CustomField>(fieldPath(field.id), { name: field.name, type: "number" })
    expect(typed.status, typed.text).toBe(200)
    expect(typed.json.type).toBe("shortText")
  })
})

describe("customFields.delete", () => {
  it("deletes (204, empty body); get is 404 after; a second delete is still 204", async () => {
    const { api } = await newTenant("p107-cf-delete")
    const field = await createField(api, { name: uniqueName("Gone"), type: "shortText" })
    expect((await api.get(fieldPath(field.name))).status).toBe(200)
    const deleted = await api.delete(fieldPath(field.id))
    expect(deleted.status, deleted.text).toBe(204)
    expect(deleted.text).toBe("")
    expectError(await api.get(fieldPath(field.id)), 404, "notFound")
    expectError(await api.get(fieldPath(field.name)), 404, "notFound")
    expect((await api.get<FieldList>("/v1/custom-fields")).json.data).toEqual([])
    expect((await api.delete(fieldPath(field.id))).status).toBe(204)
    expect((await api.delete(fieldPath(UNKNOWN_ID))).status).toBe(204)
    expectInvalidAt(await api.delete(fieldPath("abc")), "id")
  })

  it("a foreign workspace's delete is 204 and leaves the field in place", async () => {
    const { api } = await newTenant("p107-cf-delete-owner")
    const other = await newTenant("p107-cf-delete-other")
    const field = await createField(api, { name: uniqueName("Safe"), type: "shortText" })
    expect((await other.api.delete(fieldPath(field.id))).status).toBe(204)
    expect((await api.get<CustomField>(fieldPath(field.id))).json).toEqual(field)
  })
})
