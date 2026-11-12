// minigames.create / get / list / update (PUT) / updatePartial (PATCH) /
// setEnabled / delete / deleteMany / listPlays / listPlayers.
//
// Players and plays are written by the public play page (signed token), which
// has no public API path: they come from the P1.12 fixture extension.

import { describe, expect, it } from "vitest"
import {
  type Api,
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

type Minigame = Record<string, unknown> & {
  id: string
  name: string
  type: string
  enabled: boolean
  playUrl: string
  generalSettings: Record<string, unknown> & { name: string }
  prizeSettings: Record<string, unknown>
}

const MINIGAME_KEYS = [
  "appearance",
  "createdAt",
  "enabled",
  "generalSettings",
  "id",
  "name",
  "nonWinningMessageSettings",
  "participantsCount",
  "playUrl",
  "playerSettings",
  "playsCount",
  "prizeSettings",
  "sharesCount",
  "type",
  "updatedAt",
  "winnersCount",
  "winningMessageSettings",
]

const body = (name: string, overrides: Record<string, unknown> = {}) => ({
  type: "luckyWheel",
  generalSettings: {
    name,
    playedAtFrom: "2026-01-01T00:00:00.000Z",
    playedAtTo: "2027-12-31T23:59:59.000Z",
  },
  appearance: { backgroundImage: {}, prizeDescriptionImage: {}, startButtonImage: {} },
  playerSettings: { resetPolicy: "never" },
  prizeSettings: {
    prizes: [{ id: "prize-1", name: "Free coffee", icon: { mode: "url", url: "https://example.com/c.png" }, winRate: 30, quantity: 5 }],
    nonWinning: { title: "Try again", loseRate: 70, loseImage: {} },
  },
  winningMessageSettings: { title: "You won {{prize_name}}" },
  nonWinningMessageSettings: {},
  ...overrides,
})

const create = async (api: Api, name = uniqueName("Wheel"), overrides: Record<string, unknown> = {}): Promise<Minigame> => {
  const response = await api.post<Minigame>("/v1/minigames", body(name, overrides))
  expect(response.status, response.text).toBe(201)
  return response.json
}

describe("minigames.create", () => {
  it("201 with every settings group filled with its defaults and a play URL", async () => {
    const { api } = await newTenant("p112-mg-create")
    const name = uniqueName("Wheel")
    const response = await api.post<Minigame>("/v1/minigames", body(`  ${name}  `))
    expect(response.status, response.text).toBe(201)
    const game = response.json
    expect(keysOf(game)).toEqual(MINIGAME_KEYS)
    expect(game.id).toMatch(SNOWFLAKE)
    expect(game).toEqual({
      id: game.id,
      name,
      type: "luckyWheel",
      enabled: true,
      playUrl: `http://app/minigames?minigameId=${game.id}&token={{minigame_play_token}}`,
      generalSettings: {
        name,
        showName: false,
        playedAtFrom: "2026-01-01T00:00:00.000Z",
        playedAtTo: "2027-12-31T23:59:59.000Z",
        rulesDescription: "",
        openerTagIds: [],
        playerTagIds: [],
        newFriendTagIds: [],
      },
      appearance: {
        backgroundColor: "#F5A623",
        machineColor: "#4A90D9",
        decorativeColor: "#FFFFFF",
        ruleTextColor: "#000000",
        backgroundImage: { mode: "file", url: "" },
        prizeDescriptionImage: { mode: "file", url: "" },
        startButtonImage: { mode: "file", url: "" },
      },
      playerSettings: {
        resetPolicy: "never",
        drawsPerPerson: 1,
        maxSharesPerPerson: 0,
        sharingFlowId: null,
        sharingNodeId: null,
      },
      prizeSettings: {
        prizes: [
          { id: "prize-1", name: "Free coffee", icon: { mode: "url", url: "https://example.com/c.png" }, winRate: 30, quantity: 5 },
        ],
        nonWinning: { title: "Try again", loseRate: 70, loseImage: { mode: "file", url: "" } },
        prizeNameCustomFieldId: null,
      },
      winningMessageSettings: {
        title: "You won {{prize_name}}",
        description: "",
        acceptButtonText: "",
        shareButtonText: "",
        shareButtonDescription: "",
        outcomeMessage: { enabled: false, mode: "text", text: "" },
      },
      nonWinningMessageSettings: {
        title: "",
        description: "",
        shareButtonText: "",
        shareButtonDescription: "",
        outcomeMessage: { enabled: false, mode: "text", text: "" },
      },
      playsCount: 0,
      participantsCount: 0,
      winnersCount: 0,
      sharesCount: 0,
      createdAt: expect.stringMatching(ISO),
      updatedAt: expect.stringMatching(ISO),
    })
  })

  it("409 nameAlreadyExists for a taken name; another workspace may reuse it", async () => {
    const { api } = await newTenant("p112-mg-dup")
    const { api: other } = await newTenant("p112-mg-dup-other")
    const game = await create(api)
    expectError(await api.post("/v1/minigames", body(game.name)), 409, "nameAlreadyExists", {
      defined: true,
      message: "Minigame name already exists",
    })
    expect((await other.post("/v1/minigames", body(game.name))).status).toBe(201)
  })

  it("422 for a bad type, odds that do not add up to 100%, a reversed date range and a missing name", async () => {
    const { api } = await newTenant("p112-mg-invalid")
    expectInvalid(await api.post("/v1/minigames", body("x", { type: "bingo" })), "type")
    expectInvalid(
      await api.post(
        "/v1/minigames",
        body("x", {
          prizeSettings: {
            prizes: [{ id: "p", name: "P", icon: {}, winRate: 30 }],
            nonWinning: { title: "No", loseRate: 60, loseImage: {} },
          },
        }),
      ),
      "prizeSettings.nonWinning.loseRate",
    )
    expectInvalid(
      await api.post(
        "/v1/minigames",
        body("x", {
          generalSettings: { name: "x", playedAtFrom: "2027-01-01T00:00:00.000Z", playedAtTo: "2026-01-01T00:00:00.000Z" },
        }),
      ),
      "generalSettings.playedAtTo",
    )
    expectInvalid(
      await api.post(
        "/v1/minigames",
        body("x", { generalSettings: { name: " ", playedAtFrom: "2026-01-01T00:00:00.000Z", playedAtTo: "2026-02-01T00:00:00.000Z" } }),
      ),
      "generalSettings.name",
    )
    expectInvalid(await api.post("/v1/minigames", body("x", { playerSettings: { resetPolicy: "daily" } })), "playerSettings.resetPolicy")
    expectInvalid(await api.post("/v1/minigames", {}), "type")
    const listed = await api.get<{ data: unknown[] }>("/v1/minigames")
    expect(listed.json.data).toEqual([])
  })
})

describe("minigames.get / list", () => {
  it("get returns the created game; 404 for an unknown id or another workspace's game", async () => {
    const { api } = await newTenant("p112-mg-get")
    const { api: other } = await newTenant("p112-mg-get-other")
    const game = await create(api)
    const response = await api.get<Minigame>(`/v1/minigames/${game.id}`)
    expect(response.status, response.text).toBe(200)
    expect(response.json).toEqual(game)
    expectNotFound(await other.get(`/v1/minigames/${game.id}`), "Minigame not found")
    expectNotFound(await api.get(`/v1/minigames/${UNKNOWN_ID}`), "Minigame not found")
  })

  it("lists newest first with pages and a name filter", async () => {
    const { api } = await newTenant("p112-mg-list")
    const token = uniqueName("lst")
    const games: Minigame[] = []
    for (let index = 0; index < 3; index += 1) {
      games.push(await create(api, `${token} game ${index}`))
    }
    const solo = await create(api, uniqueName("Solo"))
    const page1 = await api.get<{ data: Minigame[]; pageCount: number }>("/v1/minigames", { query: { perPage: 3 } })
    expect(page1.status, page1.text).toBe(200)
    expect(keysOf(page1.json)).toEqual(["data", "pageCount"])
    expect(page1.json.pageCount).toBe(2)
    expect(page1.json.data.map((game) => game.id)).toEqual([solo.id, games[2]?.id, games[1]?.id])
    expect(page1.json.data[0]).toEqual(solo)
    const page2 = await api.get<{ data: Minigame[] }>("/v1/minigames", { query: { perPage: 3, page: 2 } })
    expect(page2.json.data.map((game) => game.id)).toEqual([games[0]?.id])

    const filtered = await api.get<{ data: Minigame[]; pageCount: number }>("/v1/minigames", {
      query: { name: token.toUpperCase() },
    })
    expect(filtered.json.data.map((game) => game.id)).toEqual([games[2]?.id, games[1]?.id, games[0]?.id])
    expect(filtered.json.pageCount).toBe(1)
    expectInvalid(await api.get("/v1/minigames", { query: { perPage: 51 } }), "perPage")
  })
})

describe("minigames.update (PUT) / updatePartial (PATCH)", () => {
  it("PUT replaces every group; the name follows generalSettings.name", async () => {
    const { api } = await newTenant("p112-mg-put")
    const game = await create(api)
    const newName = uniqueName("Jackpot")
    const response = await api.put<Minigame>(
      `/v1/minigames/${game.id}`,
      body(newName, {
        type: "jackpot",
        prizeSettings: { prizes: [], nonWinning: { title: "Always lose", loseRate: 100, loseImage: {} } },
      }),
    )
    expect(response.status, response.text).toBe(200)
    expect(response.json).toMatchObject({
      id: game.id,
      name: newName,
      type: "jackpot",
      generalSettings: { name: newName },
      prizeSettings: { prizes: [], nonWinning: { title: "Always lose", loseRate: 100 } },
    })
    expect((await api.get<Minigame>(`/v1/minigames/${game.id}`)).json).toEqual(response.json)
  })

  it("PATCH changes only the sent groups and needs at least one", async () => {
    const { api } = await newTenant("p112-mg-patch")
    const game = await create(api)
    const response = await api.patch<Minigame>(`/v1/minigames/${game.id}`, {
      appearance: { backgroundColor: "#000000", backgroundImage: {}, prizeDescriptionImage: {}, startButtonImage: {} },
    })
    expect(response.status, response.text).toBe(200)
    expect(response.json).toEqual({
      ...game,
      appearance: { ...(game.appearance as object), backgroundColor: "#000000" },
      updatedAt: expect.stringMatching(ISO),
    })
    expectInvalid(await api.patch(`/v1/minigames/${game.id}`, {}), "")
  })

  it("404 for unknown or foreign ids; 409 when renamed to a taken name", async () => {
    const { api } = await newTenant("p112-mg-update-err")
    const { api: other } = await newTenant("p112-mg-update-err-other")
    const game = await create(api)
    const taken = await create(api)
    const foreign = await create(other)
    expectNotFound(await api.put(`/v1/minigames/${UNKNOWN_ID}`, body("x")), "Minigame not found")
    expectNotFound(await api.put(`/v1/minigames/${foreign.id}`, body("x")), "Minigame not found")
    expectNotFound(await api.patch(`/v1/minigames/${foreign.id}`, { type: "gashapon" }), "Minigame not found")
    expectError(await api.put(`/v1/minigames/${game.id}`, body(taken.name)), 409, "nameAlreadyExists")
    expectError(
      await api.patch(`/v1/minigames/${game.id}`, {
        generalSettings: { name: taken.name, playedAtFrom: "2026-01-01T00:00:00.000Z", playedAtTo: "2026-01-02T00:00:00.000Z" },
      }),
      409,
      "nameAlreadyExists",
    )
    expect((await other.get<Minigame>(`/v1/minigames/${foreign.id}`)).json.type).toBe("luckyWheel")
  })
})

describe("minigames.setEnabled", () => {
  it("turns the game off and on; 404 for an unknown or foreign id", async () => {
    const { api } = await newTenant("p112-mg-enabled")
    const { api: other } = await newTenant("p112-mg-enabled-other")
    const game = await create(api)
    const off = await api.patch<Minigame>(`/v1/minigames/${game.id}/enabled`, { enabled: false })
    expect(off.status, off.text).toBe(200)
    expect(off.json).toMatchObject({ id: game.id, enabled: false, playUrl: game.playUrl })
    expect((await api.get<Minigame>(`/v1/minigames/${game.id}`)).json.enabled).toBe(false)
    const on = await api.patch<Minigame>(`/v1/minigames/${game.id}/enabled`, { enabled: true })
    expect(on.json.enabled).toBe(true)
    expectNotFound(await other.patch(`/v1/minigames/${game.id}/enabled`, { enabled: false }), "Minigame not found")
    expectInvalid(await api.patch(`/v1/minigames/${game.id}/enabled`, {}), "enabled")
  })
})

describe("minigames.delete / deleteMany", () => {
  it("delete is 204 and 404s for unknown and foreign ids", async () => {
    const { api } = await newTenant("p112-mg-delete")
    const { api: other } = await newTenant("p112-mg-delete-other")
    const game = await create(api)
    const foreign = await create(other)
    const response = await api.delete(`/v1/minigames/${game.id}`)
    expect(response.status, response.text).toBe(204)
    expect(response.text).toBe("")
    expectNotFound(await api.get(`/v1/minigames/${game.id}`), "Minigame not found")
    expectNotFound(await api.delete(`/v1/minigames/${game.id}`), "Minigame not found")
    expectNotFound(await api.delete(`/v1/minigames/${foreign.id}`), "Minigame not found")
    expect((await other.get(`/v1/minigames/${foreign.id}`)).status).toBe(200)
  })

  it("deleteMany is 204, deletes only this workspace's games and ignores unknown ids", async () => {
    const { api } = await newTenant("p112-mg-bulk")
    const { api: other } = await newTenant("p112-mg-bulk-other")
    const first = await create(api)
    const second = await create(api)
    const kept = await create(api)
    const foreign = await create(other)
    const response = await api.post("/v1/minigames/bulk-delete", { ids: [first.id, second.id, foreign.id, UNKNOWN_ID] })
    expect(response.status, response.text).toBe(204)
    const listed = await api.get<{ data: Minigame[] }>("/v1/minigames")
    expect(listed.json.data.map((game) => game.id)).toEqual([kept.id])
    expect((await other.get(`/v1/minigames/${foreign.id}`)).status).toBe(200)
    expect((await api.post("/v1/minigames/bulk-delete", { ids: [] })).status).toBe(204)
    expectInvalid(await api.post("/v1/minigames/bulk-delete", { ids: ["abc"] }), "ids.0")
  })
})

describe("minigames.listPlayers / listPlays", () => {
  it("players are newest-played first with their contact; plays are one contact's draws, newest first", async () => {
    const { api } = await newTenant("p112-mg-players")
    const game = await create(api)
    const token = uniqueName("Plr")
    const firstContact = await newContact(api, `${token}a`)
    const secondContact = await newContact(api, `${token}b`)
    const thirdContact = await newContact(api, uniqueName("Other"))
    const fixtures = await ext()
    const now = Date.now()
    const iso = (offsetMs: number) => new Date(now + offsetMs).toISOString()
    const first = await fixtures.createMinigamePlayer({
      minigameId: game.id,
      contactId: firstContact,
      played: 2,
      remaining: 0,
      sharesCount: 1,
      openedAt: iso(-3_000),
      lastPlayedAt: iso(-1_000),
    })
    const second = await fixtures.createMinigamePlayer({
      minigameId: game.id,
      contactId: secondContact,
      played: 1,
      remaining: 1,
      openedAt: iso(-2_000),
      lastPlayedAt: iso(-500),
    })
    const third = await fixtures.createMinigamePlayer({
      minigameId: game.id,
      contactId: thirdContact,
      played: 0,
      remaining: 1,
      openedAt: iso(-1_500),
      lastPlayedAt: iso(-1_500),
    })

    const response = await api.get<{ data: Array<Record<string, unknown>>; pageCount: number }>(
      `/v1/minigames/${game.id}/players`,
      { query: { perPage: 2 } },
    )
    expect(response.status, response.text).toBe(200)
    expect(response.json.pageCount).toBe(2)
    expect(response.json.data.map((player) => player.id)).toEqual([second.playerId, first.playerId])
    expect(response.json.data[1]).toEqual({
      id: first.playerId,
      contactId: firstContact,
      contactInboxId: null,
      played: 2,
      remaining: 0,
      sharesCount: 1,
      openedAt: iso(-3_000),
      lastPlayedAt: iso(-1_000),
      contact: {
        id: firstContact,
        fullName: expect.stringContaining(`${token}a`),
        firstName: `${token}a`,
        lastName: "Parity",
        avatar: null,
      },
    })
    const page2 = await api.get<{ data: Array<{ id: string }> }>(`/v1/minigames/${game.id}/players`, {
      query: { perPage: 2, page: 2 },
    })
    expect(page2.json.data.map((player) => player.id)).toEqual([third.playerId])
    const named = await api.get<{ data: Array<{ id: string }>; pageCount: number }>(`/v1/minigames/${game.id}/players`, {
      query: { name: token },
    })
    expect(named.json.data.map((player) => player.id)).toEqual([second.playerId, first.playerId])

    const plays = [
      await fixtures.createMinigamePlay({ minigameId: game.id, contactId: firstContact, isWinning: false, createdAt: iso(-900) }),
      await fixtures.createMinigamePlay({
        minigameId: game.id,
        contactId: firstContact,
        isWinning: true,
        prizeId: "prize-1",
        prizeName: "Free coffee",
        createdAt: iso(-800),
      }),
    ]
    await fixtures.createMinigamePlay({ minigameId: game.id, contactId: secondContact, isWinning: false })
    const listed = await api.get<{ data: unknown[] }>(`/v1/minigames/${game.id}/plays`, { query: { contactId: firstContact } })
    expect(listed.status, listed.text).toBe(200)
    expect(listed.json).toEqual({
      data: [
        { id: plays[1]?.playId, isWinning: true, prizeName: "Free coffee", createdAt: iso(-800) },
        { id: plays[0]?.playId, isWinning: false, prizeName: null, createdAt: iso(-900) },
      ],
    })
    const none = await api.get(`/v1/minigames/${game.id}/plays`, { query: { contactId: thirdContact } })
    expect(none.json).toEqual({ data: [] })
  })

  it("404 for an unknown or foreign game; 422 without contactId", async () => {
    const { api } = await newTenant("p112-mg-players-404")
    const { api: other } = await newTenant("p112-mg-players-404-other")
    const foreign = await create(other)
    expectNotFound(await api.get(`/v1/minigames/${UNKNOWN_ID}/players`), "Minigame not found")
    expectNotFound(await api.get(`/v1/minigames/${foreign.id}/players`), "Minigame not found")
    expectNotFound(await api.get(`/v1/minigames/${foreign.id}/plays`, { query: { contactId: UNKNOWN_ID } }), "Minigame not found")
    expectInvalid(await api.get(`/v1/minigames/${foreign.id}/plays`), "contactId")
    const empty = await other.get(`/v1/minigames/${foreign.id}/players`)
    expect(empty.json).toEqual({ data: [], pageCount: 0 })
  })
})
