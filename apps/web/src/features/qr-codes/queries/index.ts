// apps/builder/src/features/qr-codes/queries for the SPA:
// `qrCodeService.list` in the Rust server (P4 spec §5.1).
import { serverQuery } from "@/lib/server-action"
import type { ListQrCodesRequest, ListQrCodesResponse } from "../schema/query"

export const listQrCodes = (input: ListQrCodesRequest) =>
  serverQuery<ListQrCodesResponse>("listQrCodes", input.workspaceId, [input])
