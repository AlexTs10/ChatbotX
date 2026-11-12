// polls.list / get / create / update / delete / bulkDelete / getLinks /
// getAnalytics. Votes are cast in chat (no public API path): they come from
// the P1.12 fixture extension. `state` is computed from startsAt/endsAt at
// read time (endsAt exclusive).

import { describe, expect, it } from "vitest"
import {
  type Api,
  createWebchat,
  ext,
  expectError,
  expectInvalid,
  expectNotFound,
  ISO,
  keysOf,
  newContact,
  newTenant,
  SNOWFLAKE,
  UNKNOWN_ID,
  uniqueName,
} from "./helpers.ts"

type Candidate = {
  id: string
  position: number
  name: string
  description: string | null
  imageUrl: string | null
  votesCount: number
}
type Poll = {
  id: string
  name: string
  imageUrl: string
  startsAt: string
  endsAt: string
  afterVoteFlowId: string | null
  state: string
  createdAt: string
  updatedAt: string
  candidates: Candidate[]
}

const POLL_KEYS = ["afterVoteFlowId", "createdAt", "endsAt", "id", "imageUrl", "name", "startsAt", "state", "updatedAt"]

const HOUR = 3_600_000
const at = (offsetMs: number) => new Date(Date.now() + offsetMs).toISOString()

const body = (overrides: Record<string, unknown> = {}) => ({
  name: uniqueName("Best pizza"),
  imageUrl: "https://example.com/poll.png",
  startsAt: at(-HOUR),
  endsAt: at(24 * HOUR),
  candidates: [
    { name: "Margherita", description: "Classic", imageUrl: "https://example.com/m.png" },
    { name: "Diavola" },
  ],
  ...overrides,
})

const create = async (api: Api, overrides: Record<string, unknown> = {}): Promise<Poll> => {
  const response = await api.post<Poll>("/v1/polls", body(overrides))
  expect(response.status, response.text).toBe(200)
  return response.json
}

describe("polls.create / get", () => {
  it("creates a running poll with candidates in order and zero votes", async () => {
    const { api } = await newTenant("p113-po-create")
    const startsAt = "2026-01-01T00:00:00+02:00"
    const endsAt = "2099-01-01T00:00:00.000Z"
    const response = await api.post<Poll>("/v1/polls", body({ name: "  Lunch vote  ", startsAt, endsAt }))
    expect(response.status, response.text).toBe(200)
    const poll = response.json
    expect(keysOf(poll)).toEqual([...POLL_KEYS, "candidates"].sort())
    expect(poll).toEqual({
      id: expect.stringMatching(SNOWFLAKE),
      name: "Lunch vote",
      imageUrl: "https://example.com/poll.png",
      startsAt: "2025-12-31T22:00:00.000Z",
      endsAt,
      afterVoteFlowId: null,
      state: "running",
      createdAt: expect.stringMatching(ISO),
      updatedAt: expect.stringMatching(ISO),
      candidates: [
        {
          id: expect.stringMatching(SNOWFLAKE),
          position: 0,
          name: "Margherita",
          description: "Classic",
          imageUrl: "https://example.com/m.png",
          votesCount: 0,
        },
        { id: expect.stringMatching(SNOWFLAKE), position: 1, name: "Diavola", description: null, imageUrl: null, votesCount: 0 },
      ],
    })
    const got = await api.get<Poll>(`/v1/polls/${poll.id}`)
    expect(got.status, got.text).toBe(200)
    expect(got.json).toEqual(poll)
    const future = await create(api, { startsAt: at(HOUR), endsAt: at(2 * HOUR) })
    expect(future.state).toBe("notStarted")
    const past = await create(api, { startsAt: at(-2 * HOUR), endsAt: at(-HOUR) })
    expect(past.state).toBe("ended")
  })

  it("links an after-vote flow of the workspace", async () => {
    const { api } = await newTenant("p113-po-flow")
    const name = uniqueName("After vote")
    const flow = await api.post<{ id: string }>("/v1/flows", {
      name,
      folderId: null,
      publish: true,
      spec: { formatVersion: 1, name, steps: [{ type: "send", text: "Thanks for voting" }] },
    })
    expect(flow.status, flow.text).toBe(201)
    const poll = await create(api, { afterVoteFlowId: flow.json.id })
    expect(poll.afterVoteFlowId).toBe(flow.json.id)
    expectError(await api.post("/v1/polls", body({ afterVoteFlowId: UNKNOWN_ID })), 400, "pollFlowNotFound", {
      message: "Flow not found",
    })
  })

  it("400 for a bad date range or candidate count; 422 for bad input; 404 for unknown and foreign ids", async () => {
    const { api } = await newTenant("p113-po-invalid")
    const { api: other } = await newTenant("p113-po-invalid-other")
    const start = at(0)
    expectError(await api.post("/v1/polls", body({ startsAt: start, endsAt: start })), 400, "pollInvalidDateRange", {
      message: "Poll end must be after its start",
    })
    const countMessage = "A poll needs between 2 and 10 candidates"
    expectError(await api.post("/v1/polls", body({ candidates: [{ name: "Only" }] })), 400, "pollCandidateCount", {
      message: countMessage,
    })
    expectError(
      await api.post("/v1/polls", body({ candidates: Array.from({ length: 11 }, (_, index) => ({ name: `C${index}` })) })),
      400,
      "pollCandidateCount",
    )
    expectInvalid(await api.post("/v1/polls", body({ imageUrl: "not a url" })), "imageUrl")
    expectInvalid(await api.post("/v1/polls", body({ startsAt: "2026-01-01" })), "startsAt")
    expectInvalid(await api.post("/v1/polls", body({ name: "x".repeat(81) })), "name")
    expectInvalid(await api.post("/v1/polls", body({ candidates: [{ name: "" }, { name: "B" }] })), "candidates.0.name")
    const foreign = await create(other)
    expectNotFound(await api.get(`/v1/polls/${foreign.id}`), "Poll not found")
    expectNotFound(await api.get(`/v1/polls/${UNKNOWN_ID}`), "Poll not found")
    expect((await api.get<{ data: unknown[] }>("/v1/polls")).json.data).toEqual([])
  })
})

