// Minimal DNS server (RFC 1035 wire format, UDP + TCP). Every A query is
// answered with the fake upstream address, unless the admin API set an
// override for the name. AAAA and all other types get an empty NOERROR answer,
// so clients fall back to IPv4.

import dgram from "node:dgram"
import net from "node:net"
import { store } from "./store.ts"

export const FAKE_UPSTREAM_IP = process.env.FAKE_UPSTREAM_IP ?? "1.1.1.1"

const TYPE_A = 1
const TYPE_AAAA = 28
const TYPE_NAMES: Record<number, string> = {
  1: "A",
  2: "NS",
  5: "CNAME",
  6: "SOA",
  12: "PTR",
  15: "MX",
  16: "TXT",
  28: "AAAA",
  33: "SRV",
  65: "HTTPS",
}

export const dnsTypeName = (type: number): string =>
  TYPE_NAMES[type] ?? `TYPE${type}`

export type DnsQuestion = { name: string; type: number; end: number }

/** Parses the first question of a query. Names in questions are not compressed. */
export const parseQuestion = (packet: Buffer): DnsQuestion | null => {
  if (packet.length < 12 || packet.readUInt16BE(4) < 1) {
    return null
  }
  const labels: string[] = []
  let offset = 12
  while (offset < packet.length) {
    const length = packet[offset] ?? 0
    if (length === 0) {
      offset += 1
      break
    }
    if (length > 63) {
      return null
    }
    labels.push(packet.subarray(offset + 1, offset + 1 + length).toString("ascii"))
    offset += 1 + length
  }
  if (offset + 4 > packet.length) {
    return null
  }
  return {
    name: labels.join(".").toLowerCase(),
    type: packet.readUInt16BE(offset),
    end: offset + 4,
  }
}

const ipv4Bytes = (ip: string): Buffer =>
  Buffer.from(ip.split(".").map((part) => Number.parseInt(part, 10)))

const ipv6Bytes = (ip: string): Buffer => {
  const [head = "", tail = ""] = ip.split("::")
  const headParts = head ? head.split(":") : []
  const tailParts = tail ? tail.split(":") : []
  const missing = 8 - headParts.length - tailParts.length
  const parts = [...headParts, ...Array(Math.max(missing, 0)).fill("0"), ...tailParts]
  const buffer = Buffer.alloc(16)
  parts.forEach((part, index) => {
    buffer.writeUInt16BE(Number.parseInt(part || "0", 16), index * 2)
  })
  return buffer
}

/** Addresses the fake answers for a name and type (A/AAAA only). */
export const answersFor = (name: string, type: number): string[] => {
  const override = store.dnsOverride(name)
  if (type === TYPE_A) {
    return override ? override.a : [FAKE_UPSTREAM_IP]
  }
  if (type === TYPE_AAAA) {
    return override ? override.aaaa : []
  }
  return []
}

export const buildResponse = (query: Buffer, question: DnsQuestion): Buffer => {
  const addresses = answersFor(question.name, question.type)
  const header = Buffer.alloc(12)
  header.writeUInt16BE(query.readUInt16BE(0), 0) // id
  const requestFlags = query.readUInt16BE(2)
  const opcode = requestFlags & 0x7800
  const recursionDesired = requestFlags & 0x0100
  // QR=1, AA=1, RA=1, RCODE=0
  header.writeUInt16BE(0x8000 | opcode | 0x0400 | recursionDesired | 0x0080, 2)
  header.writeUInt16BE(1, 4) // qdcount
  header.writeUInt16BE(addresses.length, 6) // ancount
  header.writeUInt16BE(0, 8)
  header.writeUInt16BE(0, 10)
  const questionBytes = query.subarray(12, question.end)
  const answers = addresses.map((address) => {
    const rdata = question.type === TYPE_A ? ipv4Bytes(address) : ipv6Bytes(address)
    const record = Buffer.alloc(12)
    record.writeUInt16BE(0xc00c, 0) // pointer to the question name
    record.writeUInt16BE(question.type, 2)
    record.writeUInt16BE(1, 4) // class IN
    record.writeUInt32BE(30, 6) // TTL
    record.writeUInt16BE(rdata.length, 10)
    return Buffer.concat([record, rdata])
  })
  return Buffer.concat([header, questionBytes, ...answers])
}

const formErr = (query: Buffer): Buffer => {
  const header = Buffer.alloc(12)
  if (query.length >= 2) {
    header.writeUInt16BE(query.readUInt16BE(0), 0)
  }
  header.writeUInt16BE(0x8001, 2)
  return header
}

/** Answers one wire-format query and records it. */
export const answerQuery = (
  query: Buffer,
  transport: "udp" | "tcp" | "doh",
  remoteAddress: string | undefined,
): Buffer => {
  const question = parseQuestion(query)
  if (!question) {
    return formErr(query)
  }
  recordDnsQuery(question.name, question.type, transport, remoteAddress)
  return buildResponse(query, question)
}

export const recordDnsQuery = (
  name: string,
  type: number,
  transport: "udp" | "tcp" | "doh",
  remoteAddress: string | undefined,
): void => {
  store.addRecord({
    kind: "dns",
    scheme: "dns",
    host: name,
    method: dnsTypeName(type),
    path: "",
    rawQuery: "",
    query: {},
    headers: {},
    bodyText: "",
    bodyBase64: "",
    matchedScript: null,
    unmatched: false,
    dnsType: dnsTypeName(type),
    dnsTransport: transport,
    remoteAddress,
  })
}

export const startDnsServers = (port: number): void => {
  const udp = dgram.createSocket("udp4")
  udp.on("message", (message, remote) => {
    const response = answerQuery(message, "udp", remote.address)
    udp.send(response, remote.port, remote.address)
  })
  udp.bind(port, "0.0.0.0")

  const tcp = net.createServer((socket) => {
    let buffered = Buffer.alloc(0)
    socket.on("data", (chunk) => {
      buffered = Buffer.concat([buffered, chunk])
      while (buffered.length >= 2) {
        const length = buffered.readUInt16BE(0)
        if (buffered.length < 2 + length) {
          break
        }
        const message = buffered.subarray(2, 2 + length)
        buffered = buffered.subarray(2 + length)
        const response = answerQuery(message, "tcp", socket.remoteAddress)
        const prefix = Buffer.alloc(2)
        prefix.writeUInt16BE(response.length, 0)
        socket.write(Buffer.concat([prefix, response]))
      }
    })
    socket.on("error", () => socket.destroy())
  })
  tcp.listen(port, "0.0.0.0")
}

export { TYPE_A, TYPE_AAAA }
