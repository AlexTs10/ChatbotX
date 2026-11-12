// questionnaires.list / get / create / update (PUT) / rename / duplicate /
// delete and the submission reads (listSubmissions / getSubmission /
// getSubmissionStats / deleteSubmission). Submissions are written by the
// chat path (no public API): they come from the P1.12 fixture extension.

import { describe, expect, it, vi } from "vitest"
import {
  type Api,
  createPublishedFlow,
  ext,
  expectError,
  expectInvalid,
  expectNotFound,
  ISO,
  keysOf,
  newContact,
  newTenant,
  SNOWFLAKE,
  sortQuery,
  UNKNOWN_ID,
  uniqueName,
} from "./helpers.ts"

// Worker jobs and many writes per test: allow more than the 30 s default on a loaded runner.
vi.setConfig({ testTimeout: 120_000 })

type Question = Record<string, unknown> & { id: string; title: string; type: string; orderNo: number }
type Questionnaire = Record<string, unknown> & { id: string; name: string; questions: Question[] }

const create = async (api: Api, name = uniqueName("Survey")): Promise<string> => {
  const response = await api.post<{ id: string }>("/v1/questionnaires", { name })
  expect(response.status, response.text).toBe(201)
  expect(response.json).toEqual({ id: expect.stringMatching(SNOWFLAKE) })
  return response.json.id
}

const get = async (api: Api, id: string): Promise<Questionnaire> => {
  const response = await api.get<Questionnaire>(`/v1/questionnaires/${id}`)
  expect(response.status, response.text).toBe(200)
  return response.json
}

const QUESTIONS = [
  { title: "Your name?", type: "text", active: true },
  {
    title: "Favourite color?",
    type: "multipleChoice",
    active: true,
    point: 2,
    config: {
      options: [
        { id: "red", label: "Red", points: 1 },
        { id: "blue", label: "Blue", points: 3 },
      ],
    },
  },
  { title: "Age?", type: "number", active: false },
]

const updateBody = (questions: unknown[] = QUESTIONS) => ({
  triggerFlowId: null,
  enableScore: true,
  enableRetryMessages: false,
  enableCustomFieldMapping: false,
  questions,
})

describe("questionnaires.create / get", () => {
  it("creates an empty questionnaire with the defaults", async () => {
    const { api } = await newTenant("p114-qn-create")
    const id = await create(api, "  Onboarding  ")
    const questionnaire = await get(api, id)
    expect(keysOf(questionnaire)).toEqual([
      "createdAt",
      "enableCustomFieldMapping",
      "enableRetryMessages",
      "enableScore",
      "id",
      "name",
      "questions",
      "triggerFlowId",
      "updatedAt",
    ])
    expect(questionnaire).toEqual({
      id,
      name: "Onboarding",
      triggerFlowId: null,
      enableScore: false,
      enableRetryMessages: false,
      enableCustomFieldMapping: true,
      createdAt: expect.stringMatching(ISO),
      updatedAt: expect.stringMatching(ISO),
      questions: [],
    })
  })

  it("409 for a taken name; 422 empty name; 404 unknown or foreign", async () => {
    const { api } = await newTenant("p114-qn-dup")
    const { api: other } = await newTenant("p114-qn-dup-other")
    const name = uniqueName("Survey")
    const id = await create(api, name)
    expectError(await api.post("/v1/questionnaires", { name }), 409, "nameAlreadyExists", {
      message: "Questionnaire name already exists",
    })
    expect((await other.post("/v1/questionnaires", { name })).status).toBe(201)
    expectInvalid(await api.post("/v1/questionnaires", { name: " " }), "name")
    expectNotFound(await other.get(`/v1/questionnaires/${id}`), "Questionnaire not found")
    expectNotFound(await api.get(`/v1/questionnaires/${UNKNOWN_ID}`), "Questionnaire not found")
  })
})