describe("polls.list", () => {
  it("is newest first with vote counts, pages and a name filter", async () => {
    const { tenant, api } = await newTenant("p113-po-list")
    const token = uniqueName("Vote")
    const polls: Poll[] = []
    for (let index = 0; index < 3; index += 1) {
      polls.push(await create(api, { name: `${token} ${index}` }))
    }
    const solo = await create(api)
    const contactId = await newContact(api)
    await (await ext()).createPollVote({
      workspaceId: tenant.workspaceId,
      pollId: solo.id,
      candidateId: solo.candidates[1]?.id as string,
      contactId,
    })
    const page1 = await api.get<{ data: Array<Record<string, unknown>>; pageCount: number }>("/v1/polls", {
      query: { perPage: 3 },
    })
    expect(page1.status, page1.text).toBe(200)
    expect(page1.json.pageCount).toBe(2)
    expect(page1.json.data.map((poll) => poll.id)).toEqual([solo.id, polls[2]?.id, polls[1]?.id])
    const { candidates: _candidates, ...soloBase } = solo
    expect(page1.json.data[0]).toEqual({ ...soloBase, votesCount: 1 })
    expect(page1.json.data[1]?.votesCount).toBe(0)
    const page2 = await api.get<{ data: Array<{ id: string }> }>("/v1/polls", { query: { perPage: 3, page: 2 } })
    expect(page2.json.data.map((poll) => poll.id)).toEqual([polls[0]?.id])
    const named = await api.get<{ data: Array<{ id: string }>; pageCount: number }>("/v1/polls", {
      query: { name: token.toUpperCase() },
    })
    expect(named.json.data.map((poll) => poll.id)).toEqual([polls[2]?.id, polls[1]?.id, polls[0]?.id])
    expectInvalid(await api.get("/v1/polls", { query: { perPage: 51 } }), "perPage")
  })
})

