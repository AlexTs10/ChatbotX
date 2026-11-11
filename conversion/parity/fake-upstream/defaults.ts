// Built-in answers used when no script matches. Keep this list short: a test
// should script what it expects. Later units add per-host defaults here
// (Telegram getMe/setWebhook, Graph /me, ...), each as a pure function.

import {
  answerQuery,
  answersFor,
  dnsTypeName,
  recordDnsQuery,
  TYPE_A,
  TYPE_AAAA,
} from "./dns.ts"
import type { ScriptResponse } from "./store.ts"

export type DefaultRequest = {
  host: string
  method: string
  path: string
  query: URLSearchParams
  headers: Record<string, string>
  body: Buffer
  remoteAddress: string | undefined
}

export type DefaultHandler = {
  name: string
  handle: (request: DefaultRequest) => ScriptResponse | null
}

const DOH_HOSTS = new Set(["1.1.1.1", "1.0.0.1", "cloudflare-dns.com"])

const typeFromParam = (value: string | null): number => {
  if (!value) {
    return TYPE_A
  }
  if (/^\d+$/.test(value)) {
    return Number.parseInt(value, 10)
  }
  const upper = value.toUpperCase()
  if (upper === "AAAA") {
    return TYPE_AAAA
  }
  return upper === "A" ? TYPE_A : -1
}

/** DNS-over-HTTPS: JSON API (`?name=&type=`) and RFC 8484 wire format. */
const doh: DefaultHandler = {
  name: "default:doh",
  handle: (request) => {
    if (!DOH_HOSTS.has(request.host) || request.path !== "/dns-query") {
      return null
    }
    const wire =
      request.method === "POST"
        ? request.body
        : request.query.get("dns")
          ? Buffer.from(request.query.get("dns") ?? "", "base64url")
          : null
    if (wire) {
      return {
        status: 200,
        headers: { "content-type": "application/dns-message" },
        base64: answerQuery(wire, "doh", request.remoteAddress).toString(
          "base64",
        ),
      }
    }
    const name = (request.query.get("name") ?? "").toLowerCase().replace(/\.$/, "")
    const type = typeFromParam(request.query.get("type"))
    if (!name || type < 0) {
      return { status: 400, json: { error: "invalid name or type" } }
    }
    recordDnsQuery(name, type, "doh", request.remoteAddress)
    const answers = answersFor(name, type).map((data) => ({
      name,
      type,
      TTL: 30,
      data,
    }))
    return {
      status: 200,
      headers: { "content-type": "application/dns-json" },
      json: {
        Status: 0,
        TC: false,
        RD: true,
        RA: true,
        AD: false,
        CD: false,
        Question: [{ name, type }],
        ...(answers.length > 0 ? { Answer: answers } : {}),
        Comment: `parity fake upstream (${dnsTypeName(type)})`,
      },
    }
  },
}

export const defaultHandlers: DefaultHandler[] = [doh]
