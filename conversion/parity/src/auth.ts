// Email + password auth over HTTP (better-auth routes under /api/auth).

import { config } from "./config.ts"
import type { HttpClient, HttpResponse } from "./http.ts"
import { extractLinks, waitForMail } from "./mailhog.ts"

export const SESSION_COOKIE = "better-auth.session_token"

export type AuthUser = {
  id: string
  email: string
  name: string | null
  emailVerified: boolean
}

export const signUp = (
  http: HttpClient,
  input: { email: string; password: string; name: string },
): Promise<HttpResponse<{ token: string | null; user: AuthUser }>> =>
  http.post("/api/auth/sign-up/email", input)

export const signIn = (
  http: HttpClient,
  input: { email: string; password: string },
): Promise<HttpResponse<{ redirect: boolean; token: string; user: AuthUser }>> =>
  http.post("/api/auth/sign-in/email", input)

export const getSession = (
  http: HttpClient,
): Promise<HttpResponse<{ session: Record<string, unknown>; user: AuthUser } | null>> =>
  http.get("/api/auth/get-session")

/** The verification link of the newest sign-up mail to `email`. */
export const verificationLinkFor = async (email: string): Promise<string> => {
  const messages = await waitForMail(email)
  const prefix = `${config.baseUrl}/api/auth/verify-email?`
  for (const message of messages.reverse()) {
    const [link] = extractLinks(message, prefix)
    if (link) {
      return link
    }
  }
  throw new Error(`no verification link for ${email}`)
}

/** Signs in and fails with the response text if the stack refuses. */
export const signInOrThrow = async (
  http: HttpClient,
  input: { email: string; password: string },
): Promise<AuthUser> => {
  const response = await signIn(http, input)
  if (response.status !== 200 || !http.cookie(SESSION_COOKIE)) {
    throw new Error(`sign-in failed: ${response.status} ${response.text}`)
  }
  return response.json.user
}
