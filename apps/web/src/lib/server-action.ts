// The client side of a builder "use server" export (see
// apps/web/scripts/copy-from-builder.mjs): the code runs in the Rust server
// at `POST /rpc/actions/<name>`. Protocol: conversion/specs/P4-web.md §5.1.
// biome-ignore-all lint/suspicious/noExplicitAny: the action input/output types live on the server
import { ORPCError } from "@orpc/client"
import type { SafeActionResult as NextSafeActionResult } from "next-safe-action"
import type { ZodType } from "zod"
import { client } from "./orpc/orpc"

// The input schema lives on the server: a zod type whose input is `any`.
type SafeActionResult = NextSafeActionResult<
  string,
  ZodType<any, any>,
  any,
  any
>

const DEFAULT_SERVER_ERROR_MESSAGE =
  "Something went wrong while executing the operation."

function call(name: string, args: unknown[], workspace: boolean) {
  const input = workspace ? { args, workspaceId: args[0] } : { args }
  return client.actions[name](input) as Promise<unknown>
}

/**
 * A next-safe-action action: resolves to `{ data }` / `{ serverError }` /
 * `{ validationErrors }`. `noInput` (no `.inputSchema`) lets `execute()` go
 * without an argument, as the builder's types did.
 */
export function serverAction(
  name: string,
  options: { workspace?: boolean; noInput: true },
): (
  ...args: any[]
) => Promise<NextSafeActionResult<string, undefined, any, any>>
export function serverAction(
  name: string,
  options?: { workspace?: boolean },
): (...args: any[]) => Promise<SafeActionResult>
export function serverAction(
  name: string,
  options?: { workspace?: boolean; noInput?: true },
) {
  return async (...args: any[]): Promise<SafeActionResult> => {
    try {
      return { data: await call(name, args, options?.workspace ?? false) }
    } catch (error) {
      if (error instanceof ORPCError) {
        const validationErrors = (
          error.data as { validationErrors?: unknown } | undefined
        )?.validationErrors
        if (validationErrors) {
          return { validationErrors } as SafeActionResult
        }
        return { serverError: error.message }
      }
      return { serverError: DEFAULT_SERVER_ERROR_MESSAGE }
    }
  }
}

/** A plain "use server" function: the return value, or a rejection. */
export function serverFunction(name: string) {
  return (...args: any[]): Promise<any> => call(name, args, false)
}

/** A page loader (`GET /rpc/pages/<name>`, §5.2). */
export function loadPage<T>(
  name: string,
  input: Record<string, unknown> = {},
): Promise<T> {
  return client.pages[name](input) as Promise<T>
}

/**
 * A builder server-component query (`features/*\/queries`) that runs in the
 * Rust server as `POST /rpc/actions/<name>`; `workspaceId` makes the `/rpc`
 * membership check run (`assertCurrentUserCanAccessChatbot`).
 */
export function serverQuery<T>(
  name: string,
  workspaceId: string,
  args: unknown[],
): Promise<T> {
  return client.actions[name]({ args, workspaceId }) as Promise<T>
}
