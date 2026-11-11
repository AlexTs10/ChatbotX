// Realtime WebSocket client: connect, collect frames, wait for events.
// Server -> client only. Frames: {"batch":[{eventType,data,route?}],"seq":"<ms>-<n>"},
// heartbeat {"hb":1} (ignored here).

import WebSocket from "ws"
import { config } from "./config.ts"

export type RealtimeEvent = { eventType: string; data: unknown; route?: unknown }
export type RealtimeFrame = { batch: RealtimeEvent[]; seq: string }

export type RealtimeSocket = {
  socket: WebSocket
  frames: RealtimeFrame[]
  /** Resolves on open; rejects with the HTTP status when the upgrade is refused. */
  opened: Promise<void>
  closed: Promise<{ code: number; reason: string }>
  waitFor: (
    eventType: string,
    predicate?: (event: RealtimeEvent) => boolean,
    timeoutMs?: number,
  ) => Promise<RealtimeEvent>
  close: () => Promise<void>
}

export class UpgradeRefused extends Error {
  readonly status: number
  constructor(status: number) {
    super(`WebSocket upgrade refused with HTTP ${status}`)
    this.status = status
  }
}

export const workspaceSocketUrl = (workspaceId: string, token: string, lastSeq?: string) => {
  const url = new URL(`${config.rtUrl}/rt/workspaces/${workspaceId}`)
  url.searchParams.set("token", token)
  if (lastSeq) {
    url.searchParams.set("lastSeq", lastSeq)
  }
  return url.toString()
}

export const guestSocketUrl = (guestConversationId: string, token: string) => {
  const url = new URL(`${config.rtUrl}/rt/guests/${guestConversationId}`)
  url.searchParams.set("token", token)
  return url.toString()
}

export const connectRealtime = (url: string, headers: Record<string, string> = {}): RealtimeSocket => {
  const socket = new WebSocket(url, { headers })
  const frames: RealtimeFrame[] = []
  const listeners = new Set<() => void>()

  const opened = new Promise<void>((resolve, reject) => {
    socket.once("open", () => resolve())
    socket.once("unexpected-response", (_request, response) => {
      reject(new UpgradeRefused(response.statusCode ?? 0))
      response.resume()
    })
    socket.once("error", (error) => reject(error))
  })
  opened.catch(() => undefined)

  const closed = new Promise<{ code: number; reason: string }>((resolve) => {
    socket.once("close", (code, reason) => resolve({ code, reason: reason.toString("utf8") }))
  })

  socket.on("message", (data) => {
    let parsed: unknown
    try {
      parsed = JSON.parse(data.toString())
    } catch {
      return
    }
    if (parsed && typeof parsed === "object" && "batch" in parsed) {
      frames.push(parsed as RealtimeFrame)
      for (const listener of listeners) {
        listener()
      }
    }
  })

  const waitFor: RealtimeSocket["waitFor"] = (eventType, predicate = () => true, timeoutMs = 10_000) =>
    new Promise((resolve, reject) => {
      const find = () =>
        frames
          .flatMap((frame) => frame.batch)
          .find((event) => event.eventType === eventType && predicate(event))
      const existing = find()
      if (existing) {
        resolve(existing)
        return
      }
      const listener = () => {
        const event = find()
        if (event) {
          clearTimeout(timer)
          listeners.delete(listener)
          resolve(event)
        }
      }
      const timer = setTimeout(() => {
        listeners.delete(listener)
        reject(new Error(`realtime: no ${eventType} event within ${timeoutMs} ms`))
      }, timeoutMs)
      listeners.add(listener)
    })

  const close = async () => {
    if (socket.readyState === WebSocket.CLOSED) {
      return
    }
    socket.close(1000)
    await closed
  }

  return { socket, frames, opened, closed, waitFor, close }
}
