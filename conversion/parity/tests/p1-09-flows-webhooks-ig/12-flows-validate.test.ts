// flows.validate (scope `automation`): compiles a flow-spec DSL document and
// validates it like flows.publish, without writing anything. Success: the
// compiled {nodes, edges}. Failure: schema errors -> 422 invalidRequestData
// with `data.issues` (defined true); compile / channel errors -> 422
// invalidRequestData with `data` = [{path, code, message, hint?, candidates?,
// capability?}] (defined false). The DSL has no "Execute JavaScript" step
// (a dropped feature): the step union does not list it.

import { describe, expect, it } from "vitest"
import { type AuthoringError, createFlow, expectAuthoringErrors, expectError, expectInvalid, newTenant, uniqueName } from "./helpers.ts"

type Graph = {
  nodes: Array<{ id: string; type: string; position: unknown; measured: unknown; data: Record<string, unknown> }>
  edges: Array<Record<string, string>>
}

const ID = expect.stringMatching(/^\d+$/)

const spec = (name: string, steps: unknown[], extra: Record<string, unknown> = {}) => ({
  spec: { formatVersion: 1, name, steps, ...extra },
})

describe("flows.validate", () => {
  it("returns the compiled graph (200): buttons route to their own node, actions get a performAction node", async () => {
    const { api } = await newTenant("p109-validate-ok")
    const name = uniqueName("Ok")
    const response = await api.post<Graph>(
      "/v1/flows/validate",
      spec(name, [
        { type: "send", text: "hi", buttons: [{ text: "Yes", then: [{ type: "send", text: "yes!" }] }] },
        { type: "action", action: "addTags", tagNames: ["vip"] },
      ]),
    )
    expect(response.status, response.text).toBe(200)
    expect(Object.keys(response.json).sort()).toEqual(["edges", "nodes"])
    const [start, yes, action] = response.json.nodes
    expect(response.json.nodes).toHaveLength(3)
    const quickReplySettings = {
      followUp: { id: ID, enabled: false, duration: 1, unit: "days", target: null },
      retry: { id: ID, enabled: false, message: "", maxRetries: 3, target: null },
    }
    expect(start).toEqual({
      id: ID,
      position: { x: 100, y: 100 },
      measured: { width: 288, height: 100 },
      type: "sendMessage",
      data: {
        name: "Send Message",
        isStartNode: true,
        details: {
          beforeStep: { id: ID, stepType: "chooseChannel", channel: "omnichannel" },
          steps: [
            {
              id: ID,
              stepType: "sendText",
              text: "hi",
              buttons: [
                {
                  id: ID,
                  label: "Yes",
                  buttonType: "startAnotherNode",
                  beforeStep: { id: ID, stepType: "startAnotherNode", nodeId: yes?.id, viewOnly: true },
                  steps: [],
                },
              ],
            },
          ],
          quickReplies: [],
          quickReplySettings,
        },
      },
    })
    expect(yes).toMatchObject({
      type: "sendMessage",
      position: { x: 508, y: 100 },
      data: { isStartNode: false, details: { steps: [{ id: ID, stepType: "sendText", text: "yes!", buttons: [] }] } },
    })
    expect(action).toEqual({
      id: ID,
      position: { x: 508, y: 280 },
      measured: { width: 288, height: 100 },
      type: "performAction",
      data: {
        name: "Perform Action",
        isStartNode: false,
        details: { steps: [{ id: ID, stepType: "addContactTag", tags: ["vip"] }] },
      },
    })
    const button = ((start?.data.details as { steps: Array<{ buttons: Array<{ id: string }> }> }).steps[0]?.buttons[0]) as {
      id: string
    }
    expect(response.json.edges).toEqual([
      { id: ID, source: start?.id, sourceHandle: button.id, target: yes?.id, targetHandle: yes?.id },
      { id: ID, source: start?.id, sourceHandle: start?.id, target: action?.id, targetHandle: action?.id },
    ])
    // Nothing is written.
    expect((await api.get("/v1/flows")).json).toEqual({ data: [], pageCount: 0 })
  })

  it("a channel spec sets the channel step; a channel's button rules are applied", async () => {
    const { api } = await newTenant("p109-validate-channel")
    const response = await api.post<Graph>(
      "/v1/flows/validate",
      spec(uniqueName("Tik"), [{ type: "send", text: "a", buttons: [{ text: "1" }] }], { channel: "tiktok" }),
    )
    expect(response.status, response.text).toBe(200)
    expect(response.json.nodes).toHaveLength(1)
    expect(response.json.edges).toEqual([])
    expect(response.json.nodes[0]?.data.details).toMatchObject({
      beforeStep: { stepType: "chooseChannel", channel: "tiktok" },
      steps: [{ stepType: "sendText", text: "a", buttons: [{ id: ID, label: "1", buttonType: null, beforeStep: null, steps: [] }] }],
    })
  })

  describe("compile and channel errors -> 422 with authoring errors (defined false)", () => {
    const cases: Array<[string, unknown[], Record<string, unknown>, AuthoringError[]]> = [
      [
        "a step the channel does not support (unsupportedBlock + capability)",
        [{ type: "send", text: "x" }],
        { channel: "threads" },
        [
          {
            capability: { block: "sendText", channel: "threads", code: "unsupportedBlock", policyVersion: 3 },
            path: "steps[0].data.details.steps[0]",
            code: "unsupportedBlock",
            message: "The threads channel does not support sendText.",
          },
        ],
      ],
      [
        "startFlow to an unknown flow name (unknownFlow + hint + candidates)",
        [{ type: "startFlow", flowName: "Nope" }],
        {},
        [
          {
            path: "steps[0].flowName",
            code: "unknownFlow",
            message: 'No flow named "Nope" in this workspace.',
            hint: "Call flows.list and pick a name from the results.",
            candidates: [],
          },
        ],
      ],
      [
        "setCustomField with an unknown custom field (unknownCustomField)",
        [{ type: "action", action: "setCustomField", customFieldName: "nope", value: "1" }],
        {},
        [
          {
            path: "steps[0].customFieldName",
            code: "unknownCustomField",
            message: 'No custom field named "nope" in this workspace.',
            hint: "Call contacts.listFilterFields and pick a custom field name from the results.",
            candidates: [],
          },
        ],
      ],
      [
        "steps after a goto (unreachableStep) and a goto to the previous step (selfLoopGoto)",
        [
          { type: "send", id: "a", text: "a" },
          { type: "goto", targetId: "a" },
          { type: "send", text: "b" },
        ],
        {},
        [
          {
            path: "steps[2]",
            code: "unreachableStep",
            message: 'Step 3 of 3 can never run — "goto" at step 2 ends its step list.',
            hint: "Move the following steps inside this step's own branch (e.g. a branch case's `then`) or delete them.",
          },
          {
            path: "steps[1]",
            code: "selfLoopGoto",
            message: '"goto" targets the immediately preceding step, which would create a self-loop edge.',
            hint: 'Target an earlier step, or remove this "goto" — the flow already continues into the preceding step by default.',
          },
        ],
      ],
      [
        "a goto to an unknown step id (invalidGotoTarget)",
        [
          { type: "send", text: "a" },
          { type: "send", text: "b" },
          { type: "goto", targetId: "zzz" },
        ],
        {},
        [
          {
            path: "steps[2].targetId",
            code: "invalidGotoTarget",
            message: '"goto" targets step id "zzz", which is not an earlier step\'s id in this spec.',
            hint: "Set an explicit `id` on the step you want to jump to, earlier in the spec.",
          },
        ],
      ],
      [
        "a goto as the first step (invalidFirstStep + invalidGotoTarget)",
        [{ type: "goto", targetId: "a" }],
        {},
        [
          {
            path: "steps[0]",
            code: "invalidFirstStep",
            message: '"goto" cannot be the first step — there is no earlier step yet to jump from.',
          },
          {
            path: "steps[0].targetId",
            code: "invalidGotoTarget",
            message: '"goto" targets step id "a", which is not an earlier step\'s id in this spec.',
            hint: "Set an explicit `id` on the step you want to jump to, earlier in the spec.",
          },
        ],
      ],
      [
        "two steps with the same id (duplicateStepId on each)",
        [
          { type: "send", id: "a", text: "a" },
          { type: "send", id: "a", text: "b" },
        ],
        {},
        [
          {
            path: "steps[0]",
            code: "duplicateStepId",
            message: 'Step id "a" is used by 2 steps; ids must be unique across the whole spec.',
          },
          {
            path: "steps[1]",
            code: "duplicateStepId",
            message: 'Step id "a" is used by 2 steps; ids must be unique across the whole spec.',
          },
        ],
      ],
    ]
    it.each(cases)("%s", async (_label, steps, extra, expected) => {
      const { api } = await newTenant("p109-validate-err")
      const response = await api.post("/v1/flows/validate", spec(uniqueName("Err"), steps, extra))
      expect(expectAuthoringErrors(response)).toEqual(expected)
      expect((response.json as { defined: boolean }).defined).toBe(false)
      expect((response.json as { message: string }).message).toBe(
        expected.map((error) => `${error.path}: ${error.message}`).join("; "),
      )
    })
  })

  it("unknownFlow candidates are the closest flow names of the workspace", async () => {
    const { api } = await newTenant("p109-validate-candidates")
    await createFlow(api, { name: "Welcome flow" })
    await createFlow(api, { name: "Welcome back" })
    await createFlow(api, { name: "Unrelated" })
    const response = await api.post("/v1/flows/validate", spec("x", [{ type: "startFlow", flowName: "Welcome" }]))
    const [error] = expectAuthoringErrors(response)
    expect(error?.code).toBe("unknownFlow")
    expect([...(error?.candidates ?? [])].sort()).toEqual(["Welcome back", "Welcome flow"])
    // An existing flow name compiles.
    const ok = await api.post("/v1/flows/validate", spec("x", [{ type: "startFlow", flowName: "Unrelated" }]))
    expect(ok.status, ok.text).toBe(200)
  })

  describe("schema errors -> 422 invalidRequestData with data.issues (defined true)", () => {
    const cases: Array<[string, unknown, string]> = [
      ["missing formatVersion", { spec: { name: "x", steps: [{ type: "send", text: "a" }] } }, "spec.formatVersion"],
      ["empty steps", { spec: { formatVersion: 1, name: "x", steps: [] } }, "spec.steps"],
      ["empty name", { spec: { formatVersion: 1, name: " ", steps: [{ type: "send", text: "a" }] } }, "spec.name"],
      ["unknown channel", { spec: { formatVersion: 1, name: "x", channel: "fax", steps: [{ type: "send", text: "a" }] } }, "spec.channel"],
      ["send with text and imageUrl", { spec: { formatVersion: 1, name: "x", steps: [{ type: "send", text: "a", imageUrl: "https://x.parity.test/a.png" }] } }, "spec.steps.0"],
      ["send with 4 buttons", { spec: { formatVersion: 1, name: "x", steps: [{ type: "send", text: "a", buttons: [{ text: "1" }, { text: "2" }, { text: "3" }, { text: "4" }] }] } }, "spec.steps.0.buttons"],
      ["send text over 1000 characters", { spec: { formatVersion: 1, name: "x", steps: [{ type: "send", text: "a".repeat(1001) }] } }, "spec.steps.0.text"],
      ["an unknown step type", { spec: { formatVersion: 1, name: "x", steps: [{ type: "executeJavascript" }] } }, "spec.steps.0.type"],
      ["addTags without tagNames", { spec: { formatVersion: 1, name: "x", steps: [{ type: "action", action: "addTags" }] } }, "spec.steps.0.tagNames"],
      ["no spec", {}, "spec"],
    ]
    it.each(cases)("%s", async (_label, body, path) => {
      const { api } = await newTenant("p109-validate-schema")
      const response = await api.post("/v1/flows/validate", body)
      expectInvalid(response, path)
      expectError(response, 422, "invalidRequestData", { defined: true, message: "Input validation failed" })
    })

    it("spec with nodes/edges in the same body", async () => {
      const { api } = await newTenant("p109-validate-mixed")
      const response = await api.post("/v1/flows/validate", {
        spec: { formatVersion: 1, name: "x", steps: [{ type: "send", text: "a" }] },
        nodes: [],
        edges: [],
      })
      expectInvalid(response, "")
      expect((response.json as { data: { issues: Array<{ message: string }> } }).data.issues[0]?.message).toBe(
        'Cannot combine "nodes"/"edges" with "spec" in the same request body.',
      )
    })
  })
})
