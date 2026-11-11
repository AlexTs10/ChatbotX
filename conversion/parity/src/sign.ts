// Signers for inbound webhooks (spec P1.1 section 4). Each function makes the
// header value a real provider would send for `rawBody`. Route tests (P1.20+)
// use them; the secrets come from fixtures or from recorded upstream calls.

import { createHash, createHmac, timingSafeEqual } from "node:crypto"

const hmac = (secret: string, data: string) => createHmac("sha256", secret).update(data)

/** Meta (Messenger, Instagram, Threads, WhatsApp): `x-hub-signature-256`. */
export const metaSignature = (appSecret: string, rawBody: string): string =>
  `sha256=${hmac(appSecret, rawBody).digest("hex")}`

/** TikTok: `TikTok-Signature: t=<unix>,s=<hex>` over `<t>.<body>`. */
export const tiktokSignature = (
  clientSecret: string,
  rawBody: string,
  unixSeconds = Math.floor(Date.now() / 1000),
): string => `t=${unixSeconds},s=${hmac(clientSecret, `${unixSeconds}.${rawBody}`).digest("hex")}`

/** Zalo: `X-ZEvent-Signature: mac=sha256(appId + body + timestamp + verifyToken)`. */
export const zaloSignature = (
  appId: string,
  rawBody: string,
  timestamp: string,
  verifyToken: string,
): string =>
  `mac=${createHash("sha256").update(`${appId}${rawBody}${timestamp}${verifyToken}`).digest("hex")}`

/** Shopify: `X-Shopify-Hmac-Sha256`, base64 HMAC. */
export const shopifySignature = (secret: string, rawBody: string): string =>
  hmac(secret, rawBody).digest("base64")

/** WooCommerce: `X-WC-Webhook-Signature`, base64 HMAC. */
export const wooCommerceSignature = (secret: string, rawBody: string): string =>
  hmac(secret, rawBody).digest("base64")

/** Stripe: `stripe-signature: t=<unix>,v1=<hex>` over `<t>.<body>`. */
export const stripeSignature = (
  webhookSecret: string,
  rawBody: string,
  unixSeconds = Math.floor(Date.now() / 1000),
): string => `t=${unixSeconds},v1=${hmac(webhookSecret, `${unixSeconds}.${rawBody}`).digest("hex")}`

/**
 * Checks an outbound API-channel callback: `X-ChatbotX-Signature: sha256=<hex>`
 * over `<X-ChatbotX-Timestamp>.<body>`.
 */
export const verifyChatbotxCallback = (
  signingSecret: string,
  headers: Record<string, string>,
  rawBody: string,
): boolean => {
  const timestamp = headers["x-chatbotx-timestamp"] ?? ""
  const signature = headers["x-chatbotx-signature"] ?? ""
  const expected = `sha256=${hmac(signingSecret, `${timestamp}.${rawBody}`).digest("hex")}`
  const left = Buffer.from(signature)
  const right = Buffer.from(expected)
  return left.length === right.length && timingSafeEqual(left, right)
}