describe("polls.update", () => {
  it("keeps candidates sent with their id, adds new ones, deletes the rest, in the sent order", async () => {
    const { api } = await newTenant("p113-po-update")
    const poll = await create(api, {
      candidates: [{ name: "A" }, { name: "B" }, { name: "C" }],
    })
    const [a, , c] = poll.candidates as [Candidate, Candidate, Candidate]
    const response = await api.patch<Poll>(`/v1/polls/${poll.id}`, {
      name: "Renamed poll",
      candidates: [{ id: c.id, name: "C renamed" }, { name: "D" }, { id: a.id, name: "A", description: "first" }],
    })
    expect(response.status, response.text).toBe(200)
    expect(response.json).toMatchObject({ id: poll.id, name: "Renamed poll", startsAt: poll.startsAt })
    expect(response.json.candidates.map((candidate) => [candidate.name, candidate.position, candidate.description])).toEqual([
      ["C renamed", 0, null],
      ["D", 1, null],
      ["A", 2, "first"],
    ])
    expect(response.json.candidates[0]?.id).toBe(c.id)
    expect(response.json.candidates[2]?.id).toBe(a.id)
    expect((await api.get<Poll>(`/v1/polls/${poll.id}`)).json).toEqual(response.json)

    const datesOnly = await api.patch<Poll>(`/v1/polls/${poll.id}`, { endsAt: "2098-06-01T00:00:00.000Z" })
    expect(datesOnly.json).toMatchObject({ endsAt: "2098-06-01T00:00:00.000Z", name: "Renamed poll" })
    expect(datesOnly.json.candidates).toHaveLength(3)
  })

  it("400 for a candidate of another poll, a bad range or count; 422 empty body; 404 unknown", async () => {
    const { api } = await newTenant("p113-po-update-err")
    const poll = await create(api)
    const other = await create(api)
    expectError(
      await api.patch(`/v1/polls/${poll.id}`, {
        candidates: [{ id: other.candidates[0]?.id, name: "Stolen" }, { name: "B" }],
      }),
      400,
      "pollCandidateNotInPoll",
      { message: "Candidate does not belong to this poll" },
    )
    expectError(
      await api.patch(`/v1/polls/${poll.id}`, {
        candidates: [
          { id: poll.candidates[0]?.id, name: "Twice" },
          { id: poll.candidates[0]?.id, name: "Twice" },
        ],
      }),
      400,
      "pollCandidateNotInPoll",
    )
    expectError(await api.patch(`/v1/polls/${poll.id}`, { endsAt: poll.startsAt }), 400, "pollInvalidDateRange")
    expectError(await api.patch(`/v1/polls/${poll.id}`, { candidates: [] }), 400, "pollCandidateCount")
    expectInvalid(await api.patch(`/v1/polls/${poll.id}`, {}), "")
    expectNotFound(await api.patch(`/v1/polls/${UNKNOWN_ID}`, { name: "x" }), "Poll not found")
    expect((await api.get<Poll>(`/v1/polls/${poll.id}`)).json).toEqual(poll)
  })
})

describe("polls.delete / bulkDelete", () => {
  it("delete is 204; bulkDelete counts only this workspace's polls", async () => {
    const { api } = await newTenant("p113-po-delete")
    const { api: other } = await newTenant("p113-po-delete-other")
    const poll = await create(api)
    const response = await api.delete(`/v1/polls/${poll.id}`)
    expect(response.status, response.text).toBe(204)
    expect(response.text).toBe("")
    expectNotFound(await api.get(`/v1/polls/${poll.id}`), "Poll not found")
    expectNotFound(await api.delete(`/v1/polls/${poll.id}`), "Poll not found")

    const first = await create(api)
    const second = await create(api)
    const kept = await create(api)
    const foreign = await create(other)
    const bulk = await api.post<{ deleted: number }>("/v1/polls/bulk-delete", { ids: [first.id, second.id, foreign.id, UNKNOWN_ID] })
    expect(bulk.status, bulk.text).toBe(200)
    expect(bulk.json).toEqual({ deleted: 2 })
    expect((await api.get<{ data: Array<{ id: string }> }>("/v1/polls")).json.data.map((p) => p.id)).toEqual([kept.id])
    expect((await other.get(`/v1/polls/${foreign.id}`)).status).toBe(200)
    expectInvalid(await api.post("/v1/polls/bulk-delete", { ids: [] }), "ids")
  })
})

