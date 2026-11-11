// TLS material for the HTTPS listener. `gen-certs.sh` makes a test CA and one
// leaf certificate whose SANs are the lines of `hosts.txt`. A client that
// sends an SNI name outside that list gets the same leaf (so its handshake
// fails) and the fake records an `unknownSni` entry, which the interception
// self-check reports.

import { readFileSync } from "node:fs"
import tls from "node:tls"
import { store } from "./store.ts"

const IP_PATTERN = /^\d{1,3}(\.\d{1,3}){3}$/

export const readSanPatterns = (hostsFile: string): string[] =>
  readFileSync(hostsFile, "utf8")
    .split("\n")
    .map((line) => line.replace(/#.*/, "").trim().toLowerCase())
    .filter((line) => line.length > 0)

export const coveredBy = (patterns: string[], name: string): boolean => {
  const lower = name.toLowerCase()
  return patterns.some((pattern) => {
    if (pattern.startsWith("*.")) {
      const suffix = pattern.slice(1) // ".example.com"
      if (!lower.endsWith(suffix)) {
        return false
      }
      const label = lower.slice(0, -suffix.length)
      return label.length > 0 && !label.includes(".")
    }
    return pattern === lower
  })
}

export const createTlsOptions = (input: {
  tlsDir: string
  hostsFile: string
}): tls.TlsOptions => {
  const key = readFileSync(`${input.tlsDir}/leaf.key`)
  const cert = readFileSync(`${input.tlsDir}/leaf.crt`)
  const context = tls.createSecureContext({ key, cert })
  const patterns = readSanPatterns(input.hostsFile).filter(
    (pattern) => !IP_PATTERN.test(pattern),
  )
  return {
    key,
    cert,
    ALPNProtocols: ["http/1.1"],
    SNICallback: (servername, callback) => {
      if (!coveredBy(patterns, servername)) {
        store.addRecord({
          kind: "tls",
          scheme: "https",
          host: servername.toLowerCase(),
          method: "",
          path: "",
          rawQuery: "",
          query: {},
          headers: {},
          bodyText: "",
          bodyBase64: "",
          matchedScript: null,
          unmatched: true,
          unknownSni: servername.toLowerCase(),
        })
      }
      callback(null, context)
    },
  }
}
