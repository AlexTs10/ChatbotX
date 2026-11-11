// In-memory state of the fake upstream: request records, scripted responses
// and DNS overrides. Reset only through the admin API.

export type RecordKind = "http" | "dns" | "tls"

export type UpstreamRecord = {
  id: number
  ts: string
  kind: RecordKind
  scheme: "http" | "https" | "dns"
  host: string
  method: string
  path: string
  rawQuery: string
  query: Record<string, string | string[]>
  headers: Record<string, string>
  bodyText: string
  bodyBase64: string
  matchedScript: string | null
  /** True when no script and no built-in default answered the request. */
  unmatched: boolean
  /** TLS only: SNI name that is not covered by the leaf certificate. */
  unknownSni?: string
  /** DNS only: query type name (A, AAAA, ...). */
  dnsType?: string
  /** DNS only: transport of the query. */
  dnsTransport?: "udp" | "tcp" | "doh"
  remoteAddress?: string
}

export type ScriptMatch = {
  host: string
  method?: string
  path?: string
  pathPrefix?: string
  pathRegex?: string
  bodyIncludes?: string
}

export type ScriptResponse = {
  status: number
  headers?: Record<string, string>
  json?: unknown
  text?: string
  base64?: string
  delayMs?: number
}

export type Script = {
  id: string
  match: ScriptMatch
  respond: ScriptResponse
  /** Number of times the script may answer; unlimited when absent. */
  times?: number
  used: number
}

export type DnsOverride = { name: string; a: string[]; aaaa: string[] }

type Waiter = { check: () => boolean; resolve: () => void }

let nextRecordId = 1
let nextScriptId = 1
let records: UpstreamRecord[] = []
let scripts: Script[] = []
let dnsOverrides = new Map<string, DnsOverride>()
let waiters: Waiter[] = []

export const store = {
  reset(): void {
    records = []
    scripts = []
    dnsOverrides = new Map()
    for (const waiter of waiters) {
      waiter.resolve()
    }
    waiters = []
  },

  addRecord(record: Omit<UpstreamRecord, "id" | "ts">): UpstreamRecord {
    const full: UpstreamRecord = {
      id: nextRecordId++,
      ts: new Date().toISOString(),
      ...record,
    }
    records.push(full)
    const pending = waiters
    waiters = []
    for (const waiter of pending) {
      if (waiter.check()) {
        waiter.resolve()
      } else {
        waiters.push(waiter)
      }
    }
    return full
  },

  records(): UpstreamRecord[] {
    return records
  },

  addScript(input: {
    match: ScriptMatch
    respond: ScriptResponse
    times?: number
  }): Script {
    if (!input.match?.host) {
      throw new Error("script.match.host is required")
    }
    if (typeof input.respond?.status !== "number") {
      throw new Error("script.respond.status is required")
    }
    const script: Script = {
      id: `s${nextScriptId++}`,
      match: { ...input.match, host: input.match.host.toLowerCase() },
      respond: input.respond,
      times: input.times,
      used: 0,
    }
    scripts.push(script)
    return script
  },

  /** Newest script first, so a later script overrides an earlier one. */
  findScript(request: {
    host: string
    method: string
    path: string
    bodyText: string
  }): Script | null {
    for (let index = scripts.length - 1; index >= 0; index -= 1) {
      const script = scripts[index]
      if (!script) {
        continue
      }
      if (script.times !== undefined && script.used >= script.times) {
        continue
      }
      if (matches(script.match, request)) {
        script.used += 1
        return script
      }
    }
    return null
  },

  setDnsOverride(input: { name: string; a?: string[]; aaaa?: string[] }) {
    const name = input.name.toLowerCase().replace(/\.$/, "")
    dnsOverrides.set(name, { name, a: input.a ?? [], aaaa: input.aaaa ?? [] })
  },

  dnsOverride(name: string): DnsOverride | undefined {
    return dnsOverrides.get(name.toLowerCase().replace(/\.$/, ""))
  },

  waitFor(check: () => boolean, timeoutMs: number): Promise<boolean> {
    if (check()) {
      return Promise.resolve(true)
    }
    return new Promise((resolve) => {
      const timer = setTimeout(() => {
        waiters = waiters.filter((waiter) => waiter !== entry)
        resolve(check())
      }, timeoutMs)
      const entry: Waiter = {
        check,
        resolve: () => {
          clearTimeout(timer)
          resolve(check())
        },
      }
      waiters.push(entry)
    })
  },
}

const matches = (
  match: ScriptMatch,
  request: { host: string; method: string; path: string; bodyText: string },
): boolean => {
  if (match.host !== request.host) {
    return false
  }
  if (match.method && match.method.toUpperCase() !== request.method) {
    return false
  }
  if (match.path !== undefined && match.path !== request.path) {
    return false
  }
  if (match.pathPrefix && !request.path.startsWith(match.pathPrefix)) {
    return false
  }
  if (match.pathRegex && !new RegExp(match.pathRegex).test(request.path)) {
    return false
  }
  if (match.bodyIncludes && !request.bodyText.includes(match.bodyIncludes)) {
    return false
  }
  return true
}

export type RecordFilter = {
  kind?: string
  host?: string
  method?: string
  pathPrefix?: string
  since?: number
  unmatched?: boolean
}

export const filterRecords = (filter: RecordFilter): UpstreamRecord[] =>
  records.filter(
    (record) =>
      (filter.kind === undefined || record.kind === filter.kind) &&
      (filter.host === undefined || record.host === filter.host) &&
      (filter.method === undefined ||
        record.method === filter.method.toUpperCase()) &&
      (filter.pathPrefix === undefined ||
        record.path.startsWith(filter.pathPrefix)) &&
      (filter.since === undefined || record.id > filter.since) &&
      (filter.unmatched === undefined ||
        (record.unmatched || record.unknownSni !== undefined) ===
          filter.unmatched),
  )