describe("questionnaires.update", () => {
  it("replaces the settings and questions in order; questions sent with an id are kept", async () => {
    const { api } = await newTenant("p114-qn-update")
    const id = await create(api)
    const flowId = await createPublishedFlow(api)
    const response = await api.put(`/v1/questionnaires/${id}`, { ...updateBody(), triggerFlowId: flowId })
    expect(response.status, response.text).toBe(204)
    expect(response.text).toBe("")
    const saved = await get(api, id)
    expect(saved).toMatchObject({ triggerFlowId: flowId, enableScore: true, enableRetryMessages: false, enableCustomFieldMapping: false })
    expect(saved.questions).toHaveLength(3)
    expect(keysOf(saved.questions[0])).toEqual([
      "active",
      "config",
      "customFieldId",
      "id",
      "image",
      "orderNo",
      "point",
      "retryMessage",
      "systemFieldKey",
      "title",
      "type",
    ])
    expect(saved.questions).toEqual([
      {
        id: expect.stringMatching(SNOWFLAKE),
        title: "Your name?",
        type: "text",
        active: true,
        image: null,
        orderNo: 0,
        point: 1,
        retryMessage: null,
        customFieldId: null,
        systemFieldKey: null,
        config: null,
      },
      {
        id: expect.stringMatching(SNOWFLAKE),
        title: "Favourite color?",
        type: "multipleChoice",
        active: true,
        image: null,
        orderNo: 1,
        point: 2,
        retryMessage: null,
        customFieldId: null,
        systemFieldKey: null,
        config: {
          options: [
            { id: "red", label: "Red", points: 1 },
            { id: "blue", label: "Blue", points: 3 },
          ],
        },
      },
      {
        id: expect.stringMatching(SNOWFLAKE),
        title: "Age?",
        type: "number",
        active: false,
        image: null,
        orderNo: 2,
        point: 1,
        retryMessage: null,
        customFieldId: null,
        systemFieldKey: null,
        config: null,
      },
    ])
    const [first, , third] = saved.questions as [Question, Question, Question]
    // Keep the third (now first), drop the others, add a new one.
    const second = await api.put(`/v1/questionnaires/${id}`, {
      ...updateBody([
        { id: third.id, title: "Your age?", type: "number", active: true },
        { title: "Email?", type: "email", active: true },
      ]),
    })
    expect(second.status, second.text).toBe(204)
    const after = await get(api, id)
    expect(after.questions.map((question) => [question.title, question.orderNo])).toEqual([
      ["Your age?", 0],
      ["Email?", 1],
    ])
    expect(after.questions[0]?.id).toBe(third.id)
    expect(after.questions.map((question) => question.id)).not.toContain(first.id)
  })

  it("400 without an active question, with too few or duplicate options; 404 unknown flow or questionnaire", async () => {
    const { api } = await newTenant("p114-qn-update-err")
    const id = await create(api)
    expectError(
      await api.put(`/v1/questionnaires/${id}`, updateBody([{ title: "Off", type: "text", active: false }])),
      400,
      "noActiveQuestions",
      { message: "Questionnaire requires at least one active question" },
    )
    expectError(
      await api.put(
        `/v1/questionnaires/${id}`,
        updateBody([{ title: "Pick", type: "multipleChoice", active: true, config: { options: [{ id: "a", label: "A" }] } }]),
      ),
      400,
      "invalidOptions",
      { message: "Multiple choice questions require at least two options" },
    )
    expectError(
      await api.put(
        `/v1/questionnaires/${id}`,
        updateBody([
          {
            title: "Pick",
            type: "multipleChoice",
            active: true,
            config: {
              options: [
                { id: "a", label: "Same" },
                { id: "b", label: "Same" },
              ],
            },
          },
        ]),
      ),
      400,
      "invalidOptions",
      { message: "Multiple choice option labels must be unique" },
    )
    expectNotFound(await api.put(`/v1/questionnaires/${id}`, { ...updateBody(), triggerFlowId: UNKNOWN_ID }), "Flow not found")
    expectNotFound(await api.put(`/v1/questionnaires/${UNKNOWN_ID}`, updateBody()), "Questionnaire not found")
    expectInvalid(await api.put(`/v1/questionnaires/${id}`, updateBody([{ title: "x", type: "date", active: true }])), "questions.0.type")
    expectInvalid(await api.put(`/v1/questionnaires/${id}`, { questions: [] }), "enableScore")
    expect((await get(api, id)).questions).toEqual([])
  })
})

