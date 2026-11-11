// MailHog API client: find the mail sent to an address and pull links out of it.

import { config } from "./config.ts"
import { eventually } from "./eventually.ts"

export type MailhogMessage = {
  ID: string
  From: { Mailbox: string; Domain: string }
  To: Array<{ Mailbox: string; Domain: string }>
  Content: { Headers: Record<string, string[]>; Body: string }
  MIME: { Parts?: Array<{ Headers: Record<string, string[]>; Body: string }> } | null
  Created: string
}

type SearchResponse = { total: number; count: number; items: MailhogMessage[] }

export const searchTo = async (email: string): Promise<MailhogMessage[]> => {
  const url = new URL(`${config.mailhogUrl}/api/v2/search`)
  url.searchParams.set("kind", "to")
  url.searchParams.set("query", email)
  url.searchParams.set("limit", "50")
  const response = await fetch(url)
  if (!response.ok) {
    throw new Error(`MailHog search failed: ${response.status}`)
  }
  const body = (await response.json()) as SearchResponse
  // `kind=to` is a substring match; keep exact recipients only.
  return body.items.filter((item) =>
    item.To.some((to) => `${to.Mailbox}@${to.Domain}`.toLowerCase() === email.toLowerCase()),
  )
}

/** Waits until `count` messages to `email` exist and returns them (oldest first). */
export const waitForMail = async (
  email: string,
  options: { count?: number; timeoutMs?: number } = {},
): Promise<MailhogMessage[]> =>
  eventually(
    async () => {
      const items = await searchTo(email)
      if (items.length < (options.count ?? 1)) {
        throw new Error(`${items.length} message(s) to ${email}`)
      }
      return items.sort((a, b) => a.Created.localeCompare(b.Created))
    },
    { timeoutMs: options.timeoutMs ?? 15_000, intervalMs: 250, description: `mail to ${email}` },
  )

const header = (headers: Record<string, string[]> | undefined, name: string) =>
  Object.entries(headers ?? {}).find(([key]) => key.toLowerCase() === name)?.[1]?.[0] ?? ""

export const decodeQuotedPrintable = (input: string): string => {
  const softBreaksRemoved = input.replace(/=\r?\n/g, "")
  const bytes: number[] = []
  for (let index = 0; index < softBreaksRemoved.length; index += 1) {
    const char = softBreaksRemoved[index] ?? ""
    const hex = softBreaksRemoved.slice(index + 1, index + 3)
    if (char === "=" && /^[0-9A-Fa-f]{2}$/.test(hex)) {
      bytes.push(Number.parseInt(hex, 16))
      index += 2
    } else {
      bytes.push(...Buffer.from(char, "utf8"))
    }
  }
  return Buffer.from(bytes).toString("utf8")
}

const decodePart = (headers: Record<string, string[]> | undefined, body: string): string => {
  const encoding = header(headers, "content-transfer-encoding").toLowerCase()
  if (encoding === "quoted-printable") {
    return decodeQuotedPrintable(body)
  }
  if (encoding === "base64") {
    return Buffer.from(body.replace(/\s+/g, ""), "base64").toString("utf8")
  }
  return body
}

/** All decoded text of a message (every MIME part, or the single body). */
export const messageText = (message: MailhogMessage): string => {
  const parts = message.MIME?.Parts ?? []
  if (parts.length > 0) {
    return parts.map((part) => decodePart(part.Headers, part.Body)).join("\n")
  }
  return decodePart(message.Content.Headers, message.Content.Body)
}

export const messageSubject = (message: MailhogMessage): string =>
  header(message.Content.Headers, "subject")

const decodeHtmlEntities = (value: string): string =>
  value
    .replaceAll("&amp;", "&")
    .replaceAll("&#x3D;", "=")
    .replaceAll("&#61;", "=")
    .replaceAll("&quot;", '"')

/** Links in the message that start with `prefix`, decoded, without duplicates. */
export const extractLinks = (message: MailhogMessage, prefix: string): string[] => {
  const escaped = prefix.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")
  const pattern = new RegExp(`${escaped}[^\\s"'<>)]*`, "g")
  const found = messageText(message).match(pattern) ?? []
  return [...new Set(found.map(decodeHtmlEntities))]
}
