# Q.D8 — e-commerce rest (partial)

Branch `feat/conversion-d8` (after P3.6-D1o).

## Slices

| # | Slice | Rust | Original | State |
|---|-------|------|----------|-------|
| 1 | Discount codes: list, get, generateCode, create, update (partial, checked as a whole), setActive, delete, validate (priced as `orders.create`) | `business::discount_code`, `api::handlers::discount_codes` | `packages/business/src/discount-code/service.ts`, `features/discount-codes/api/public.ts` | done |
| 2 | E-commerce analytics: lifetime / period sales, orders, items, hourly or daily series, top products and vendors | `business::ecommerce_analytics`, `api::handlers::ecommerce_analytics` | `packages/business/src/ecommerce-analytics/service.ts` | done |
| 3 | `webshop.getLink` (signed customer link, preview link, `{{shop_token}}` button link) | `api::handlers::webshop` | `packages/business/src/webshop/link.ts` | done |
| 4 | SPA: orders (list, detail, status moves, cancel, delete, export), discount codes (list, form, on/off, delete, generate), settings sections, analytics | `api::rpc_procedures::ecommerce`, `apps/web/src/routes/ecommerce.tsx` + copied builder features | `app/space/[workspaceId]/(e-commerce)/*` | done |
| 5 | Shopify / WooCommerce sync (`shopifyWebhook` / `woocommerceWebhook` jobs, catalog and order mirror, connect / status / sync routes, SPA status and actions) | `business::{shopify,woocommerce}`, `worker::store_sync`, `api::handlers::store_connections` | `packages/business/src/{shopify,woocommerce}/*` | done |
| 6a | `carts.get`, discount `listAvailable`, coupons (`coupons.*`, 16 routes incl. CSV import / export) | `business::{cart,coupon,import::coupons}`, `api::handlers::{webshop,coupons}` | `webshop/service.ts` (`getCart`), `coupon/service.ts` | done |
| 6b | Public web shop pages and their actions, commerce enrichment | — | `app/(webshop)/*`, `webshop/service.ts` | not ported |
| 7 | Stripe Connect OAuth (`stripeAvailable`, `startStripeConnectAction`, `/integrations/stripe/callback`) | `business::payment_provider`, `api::connect_callback`, `api::rpc_procedures::ecommerce` | `payment-provider/service.ts`, `integrations/stripe/callback/route.ts` | done |

## Security

- Every `/v1` route is a workspace token with the `ecommerce` scope (route
  table); every `/rpc` page and action needs workspace membership and the
  `ecommerce` permission (`notFound` without it), as the layout.
- `discountCodes.validate` prices on the server from product ids and
  quantities (`order::quote`), never from client amounts.
- `webshop.getLink` for a contact refuses a read-only token (403), as the
  original; tokens are signed with `chatbotx_core::encryption`.
- Store webhooks: Shopify HMAC and WooCommerce signatures are compared in
  constant time (i34 receivers); store secrets are stored encrypted
  (`shopify:<ws>` / `woocommerce:<ws>` AAD); the WooCommerce site URL goes
  through the SSRF guard with pinned IPs.
- Stripe Connect: the state is a single-use connect session bound to the
  member and workspace; the callback checks the signed-in user, the
  super-admin role and the platform app; redirects stay relative.
- Coupons and carts: workspace-scoped; cart prices come from `order::quote`.

## Decisions

See `conversion/DECISIONS.md` (2026-11-17 and 2026-11-18 "Q.D8" lines).

## Tests

- Unit: `ecommerce_analytics` (percent change, bucket keys), `webshop`
  (shop URLs).
- The TS service tests (`discount-code-service`, `pricing`,
  `ecommerce-analytics-service`, `link`) are covered by the parity suites
  below.

## Parity

Not run in this unit: p1-07 `10-coupon-topics`, `11-coupons`,
`12-coupon-import-export`, `15-discount-codes`; p1-04 `08-carts`; p1-08
`02-ecommerce-settings`, `03-ecommerce-analytics`; p1-12
`20-payment-providers`, `30-orders`; the Shopify / WooCommerce receiver
suites.

## Not ported (left for a later unit)

- Slice 6b: the public web shop pages (5 Next pages, 6 actions, about 1700
  lines of `webshop/service.ts`) and commerce enrichment: too large for
  this unit.
- The coupon-template guards on message-template edit / delete.