describe("questionnaires.rename / duplicate / delete", () => {
  it("rename (204), duplicate as '<name> - Copy' with its questions, delete (204)", async () => {
    const { api } = await newTenant("p114-qn-copy")
    const id = await create(api, "Feedback")
    expect((await api.put(`/v1/questionnaires/${id}`, updateBody())).status).toBe(204)
    const renamed = await api.patch(`/v1/questionnaires/${id}/rename`, { name: "Customer feedback" })
    expect(renamed.status, renamed.text).toBe(204)
    expect((await get(api, id)).name).toBe("Customer feedback")
    const taken = await create(api)
    expectError(await api.patch(`/v1/questionnaires/${taken}/rename`, { name: "Customer feedback" }), 409, "nameAlreadyExists")
    expectNotFound(await api.patch(`/v1/questionnaires/${UNKNOWN_ID}/rename`, { name: "x" }), "Questionnaire not found")

    const copy = await api.post<{ id: string }>(`/v1/questionnaires/${id}/duplicate`, {})
    expect(copy.status, copy.text).toBe(201)
    const source = await get(api, id)
    const duplicated = await get(api, copy.json.id)
    expect(duplicated.name).toBe("Customer feedback - Copy")
    expect(duplicated.questions.map(({ id: _id, ...question }) => question)).toEqual(
      source.questions.map(({ id: _id, ...question }) => question),
    )
    const copy2 = await api.post<{ id: string }>(`/v1/questionnaires/${id}/duplicate`, {})
    expect((await get(api, copy2.json.id)).name).toBe("Customer feedback - Copy 2")
    expectNotFound(await api.post(`/v1/questionnaires/${UNKNOWN_ID}/duplicate`, {}), "Questionnaire not found")

    const deleted = await api.delete(`/v1/questionnaires/${id}`)
    expect(deleted.status, deleted.text).toBe(204)
    expectNotFound(await api.get(`/v1/questionnaires/${id}`), "Questionnaire not found")
    // The name is free again only if the row is gone; a deleted id 404s on rename.
    expectNotFound(await api.patch(`/v1/questionnaires/${id}/rename`, { name: "x" }), "Questionnaire not found")
  })
})

describe("questionnaires.list", () => {
  it("is sorted by name (asc by default) with applicant counts and pages", async () => {
    const { tenant, api } = await newTenant("p114-qn-list")
    const token = uniqueName("q")
    const b = await create(api, `B ${token}`)
    const a = await create(api, `A ${token}`)
    const c = await create(api, `C ${token}`)
    const contactId = await newContact(api)
    await (await ext()).createQuestionnaireSubmission({
      workspaceId: tenant.workspaceId,
      questionnaireId: b,
      contactId,
      status: "completed",
    })
    const page1 = await api.get<{ data: Array<Record<string, unknown>>; pageCount: number }>("/v1/questionnaires", {
      query: { perPage: 2 },
    })
    expect(page1.status, page1.text).toBe(200)
    expect(page1.json.pageCount).toBe(2)
    expect(page1.json.data.map((row) => row.id)).toEqual([a, b])
    expect(keysOf(page1.json.data[0])).toEqual(["applicantsCount", "createdAt", "id", "name", "updatedAt"])
    expect(page1.json.data[1]).toMatchObject({ id: b, name: `B ${token}`, applicantsCount: 1 })
    const page2 = await api.get<{ data: Array<{ id: string }> }>("/v1/questionnaires", { query: { perPage: 2, page: 2 } })
    expect(page2.json.data.map((row) => row.id)).toEqual([c])
    const desc = await api.get<{ data: Array<{ id: string }> }>(`/v1/questionnaires?${sortQuery("name", true)}`)
    expect(desc.json.data.map((row) => row.id)).toEqual([c, b, a])
    const filtered = await api.get<{ data: Array<{ id: string }> }>("/v1/questionnaires", { query: { name: `c ${token}` } })
    expect(filtered.json.data.map((row) => row.id)).toEqual([c])
    expectInvalid(await api.get(`/v1/questionnaires?${sortQuery("createdAt", true)}`), "sort.0.id")
  })
})

