// Normalization rules (spec P1.1 section 7). Both stacks produce values that
// differ only in generated data (ids, times, tokens, signatures). These helpers
// replace that data with stable placeholders so a value can be compared to a
// snapshot recorded on the original stack.

import { expect } from "vitest"

const SNOWFLAKE = /^\d{15,20}$/
const ISO_TIMESTAMP = /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(\.\d{1,9})?(Z|[+-]\d{2}:?\d{2})$/
const JWT = /^eyJ[\w-]*\.[\w-]+\.[\w-]*$/
const SEQ = /^\d+-\d+$/
const TOKEN_PATTERNS: Array<[RegExp, string]> = [
  [/^cbx_ws_[\w-]+$/, "<cbx_ws>"],
  [/^cbx_api_[\w-]+$/, "<cbx_api>"],
  [/^cbx_wh_[\w-]+$/, "<cbx_wh>"],
  [/^whsec_[\w-]+$/, "<whsec>"],
]
/** Query parameters that carry signatures or expiry and differ per call. */
const SIGNATURE_PARAMS = new Set([
  "x-amz-algorithm",
  "x-amz-credential",
  "x-amz-date",
  "x-amz-expires",
  "x-amz-signature",
  "x-amz-signedheaders",
  "x-amz-security-token",
  "x-amz-checksum-crc32",
  "x-id",
  "signature",
  "expires",
  "token",
])
const TIMESTAMP_SLACK_MS = 5_000

const isIdKey = (key: string | undefined): boolean =>
  key !== undefined && (key === "id" || /Id$/.test(key) || /Ids$/.test(key) || key === "ids")

export type NormalizeOptions = {
  /** Keys whose values stay as they are (for values the test set itself). */
  keep?: string[]
  /** Extra exact-value replacements, e.g. {[workspaceId]: "<workspace>"}. */
  replace?: Record<string, string>
  /** Check that every timestamp lies in [testStart - 5 s, now + 5 s]. Default true. */
  checkTimestamps?: boolean
}

/**
 * One normalizer per test: the same id gets the same label (`<id:1>`) in every
 * value the test normalizes, so references between objects stay checked.
 */
export class Normalizer {
  private readonly ids = new Map<string, string>()
  readonly testStart: number

  constructor(testStart = Date.now()) {
    this.testStart = testStart
  }

  idLabel(id: string): string {
    let label = this.ids.get(id)
    if (!label) {
      label = `<id:${this.ids.size + 1}>`
      this.ids.set(id, label)
    }
    return label
  }

  normalize<T>(value: T, options: NormalizeOptions = {}): unknown {
    const errors: string[] = []
    const result = this.walk(value, undefined, options, errors, "$")
    if (errors.length > 0) {
      throw new Error(`normalize: ${errors.join("; ")}`)
    }
    return result
  }

  private walk(
    value: unknown,
    key: string | undefined,
    options: NormalizeOptions,
    errors: string[],
    path: string,
  ): unknown {
    if (key !== undefined && options.keep?.includes(key)) {
      return value
    }
    if (Array.isArray(value)) {
      // `*Ids` arrays: label each element as an id.
      return value.map((item, index) =>
        this.walk(item, key !== undefined && /Ids$|^ids$/.test(key) ? "id" : undefined, options, errors, `${path}[${index}]`),
      )
    }
    if (value && typeof value === "object") {
      const output: Record<string, unknown> = {}
      for (const childKey of Object.keys(value as object).sort()) {
        output[childKey] = this.walk(
          (value as Record<string, unknown>)[childKey],
          childKey,
          options,
          errors,
          `${path}.${childKey}`,
        )
      }
      return output
    }
    if (typeof value === "string") {
      return this.normalizeString(value, key, options, errors, path)
    }
    return value
  }

  private normalizeString(
    value: string,
    key: string | undefined,
    options: NormalizeOptions,
    errors: string[],
    path: string,
  ): string {
    const replaced = options.replace?.[value]
    if (replaced !== undefined) {
      return replaced
    }
    if (SNOWFLAKE.test(value) && isIdKey(key)) {
      return this.idLabel(value)
    }
    if (ISO_TIMESTAMP.test(value)) {
      if (options.checkTimestamps !== false) {
        const time = Date.parse(value)
        // Expiry times lie in the future; only their lower bound is checked.
        const isExpiry = key !== undefined && /expire/i.test(key)
        if (
          Number.isNaN(time) ||
          time < this.testStart - TIMESTAMP_SLACK_MS ||
          (!isExpiry && time > Date.now() + TIMESTAMP_SLACK_MS)
        ) {
          errors.push(`${path}: timestamp ${value} is outside the test window`)
        }
      }
      return "<ts>"
    }
    for (const [pattern, label] of TOKEN_PATTERNS) {
      if (pattern.test(value)) {
        return label
      }
    }
    if (JWT.test(value)) {
      return "<jwt>"
    }
    if (key === "seq" && SEQ.test(value)) {
      return "<seq>"
    }
    if (/^https?:\/\//.test(value)) {
      return normalizeUrl(value)
    }
    return value
  }
}

/** Keeps scheme, host and path; drops signature and expiry query parameters. */
export const normalizeUrl = (value: string): string => {
  let url: URL
  try {
    url = new URL(value)
  } catch {
    return value
  }
  for (const name of [...url.searchParams.keys()]) {
    if (SIGNATURE_PARAMS.has(name.toLowerCase())) {
      url.searchParams.delete(name)
    }
  }
  url.searchParams.sort()
  return url.toString()
}

/** Headers that parity tests compare. All other headers differ by stack. */
export const comparableHeaders = (headers: Headers): Record<string, string> => {
  const result: Record<string, string> = {}
  const contentType = headers.get("content-type")
  if (contentType) {
    result["content-type"] = (contentType.split(";")[0] ?? "").trim().toLowerCase()
  }
  for (const name of ["location", "cache-control", "allow", "www-authenticate"]) {
    const value = headers.get(name)
    if (value !== null) {
      result[name] = name === "location" ? normalizeUrl(value) : value
    }
  }
  if (headers.get("retry-after") !== null) {
    result["retry-after"] = "<present>"
  }
  return result
}

/** Sorts a list by a stable key, for lists whose order the API does not define. */
export const unordered = <T>(list: T[], key: (item: T) => string | number): T[] =>
  [...list].sort((a, b) => {
    const left = key(a)
    const right = key(b)
    return left < right ? -1 : left > right ? 1 : 0
  })

/** The parts of an oRPC / public API error that both stacks must match. */
export const comparableError = (
  body: unknown,
  options: { keepMessage?: boolean } = {},
): Record<string, unknown> => {
  const error = (body ?? {}) as Record<string, unknown>
  const result: Record<string, unknown> = {}
  for (const key of ["defined", "code", "status", "data"]) {
    if (key in error) {
      result[key] = error[key]
    }
  }
  if (options.keepMessage && "message" in error) {
    result.message = error.message
  }
  return result
}

export const decodeJwt = (token: string): { header: Record<string, unknown>; payload: Record<string, unknown> } => {
  const [header = "", payload = ""] = token.split(".")
  const decode = (part: string) => JSON.parse(Buffer.from(part, "base64url").toString("utf8"))
  return { header: decode(header), payload: decode(payload) }
}

/** `expect(normalized value)`; use with `.toMatchSnapshot()` or `.toEqual()`. */
export const expectNormalized = (
  normalizer: Normalizer,
  value: unknown,
  options?: NormalizeOptions,
) => expect(normalizer.normalize(value, options))
