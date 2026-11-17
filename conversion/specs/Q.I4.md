# Q.I4 — Google, OpenAI, payments and store webhooks

Branch `feat/conversion-i34`.

## Ported

| # | Slice | Rust | Original |
|---|-------|------|----------|
| 1 | `POST /api/shopify/{workspaceId}/webhook`, `POST /api/woocommerce/{workspaceId}/webhook`: HMAC-SHA256 (base64) with the store secret, constant-time compare; shop domain / site check; job `shopifyWebhook` / `woocommerceWebhook` on the integration queue, job id = delivery (`shopify-webhook-<id>`, `woocommerce-webhook-<ws>-<sha256>`) | `api::store_webhooks` | `app/api/{shopify,woocommerce}/[workspaceId]/webhook/route.ts`, `packages/business/src/{shopify,woocommerce}/webhooks.ts` |

The store secret is decrypted with the original AAD (`shopify:<ws>`,
`woocommerce:<ws>`). A bad signature is 401, no store 404, a failure 500
(the store delivers again).

## Not in this unit (owner)

- The job side `processShopifyWebhook` / `processWooCommerceWebhook` (re-read
  and write the object): Q.D8 store sync. Until then the worker fails these
  jobs with a clear message; they stay in the failed set for a retry.
- Payment webhooks (Stripe, PayPal): Q.D8. The commerce agent
  (`feat/conversion-d8`) has the providers and their webhook checks in
  progress (`business::payments`).
- Google Ads: Q.D7 (TRACKER lists it there).
- Spreadsheets (flow step `runGoogleSheet`, `/v1/spreadsheets`, rows): Q.D13.
- OpenAI: nothing left; connect / disconnect / update and `aiIntegrationsAPI`
  were ported in Q.D2c / Q.D6.
- Inbound hooks `/api/hooks/{token}`: Q.D13 (inbound webhooks).

## Left (this unit)

- Google Sheets OAuth connect (`googleSheets` adapter for the connect-session
  engine: authorize URL, code exchange, `tokeninfo` identity, the
  `IntegrationGoogleSheet` + `Integration` rows), disconnect (token revoke)
  and the Sheets client (`listSheetNames`, `listSheetHeaders`,
  `getSheetValues`, `insertRow`, `updateRow`, `clearRow`) with token refresh.
  The legacy unsigned JSON state is not ported: the connect action must
  start a signed connect session (`business::connect_session`).

## Parity

No suite yet (P1.23 is `todo`). `conversion/parity/src/sign.ts` already has
the Shopify / WooCommerce signers for it.
