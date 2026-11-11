// inboundWebhookChannels.list: the static channel registry an inbound webhook
// configuration is authored against (one entry per channel type, in the
// channel enum order).

import { describe, expect, it } from "vitest"
import { tenantApi } from "./helpers-webhooks.ts"

const ids = {
  pageId:
    "The numeric Facebook page id (`Inbox.sourceId` of a Messenger inbox, see `inboxes.list`).",
  igId: "The numeric Instagram account id (`sourceId` of an Instagram inbox).",
  phoneNumberId:
    "The WhatsApp Business phone number id from Meta (`sourceId` of a WhatsApp inbox) — exact, preferred.",
  displayPhoneNumber:
    "The WhatsApp number as people read it (e.g. +84 90 123 4567); only digits are compared, and two numbers sharing it make the request ambiguous.",
  botId: "The numeric Telegram bot id (`sourceId` of a Telegram inbox), not the @username.",
  oaId: "The Zalo Official Account id (`sourceId` of a Zalo inbox).",
  openId: "The connected TikTok account's open id, not the username or a customer's id.",
  threadsUserId: "The numeric Threads account id (`sourceId` of a Threads inbox).",
}

const users = {
  psid: "The Messenger page-scoped user id of the person.",
  igsid: "The Instagram-scoped user id of the person.",
  telegramChatId: "The Telegram chat id of the person.",
  zaloUserId: "The Zalo user id of the person.",
  tiktokCustomerOpenId: "The TikTok open id of the customer (not the connected account).",
  threadsUsername: "The person's Threads username; matched lower-cased.",
  whatsappId: "The WhatsApp user id (`wa_id`): the phone number's digits, with or without a leading +.",
  apiSourceId: "The caller-defined contact id used with the API channel.",
}

const inboxId = (kind: keyof typeof ids, strategy: string, examplePath: string) => ({
  kind,
  strategy,
  examplePath,
  description: ids[kind],
})
const userId = (kind: keyof typeof users, examplePath: string) => ({ kind, examplePath, description: users[kind] })

const EXPECTED = [
  { channel: "omnichannel", payloadResolvable: false, inboxIdentifiers: [], channelUserId: null, creationIdentities: [] },
  {
    channel: "webchat",
    payloadResolvable: false,
    inboxIdentifiers: [],
    channelUserId: null,
    creationIdentities: ["phone", "email"],
  },
  {
    channel: "messenger",
    payloadResolvable: true,
    inboxIdentifiers: [inboxId("pageId", "inboxSourceId", "$.entry[0].id")],
    channelUserId: userId("psid", "$.sender.id"),
    creationIdentities: ["channelUserId"],
  },
  {
    channel: "whatsapp",
    payloadResolvable: true,
    inboxIdentifiers: [
      inboxId("phoneNumberId", "inboxSourceId", "$.metadata.phone_number_id"),
      inboxId("displayPhoneNumber", "whatsappDisplayPhone", "$.metadata.display_phone_number"),
    ],
    channelUserId: userId("whatsappId", "$.contacts[0].wa_id"),
    creationIdentities: ["phone"],
  },
  {
    channel: "zalo",
    payloadResolvable: true,
    inboxIdentifiers: [inboxId("oaId", "inboxSourceId", "$.oa_id")],
    channelUserId: userId("zaloUserId", "$.user_id"),
    creationIdentities: ["channelUserId"],
  },
  { channel: "smtp", payloadResolvable: false, inboxIdentifiers: [], channelUserId: null, creationIdentities: ["email"] },
  {
    channel: "telegram",
    payloadResolvable: true,
    inboxIdentifiers: [inboxId("botId", "inboxSourceId", "$.bot.id")],
    channelUserId: userId("telegramChatId", "$.chat.id"),
    creationIdentities: ["channelUserId"],
  },
  {
    channel: "instagram",
    payloadResolvable: true,
    inboxIdentifiers: [inboxId("igId", "inboxSourceId", "$.entry[0].id")],
    channelUserId: userId("igsid", "$.sender.id"),
    creationIdentities: ["channelUserId"],
  },
  {
    channel: "threads",
    payloadResolvable: true,
    inboxIdentifiers: [inboxId("threadsUserId", "inboxSourceId", "$.user_id")],
    channelUserId: userId("threadsUsername", "$.user.username"),
    creationIdentities: ["channelUserId"],
  },
  {
    channel: "tiktok",
    payloadResolvable: true,
    inboxIdentifiers: [inboxId("openId", "connectionSourceId", "$.open_id")],
    channelUserId: userId("tiktokCustomerOpenId", "$.customer.open_id"),
    creationIdentities: ["channelUserId"],
  },
  {
    channel: "api",
    payloadResolvable: false,
    inboxIdentifiers: [],
    channelUserId: userId("apiSourceId", "$.contact.id"),
    creationIdentities: ["channelUserId"],
  },
]

describe("inboundWebhookChannels.list", () => {
  it("returns the rules of every channel, in channel order", async () => {
    const { api } = await tenantApi("p109-iwh-channels")
    const response = await api.get<unknown[]>("/v1/inbound-webhook-channels")
    expect(response.status, response.text).toBe(200)
    expect(response.json).toEqual(EXPECTED)
  })

  it("is the same for every workspace and ignores query parameters", async () => {
    const { api } = await tenantApi("p109-iwh-channels-2")
    const response = await api.get<unknown[]>("/v1/inbound-webhook-channels?page=2&perPage=1")
    expect(response.status, response.text).toBe(200)
    expect(response.json).toEqual(EXPECTED)
  })
})
