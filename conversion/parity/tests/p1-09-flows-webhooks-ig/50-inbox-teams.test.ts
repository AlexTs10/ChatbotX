// inboxTeams.*: inbox teams and their members (`/v1/teams`). The module lives
// under `enterprise/` but the public router serves it on every edition; this
// file runs against the community edition. Extra workspace members (agents)
// come from the P1.9 fixture extension.

import { describe, expect, it } from "vitest"
import type { Tenant } from "../../src/fixtures/index.ts"
import {
  type Api,
  addAgent,
  createOutsider,
  expectError,
  expectInvalid,
  ISO,
  SNOWFLAKE,
  tenantApi,
  UNKNOWN_ID,
  uniqueName,
} from "./helpers-ig.ts"

type Team = { id: string; name: string; workspaceId: string; createdAt: string; updatedAt: string }
type ListedTeam = Team & {
  inboxTeamMembers: Array<{
    id: string
    inboxTeamId: string
    userId: string
    createdAt: string
    updatedAt: string
    user: Record<string, unknown>
  }>
}

const TEAM_KEYS = ["createdAt", "id", "name", "updatedAt", "workspaceId"]
const USER_KEYS = [
  "createdAt",
  "email",
  "emailVerified",
  "id",
  "image",
  "isAnonymous",
  "mustChangePassword",
  "name",
  "tenantId",
  "updatedAt",
]
const NOT_MEMBERS = "One or more userIds are not members of this workspace"
const NOT_FOUND = { message: "Inbox team not found" }

const createTeam = async (api: Api, userIds: string[] = [], name = uniqueName("team")): Promise<Team> => {
  const response = await api.post<Team>("/v1/teams", { name, userIds })
  expect(response.status, response.text).toBe(201)
  return response.json
}

const listTeams = async (api: Api, query: Record<string, number> = {}) => {
  const response = await api.get<{ data: ListedTeam[]; pageCount: number }>("/v1/teams", { query })
  expect(response.status, response.text).toBe(200)
  return response.json
}

const memberIds = (team: ListedTeam | undefined): string[] =>
  (team?.inboxTeamMembers ?? []).map((member) => member.userId).sort()

describe("inboxTeams.create", () => {
  it("creates a team (201) with members; list shows them with their users", async () => {
    const { tenant, api } = await tenantApi("p109-team-create")
    const agent = await addAgent(tenant)
    const name = uniqueName("Support")
    const response = await api.post<Team>("/v1/teams", { name: `  ${name}  `, userIds: [tenant.userId, agent.userId] })
    expect(response.status, response.text).toBe(201)
    expect(Object.keys(response.json).sort()).toEqual(TEAM_KEYS)
    expect(response.json).toEqual({
      id: expect.stringMatching(SNOWFLAKE),
      name,
      workspaceId: tenant.workspaceId,
      createdAt: expect.stringMatching(ISO),
      updatedAt: expect.stringMatching(ISO),
    })

    const list = await listTeams(api)
    expect(list.pageCount).toBe(1)
    expect(list.data).toHaveLength(1)
    const [listed] = list.data as [ListedTeam]
    expect({ ...listed, inboxTeamMembers: undefined }).toEqual({ ...response.json, inboxTeamMembers: undefined })
    expect(memberIds(listed)).toEqual([tenant.userId, agent.userId].sort())
    for (const member of listed.inboxTeamMembers) {
      expect(Object.keys(member).sort()).toEqual(["createdAt", "id", "inboxTeamId", "updatedAt", "user", "userId"])
      expect(member).toMatchObject({
        id: expect.stringMatching(SNOWFLAKE),
        inboxTeamId: response.json.id,
        createdAt: expect.stringMatching(ISO),
      })
      expect(Object.keys(member.user).sort()).toEqual(USER_KEYS)
      expect(member.user).toMatchObject({
        id: member.userId,
        email: member.userId === tenant.userId ? tenant.email : agent.email,
        emailVerified: true,
        isAnonymous: false,
        mustChangePassword: false,
        image: null,
        tenantId: "1",
      })
    }
  })

  it("a team with no members is fine", async () => {
    const { api } = await tenantApi("p109-team-empty")
    const team = await createTeam(api)
    const list = await listTeams(api)
    expect(list.data.map((item) => [item.id, item.inboxTeamMembers])).toEqual([[team.id, []]])
  })

  it("refuses a user who is not a member of the workspace (400 invalidTeamMember) and writes nothing", async () => {
    const { tenant, api } = await tenantApi("p109-team-outsider")
    const outsider = await createOutsider()
    expectError(
      await api.post("/v1/teams", { name: uniqueName("team"), userIds: [tenant.userId, outsider.userId] }),
      400,
      "invalidTeamMember",
      { message: NOT_MEMBERS },
    )
    // A member of another workspace is not a member here either.
    const { tenant: other } = await tenantApi("p109-team-outsider-o")
    expectError(await api.post("/v1/teams", { name: uniqueName("team"), userIds: [other.userId] }), 400, "invalidTeamMember")
    expect((await listTeams(api)).data).toEqual([])
  })

  it("validates the body (422 invalidRequestData)", async () => {
    const { api } = await tenantApi("p109-team-invalid")
    expectInvalid(await api.post("/v1/teams", { name: "   ", userIds: [] }), "name")
    expectInvalid(await api.post("/v1/teams", { name: "x".repeat(256), userIds: [] }), "name")
    expectInvalid(await api.post("/v1/teams", { name: "Team" }), "userIds")
    expectInvalid(await api.post("/v1/teams", { name: "Team", userIds: ["abc"] }), "userIds.0")
  })
})

