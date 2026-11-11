import { describe, expect, it } from "vitest"
import {
  getSession,
  SESSION_COOKIE,
  signIn,
  signUp,
  verificationLinkFor,
} from "../../src/auth.ts"
import { config } from "../../src/config.ts"
import { HttpClient } from "../../src/http.ts"
import { comparableHeaders, Normalizer } from "../../src/normalize.ts"
import { uniqueEmail } from "../../src/unique.ts"

describe("smoke: sign-up, email verification, sign-in", () => {
  it("signs up, verifies through the mailed link, signs in and reads the session", async () => {
    const normalizer = new Normalizer()
    const http = new HttpClient()
    const email = uniqueEmail("signup")
    const password = "Parity#12345"

    const signUpResponse = await signUp(http, { email, password, name: "Parity Sign Up" })
    expect(signUpResponse.status).toBe(200)
    expect(normalizer.normalize(signUpResponse.json)).toMatchObject({
      token: null,
      user: { email, emailVerified: false, name: "Parity Sign Up", id: "<id:1>" },
    })
    expect(http.cookie(SESSION_COOKIE)).toBeUndefined()

    // Email verification is required before sign-in.
    const early = await signIn(http, { email, password })
    expect(early.status).toBe(403)
    expect((early.json as { code?: string }).code).toBe("EMAIL_NOT_VERIFIED")
    expect(http.cookie(SESSION_COOKIE)).toBeUndefined()

    const link = await verificationLinkFor(email)
    const linkUrl = new URL(link)
    expect(`${linkUrl.origin}${linkUrl.pathname}`).toBe(`${config.baseUrl}/api/auth/verify-email`)
    expect(linkUrl.searchParams.get("token")).toBeTruthy()

    const verify = await http.get(link)
    expect([200, 302]).toContain(verify.status)
    if (verify.status === 302) {
      expect(comparableHeaders(verify.headers).location).toBeDefined()
    }

    const signInResponse = await signIn(http, { email, password })
    expect(signInResponse.status).toBe(200)
    expect(http.cookie(SESSION_COOKIE)).toBeTruthy()
    expect(normalizer.normalize(signInResponse.json)).toMatchObject({
      redirect: false,
      user: { id: "<id:1>", email, emailVerified: true },
    })

    const session = await getSession(http)
    expect(session.status).toBe(200)
    expect(normalizer.normalize(session.json)).toMatchObject({
      user: { id: "<id:1>", email, emailVerified: true, name: "Parity Sign Up" },
      session: { userId: "<id:1>", expiresAt: "<ts>" },
    })
  })

  it("refuses a wrong password", async () => {
    const http = new HttpClient()
    const response = await signIn(http, { email: uniqueEmail("nobody"), password: "Wrong#12345" })
    expect(response.status).toBe(401)
    expect((response.json as { code?: string }).code).toBe("INVALID_EMAIL_OR_PASSWORD")
    expect(http.cookie(SESSION_COOKIE)).toBeUndefined()
  })
})