describe("polls.getLinks", () => {
  it("returns the omnichannel link and one link per ref-capable connected inbox", async () => {
    const { tenant, api } = await newTenant("p113-po-links")
    const poll = await create(api)
    const empty = await api.get<{ omnichannelUrl: string; links: unknown[] }>(`/v1/polls/${poll.id}/links`)
    expect(empty.status, empty.text).toBe(200)
    expect(empty.json).toEqual({ omnichannelUrl: `http://app/l/${tenant.workspaceId}/p/${poll.id}`, links: [] })

    const webchatName = uniqueName("Poll chat")
    const webchat = await createWebchat(api, webchatName)
    const response = await api.get<{ omnichannelUrl: string; links: Array<Record<string, string>> }>(`/v1/polls/${poll.id}/links`)
    expect(response.json.omnichannelUrl).toBe(`http://app/l/${tenant.workspaceId}/p/${poll.id}`)
    expect(response.json.links).toHaveLength(1)
    const link = response.json.links[0] as Record<string, string>
    expect(link).toMatchObject({ inboxId: webchat.inboxId, channel: "webchat", name: webchatName })
    const url = new URL(link.url as string)
    expect(`${url.origin}${url.pathname}`).toBe("http://app/webchat")
    expect(url.searchParams.get("workspaceId")).toBe(tenant.workspaceId)
    expect(url.searchParams.get("webchatId")).toBeTruthy()
    expect(url.searchParams.get("ref")).toBeTruthy()
    expectNotFound(await api.get(`/v1/polls/${UNKNOWN_ID}/links`), "Poll not found")
  })
})

describe("polls.getAnalytics", () => {
  it("counts votes per candidate and by contact gender, with percentages", async () => {
    const { tenant, api } = await newTenant("p113-po-analytics")
    const poll = await create(api, { candidates: [{ name: "Red" }, { name: "Blue", imageUrl: "https://example.com/b.png" }] })
    const [red, blue] = poll.candidates as [Candidate, Candidate]
    const empty = await api.get(`/v1/polls/${poll.id}/analytics`)
    expect(empty.status, empty.text).toBe(200)
    const zeroCandidate = { totalVotes: 0, totalPercent: 0, maleVotes: 0, malePercent: 0, femaleVotes: 0, femalePercent: 0 }
    expect(empty.json).toEqual({
      poll: { id: poll.id, name: poll.name, imageUrl: poll.imageUrl, startsAt: poll.startsAt, endsAt: poll.endsAt },
      totalVotes: 0,
      gender: { male: 0, female: 0, unknown: 0 },
      candidates: [
        { id: red.id, name: "Red", imageUrl: null, ...zeroCandidate },
        { id: blue.id, name: "Blue", imageUrl: "https://example.com/b.png", ...zeroCandidate },
      ],
    })

    const fixtures = await ext()
    const votes: Array<[Candidate, "male" | "female" | "unknown"]> = [
      [red, "male"],
      [red, "female"],
      [blue, "female"],
      [red, "unknown"],
    ]
    for (const [candidate, gender] of votes) {
      const contactId = await newContact(api, uniqueName("Voter"), gender)
      await fixtures.createPollVote({ workspaceId: tenant.workspaceId, pollId: poll.id, candidateId: candidate.id, contactId })
    }
    const response = await api.get<Record<string, unknown>>(`/v1/polls/${poll.id}/analytics`)
    expect(response.json).toEqual({
      poll: { id: poll.id, name: poll.name, imageUrl: poll.imageUrl, startsAt: poll.startsAt, endsAt: poll.endsAt },
      totalVotes: 4,
      gender: { male: 1, female: 2, unknown: 1 },
      candidates: [
        {
          id: red.id,
          name: "Red",
          imageUrl: null,
          totalVotes: 3,
          totalPercent: 75,
          maleVotes: 1,
          malePercent: 100,
          femaleVotes: 1,
          femalePercent: 50,
        },
        {
          id: blue.id,
          name: "Blue",
          imageUrl: "https://example.com/b.png",
          totalVotes: 1,
          totalPercent: 25,
          maleVotes: 0,
          malePercent: 0,
          femaleVotes: 1,
          femalePercent: 50,
        },
      ],
    })
    const counted = await api.get<Poll>(`/v1/polls/${poll.id}`)
    expect(counted.json.candidates.map((candidate) => candidate.votesCount)).toEqual([3, 1])
    expectNotFound(await api.get(`/v1/polls/${UNKNOWN_ID}/analytics`), "Poll not found")
  })
})