describe("inboxTeams.list / get", () => {
  it("lists teams oldest first and pages in memory", async () => {
    const { api } = await tenantApi("p109-team-list")
    const a = await createTeam(api, [], "Team A")
    const b = await createTeam(api, [], "Team B")
    const c = await createTeam(api, [], "Team C")
    expect((await listTeams(api)).data.map((team) => team.id)).toEqual([a.id, b.id, c.id])
    const page2 = await listTeams(api, { perPage: 2, page: 2 })
    expect(page2.pageCount).toBe(2)
    expect(page2.data.map((team) => team.id)).toEqual([c.id])
    const page3 = await listTeams(api, { perPage: 2, page: 3 })
    expect(page3).toEqual({ data: [], pageCount: 2 })
    expectInvalid(await api.get("/v1/teams", { query: { perPage: 51 } }), "perPage")
  })

  it("get returns the team without members; 404 for unknown and foreign ids", async () => {
    const { tenant, api } = await tenantApi("p109-team-get")
    const team = await createTeam(api, [tenant.userId])
    const response = await api.get<Team>(`/v1/teams/${team.id}`)
    expect(response.status).toBe(200)
    expect(response.json).toEqual(team)
    expectError(await api.get(`/v1/teams/${UNKNOWN_ID}`), 404, "notFound", NOT_FOUND)
    const { api: otherApi } = await tenantApi("p109-team-get-o")
    const foreign = await createTeam(otherApi)
    expectError(await api.get(`/v1/teams/${foreign.id}`), 404, "notFound", NOT_FOUND)
    expectInvalid(await api.get("/v1/teams/abc"), "id")
  })
})

describe("inboxTeams.update / delete", () => {
  it("update renames; 404 unknown / foreign; 422 bad name", async () => {
    const { api } = await tenantApi("p109-team-update")
    const team = await createTeam(api)
    const name = uniqueName("Renamed")
    const response = await api.patch<Team>(`/v1/teams/${team.id}`, { name })
    expect(response.status, response.text).toBe(200)
    expect(response.json).toEqual({ ...team, name, updatedAt: expect.stringMatching(ISO) })
    expect((await api.get<Team>(`/v1/teams/${team.id}`)).json.name).toBe(name)
    expect((await listTeams(api)).data.map((item) => item.name)).toEqual([name])

    expectError(await api.patch(`/v1/teams/${UNKNOWN_ID}`, { name: "x" }), 404, "notFound", NOT_FOUND)
    const { api: otherApi } = await tenantApi("p109-team-update-o")
    const foreign = await createTeam(otherApi)
    expectError(await api.patch(`/v1/teams/${foreign.id}`, { name: "x" }), 404, "notFound", NOT_FOUND)
    expect((await otherApi.get<Team>(`/v1/teams/${foreign.id}`)).json.name).toBe(foreign.name)
    expectInvalid(await api.patch(`/v1/teams/${team.id}`, { name: "" }), "name")
  })

  it("delete answers 204 and removes the team and its memberships; then 404", async () => {
    const { tenant, api } = await tenantApi("p109-team-delete")
    const keep = await createTeam(api, [tenant.userId])
    const team = await createTeam(api, [tenant.userId])
    const response = await api.delete(`/v1/teams/${team.id}`)
    expect(response.status, response.text).toBe(204)
    expect(response.text).toBe("")
    expectError(await api.get(`/v1/teams/${team.id}`), 404, "notFound")
    expectError(await api.delete(`/v1/teams/${team.id}`), 404, "notFound", NOT_FOUND)
    const list = await listTeams(api)
    expect(list.data.map((item) => item.id)).toEqual([keep.id])
    expect(memberIds(list.data[0])).toEqual([tenant.userId])

    const { api: otherApi } = await tenantApi("p109-team-delete-o")
    const foreign = await createTeam(otherApi)
    expectError(await api.delete(`/v1/teams/${foreign.id}`), 404, "notFound", NOT_FOUND)
    expect((await otherApi.get(`/v1/teams/${foreign.id}`)).status).toBe(200)
  })
})