describe("questionnaire submissions", () => {
  it("lists newest first, shows one submission's answers, counts completion, deletes", async () => {
    const { tenant, api } = await newTenant("p114-qn-subs")
    const id = await create(api)
    expect((await api.put(`/v1/questionnaires/${id}`, updateBody())).status).toBe(204)
    const [nameQuestion, colorQuestion] = (await get(api, id)).questions as [Question, Question]
    const fixtures = await ext()
    const firstName = uniqueName("Ann")
    const annId = await newContact(api, firstName)
    const bobId = await newContact(api, uniqueName("Bob"))
    const now = Date.now()
    const done = await fixtures.createQuestionnaireSubmission({
      workspaceId: tenant.workspaceId,
      questionnaireId: id,
      contactId: annId,
      status: "completed",
      totalPoints: 3,
      createdAt: new Date(now - 2_000).toISOString(),
      answers: [
        { questionId: nameQuestion.id, questionTitle: "Your name?", questionType: "text", label: "Your name?", value: "Ann" },
        {
          questionId: colorQuestion.id,
          questionTitle: "Favourite color?",
          questionType: "multipleChoice",
          label: "Favourite color?",
          value: { optionId: "blue", label: "Blue" },
          pointsEarned: 3,
        },
      ],
    })
    const open = await fixtures.createQuestionnaireSubmission({
      workspaceId: tenant.workspaceId,
      questionnaireId: id,
      contactId: bobId,
      status: "inProgress",
      createdAt: new Date(now - 1_000).toISOString(),
    })

    const listed = await api.get<{ enableScore: boolean; data: Array<Record<string, unknown>>; pageCount: number }>(
      `/v1/questionnaires/${id}/submissions`,
    )
    expect(listed.status, listed.text).toBe(200)
    expect(listed.json.enableScore).toBe(true)
    expect(listed.json.pageCount).toBe(1)
    expect(listed.json.data.map((row) => row.id)).toEqual([open.submissionId, done.submissionId])
    expect(listed.json.data[1]).toEqual({
      id: done.submissionId,
      status: "completed",
      totalPoints: 3,
      completedAt: new Date(now - 2_000).toISOString(),
      conversationId: null,
      contact: {
        id: annId,
        fullName: expect.stringContaining(firstName),
        firstName,
        lastName: "Parity",
        // contacts.create stores the empty email it was sent.
        email: "",
        phoneNumber: null,
        avatar: null,
      },
    })
    const byPoints = await api.get<{ data: Array<{ id: string }> }>(
      `/v1/questionnaires/${id}/submissions?${sortQuery("totalPoints", true)}`,
    )
    expect(byPoints.json.data.map((row) => row.id)).toEqual([open.submissionId, done.submissionId])
    const named = await api.get<{ data: Array<{ id: string }> }>(`/v1/questionnaires/${id}/submissions`, {
      query: { name: firstName },
    })
    expect(named.json.data.map((row) => row.id)).toEqual([done.submissionId])

    const detail = await api.get(`/v1/questionnaires/${id}/submissions/${done.submissionId}`)
    expect(detail.status, detail.text).toBe(200)
    expect(detail.json).toEqual({
      id: done.submissionId,
      contact: { id: annId, fullName: expect.stringContaining(firstName) },
      conversationId: null,
      status: "completed",
      totalPoints: 3,
      answers: [
        { questionId: nameQuestion.id, label: "Your name?", value: "Ann", pointsEarned: null },
        { questionId: colorQuestion.id, label: "Favourite color?", value: { optionId: "blue", label: "Blue" }, pointsEarned: 3 },
      ],
    })

    const stats = await api.get(`/v1/questionnaires/${id}/submissions/stats`)
    expect(stats.status, stats.text).toBe(200)
    expect(stats.json).toEqual({ totalApplicants: 2, completed: 1, completionRate: 50 })

    const deleted = await api.delete(`/v1/questionnaires/${id}/submissions/${open.submissionId}`)
    expect(deleted.status, deleted.text).toBe(204)
    expectNotFound(await api.get(`/v1/questionnaires/${id}/submissions/${open.submissionId}`), "Submission not found")
    expect((await api.get(`/v1/questionnaires/${id}/submissions/stats`)).json).toEqual({
      totalApplicants: 1,
      completed: 1,
      completionRate: 100,
    })
    // Deleting an unknown submission is still 204.
    expect((await api.delete(`/v1/questionnaires/${id}/submissions/${UNKNOWN_ID}`)).status).toBe(204)
  })

  it("empty stats; 404 for an unknown questionnaire on every read", async () => {
    const { api } = await newTenant("p114-qn-subs-404")
    const id = await create(api)
    expect((await api.get(`/v1/questionnaires/${id}/submissions/stats`)).json).toEqual({
      totalApplicants: 0,
      completed: 0,
      completionRate: 0,
    })
    expect((await api.get(`/v1/questionnaires/${id}/submissions`)).json).toEqual({ enableScore: false, data: [], pageCount: 0 })
    expectNotFound(await api.get(`/v1/questionnaires/${UNKNOWN_ID}/submissions`), "Questionnaire not found")
    expectNotFound(await api.get(`/v1/questionnaires/${UNKNOWN_ID}/submissions/stats`), "Questionnaire not found")
    expectNotFound(await api.get(`/v1/questionnaires/${id}/submissions/${UNKNOWN_ID}`), "Submission not found")
  })
})
