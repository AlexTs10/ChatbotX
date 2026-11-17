# Q.D8 — e-commerce rest (partial)

Branch `feat/conversion-d8` (after P3.6-D1o).

## Slices

| # | Slice | Rust | Original | State |
|---|-------|------|----------|-------|
| 1 | Discount codes: list, get, generateCode, create, update (partial, checked as a whole), setActive, delete, validate (priced as `orders.create`) | `business::discount_code`, `api::handlers::discount_codes` | `packages/business/src/discount-code/service.ts`, `features/discount-codes/api/public.ts` | done |
| 2 | E-commerce analytics: lifetime / period sales, orders, items, hourly or daily series, top products and vendors | `business::ecommerce_analytics`, `api::handlers::ecommerce_analytics` | `packages/business/src/ecommerce-analytics/service.ts` | done |
| 3 | `webshop.getLink` (signed customer link, preview link, `{{shop_token}}` button link) | `api::handlers::webshop` | `packages/business/src/webshop/link.ts` | done |
| 4 | SPA: orders (list, detail, status moves, cancel, delete, export), discount codes (list, form, on/off, delete, generate), settings sections, analytics | `api::rpc_procedures::ecommerce`, `apps/web/src/routes/ecommerce.tsx` + copied builder features | `app/space/[workspaceId]/(e-commerce)/*` | done |
| 5 | Shopify / WooCommerce sync (`shopifyWebhook` / `woocommerceWebhook` jobs, catalog and order mirror, connect / status / sync routes) | — | `packages/business/src/{shopify,woocommerce}/*` | not ported |
| 6 | Public web shop pages, carts (`carts.get`), commerce enrichment, coupons (`coupons.*`, 16 routes) | — | `webshop/service.ts`, `coupon/service.ts` | not ported |

## Security

- Every `/v1` route is a workspace token with the `ecommerce` scope (route
  table); every `/rpc` page and action needs workspace membership and the
  `ecommerce` permission (`notFound` without it), as the layout.
- `discountCodes.validate` prices on the server from product ids and
  quantities (`order::quote`), never from client amounts.
- `webshop.getLink` for a contact refuses a read-only token (403), as the
  original; tokens are signed with `chatbotx_core::encryption`.
- No outbound fetch is added.

## Decisions

See `conversion/DECISIONS.md` (2026-11-17 "Q.D8" lines).

## Tests

- Unit: `ecommerce_analytics` (percent change, bucket keys), `webshop`
  (shop URLs).
- The TS service tests (`discount-code-service`, `pricing`,
  `ecommerce-analytics-service`, `link`) are covered by the parity suites
  below.

## Parity

p1-07 `15-discount-codes`, p1-08 `03-ecommerce-analytics` (not run in this
unit); p1-12 `30-orders` for the SPA order actions' services.

## Not ported (left for a later unit)

- Shopify and WooCommerce (slice 5): the i34 receivers queue jobs the
  worker still fails as unrecoverable. The SPA settings show both as not
  connected; their connect actions are not served.
- Slice 6: the public shop pages and carts, `carts.get`, commerce
  enrichment and the coupon pools.