describe("inboxTeams.addMembers / removeMembers", () => {
  const setup = async (label: string): Promise<{ tenant: Tenant; api: Api; agent: string; team: Team }> => {
    const { tenant, api } = await tenantApi(label)
    const { userId: agent } = await addAgent(tenant)
    const team = await createTeam(api, [tenant.userId])
    return { tenant, api, agent, team }
  }

  it("addMembers adds new members once and returns the team", async () => {
    const { tenant, api, agent, team } = await setup("p109-team-add")
    const response = await api.post<Team>(`/v1/teams/${team.id}/members`, { userIds: [agent, tenant.userId] })
    expect(response.status, response.text).toBe(200)
    expect(response.json).toEqual({ ...team, updatedAt: expect.stringMatching(ISO) })
    expect(memberIds((await listTeams(api)).data[0])).toEqual([tenant.userId, agent].sort())
    // Adding an existing member again is a no-op.
    expect((await api.post(`/v1/teams/${team.id}/members`, { userIds: [agent] })).status).toBe(200)
    expect(memberIds((await listTeams(api)).data[0])).toEqual([tenant.userId, agent].sort())
  })

  it("addMembers refuses a non-member (400) and an unknown or foreign team (404)", async () => {
    const { api, team } = await setup("p109-team-add-errors")
    const outsider = await createOutsider()
    expectError(await api.post(`/v1/teams/${team.id}/members`, { userIds: [outsider.userId] }), 400, "invalidTeamMember", {
      message: NOT_MEMBERS,
    })
    expectError(await api.post(`/v1/teams/${UNKNOWN_ID}/members`, { userIds: [] }), 404, "notFound", NOT_FOUND)
    const { api: otherApi } = await tenantApi("p109-team-add-errors-o")
    const foreign = await createTeam(otherApi)
    expectError(await api.post(`/v1/teams/${foreign.id}/members`, { userIds: [] }), 404, "notFound", NOT_FOUND)
    expectInvalid(await api.post(`/v1/teams/${team.id}/members`, {}), "userIds")
  })

  it("removeMembers (DELETE with a JSON body) removes the listed members; unknown ids are a no-op", async () => {
    const { tenant, api, agent, team } = await setup("p109-team-remove")
    await api.post(`/v1/teams/${team.id}/members`, { userIds: [agent] })
    const outsider = await createOutsider()
    const response = await api.delete<Team>(`/v1/teams/${team.id}/members`, {
      json: { userIds: [agent, outsider.userId] },
    })
    expect(response.status, response.text).toBe(200)
    expect(response.json).toMatchObject({ id: team.id, name: team.name, workspaceId: tenant.workspaceId })
    expect(memberIds((await listTeams(api)).data[0])).toEqual([tenant.userId])

    expectError(
      await api.delete(`/v1/teams/${UNKNOWN_ID}/members`, { json: { userIds: [agent] } }),
      404,
      "notFound",
      NOT_FOUND,
    )
    expectInvalid(await api.delete(`/v1/teams/${team.id}/members`, { json: { userIds: ["x"] } }), "userIds.0")
  })
})
