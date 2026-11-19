import {
  type ConnectErrorQueryCode,
  connectErrorQueryCodes,
} from "@chatbotx.io/utils/connection"

// The session lookup (`loadSettingsSession`) runs in Rust
// (`googleAdsSettingsPage`); only the client-side param parsing stays here.
export type SettingsSession = { id: string; status: string } | null

/**
 * The `?connect_error=` value when it is one of the known codes, else `null`.
 * The parameter is attacker-controllable (anyone can craft the link), so an
 * unknown or repeated value is dropped rather than shown or echoed.
 */
export const parseConnectErrorParam = (
  raw: string | string[] | undefined,
): ConnectErrorQueryCode | null => {
  const parsed = connectErrorQueryCodes.safeParse(
    Array.isArray(raw) ? raw[0] : raw,
  )
  return parsed.success ? parsed.data : null
}
