-- crates/db/migrations/0004_rest.sql  (P2.5: schema part 3, 126 tables + 4 rollup tables)
-- Generated once from packages/database/drizzle/20261110161124_resource_deletion_log/snapshot.json
-- by the P2.2 rules (as applied in 0001_core.sql / 0002_messaging.sql), then reviewed by hand. Do not edit after release:
-- the runner stores a checksum. Version 4 (0003_jobs.sql is the P2.6 jobs schema).
-- Also here: "ContactOnSmartDelay"."appointmentId" (part-2 column with a FK to "Appointment", P2.4 spec 1.5),
-- the SQL-only covering index of "AnalyticsContactEvent", the 4 rollup tables that replace the TimescaleDB continuous aggregates,
-- and their 12 triggers.
-- Needs the app SQL function lower_u (P2.3 3.1) for 4 unique indexes (Vendor, DiscountCode, CouponTopic, PlaceCategory).
-- Money numeric(19,4) -> INTEGER x 10,000; unbounded numeric -> TEXT decimal; vector(1536) -> BLOB of 6144 bytes (STRICT kept).

-- ======================== Ecommerce and payments ========================

-- Product  (product)
-- "price", "taxes", "discount" are double precision in Postgres (not money) -> REAL (P2.2 1.4: keep, JSON numbers).
-- "tags" jsonb array queried with ?| -> EXISTS (SELECT 1 FROM json_each("tags") WHERE value IN (SELECT value FROM json_each(?))).
CREATE TABLE "Product" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "name" TEXT NOT NULL,
  "shortDescription" TEXT,
  "longDescription" TEXT,
  "price" REAL NOT NULL DEFAULT 0,
  "taxes" REAL NOT NULL DEFAULT 0,
  "discount" REAL NOT NULL DEFAULT 0,
  "currency" TEXT NOT NULL DEFAULT 'USD',
  "productUrl" TEXT,
  "sku" TEXT,
  "inventoryPolicy" TEXT NOT NULL DEFAULT 'dont_track' CHECK ("inventoryPolicy" IN ('dont_track', 'track')),
  "inventoryQuantity" INTEGER NOT NULL DEFAULT 0,
  "allowOutOfStockPurchase" INTEGER NOT NULL DEFAULT 0 CHECK ("allowOutOfStockPurchase" IN (0, 1)),
  "images" TEXT NOT NULL DEFAULT '[]' CHECK (json_valid("images")),
  "tags" TEXT NOT NULL DEFAULT '[]' CHECK (json_valid("tags")),
  "vendorId" INTEGER REFERENCES "Vendor" ("id") ON DELETE SET NULL ON UPDATE CASCADE,
  "rank" INTEGER NOT NULL DEFAULT 10,
  "categoryId" INTEGER REFERENCES "ProductCategory" ("id") ON DELETE SET NULL ON UPDATE CASCADE,
  "subcategoryId" INTEGER REFERENCES "ProductCategory" ("id") ON DELETE SET NULL ON UPDATE CASCADE,
  "status" TEXT NOT NULL DEFAULT 'active' CHECK ("status" IN ('active', 'draft', 'unlisted')),
  "isSearchable" INTEGER NOT NULL DEFAULT 1 CHECK ("isSearchable" IN (0, 1)),
  "allowSpecialRequest" INTEGER NOT NULL DEFAULT 0 CHECK ("allowSpecialRequest" IN (0, 1)),
  "isAddonOnly" INTEGER NOT NULL DEFAULT 0 CHECK ("isAddonOnly" IN (0, 1)),
  "source" TEXT NOT NULL DEFAULT 'chatbotx' CHECK ("source" IN ('chatbotx', 'shopify', 'woocommerce')),
  "externalId" TEXT,
  "externalHandle" TEXT,
  "externalVariantId" TEXT,
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE
) STRICT;
CREATE INDEX "Product_workspaceId_idx" ON "Product" ("workspaceId");
CREATE INDEX "Product_categoryId_idx" ON "Product" ("categoryId");
CREATE INDEX "Product_subcategoryId_idx" ON "Product" ("subcategoryId");
CREATE INDEX "Product_vendorId_idx" ON "Product" ("vendorId");
CREATE UNIQUE INDEX "Product_workspaceId_source_externalId_key" ON "Product" ("workspaceId", "source", "externalId") WHERE "externalId" IS NOT NULL;

-- ProductCategory  (product-category)
-- Postgres UNIQUE NULLS NOT DISTINCT ("workspaceId", "source", "parentId", "name") -> unique index on ifnull("parentId", 0)
-- (snowflake ids are > 0). Upsert target: ON CONFLICT ("workspaceId", "source", ifnull("parentId", 0), "name") (checked).
CREATE TABLE "ProductCategory" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "parentId" INTEGER REFERENCES "ProductCategory" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "name" TEXT NOT NULL,
  "imageUrl" TEXT,
  "isVisible" INTEGER NOT NULL DEFAULT 1 CHECK ("isVisible" IN (0, 1)),
  "rank" INTEGER NOT NULL DEFAULT 10,
  "source" TEXT NOT NULL DEFAULT 'chatbotx' CHECK ("source" IN ('chatbotx', 'shopify', 'woocommerce'))
) STRICT;
CREATE INDEX "ProductCategory_workspaceId_idx" ON "ProductCategory" ("workspaceId");
CREATE INDEX "ProductCategory_parentId_idx" ON "ProductCategory" ("parentId");
CREATE UNIQUE INDEX "ProductCategory_workspaceId_parent_name_key" ON "ProductCategory" ("workspaceId", "source", ifnull("parentId", 0), "name");

-- ProductVariant  (product-variant)
CREATE TABLE "ProductVariant" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "productId" INTEGER NOT NULL REFERENCES "Product" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "combination" TEXT NOT NULL CHECK (json_valid("combination")),
  "price" REAL NOT NULL DEFAULT 0,
  "isEnabled" INTEGER NOT NULL DEFAULT 1 CHECK ("isEnabled" IN (0, 1)),
  "sku" TEXT,
  "inventoryQuantity" INTEGER,
  "externalId" TEXT,
  "externalInventoryItemId" TEXT
) STRICT;
CREATE INDEX "ProductVariant_productId_idx" ON "ProductVariant" ("productId");
CREATE INDEX "ProductVariant_externalInventoryItemId_idx" ON "ProductVariant" ("externalInventoryItemId") WHERE "externalInventoryItemId" IS NOT NULL;

-- ProductVariantOption  (product-variant-option)
CREATE TABLE "ProductVariantOption" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "productId" INTEGER NOT NULL REFERENCES "Product" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "name" TEXT NOT NULL,
  "values" TEXT NOT NULL DEFAULT '[]' CHECK (json_valid("values")),
  "position" INTEGER NOT NULL DEFAULT 10
) STRICT;
CREATE INDEX "ProductVariantOption_productId_idx" ON "ProductVariantOption" ("productId");

-- ProductAddon  (product-addon)
CREATE TABLE "ProductAddon" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "productId" INTEGER NOT NULL REFERENCES "Product" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "name" TEXT NOT NULL DEFAULT '',
  "maxSelections" INTEGER NOT NULL DEFAULT 1,
  "addonProductIds" TEXT NOT NULL DEFAULT '[]' CHECK (json_valid("addonProductIds"))
) STRICT;
CREATE INDEX "ProductAddon_productId_idx" ON "ProductAddon" ("productId");

-- Vendor  (vendor)
-- money numeric(19,4) -> INTEGER x 10,000; the CHECK bound 100 -> 1,000,000.
-- Unique on lower("name") -> lower_u("name") (app function, P2.3 3.1; DETERMINISTIC | INNOCUOUS). Queries must write lower_u("name") = lower_u(?).
CREATE TABLE "Vendor" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "name" TEXT NOT NULL,
  "shortDescription" TEXT,
  "phone" TEXT,
  "storeAddress" TEXT,
  "imageUrl" TEXT,
  "isActive" INTEGER NOT NULL DEFAULT 1 CHECK ("isActive" IN (0, 1)),
  "discountPercent" INTEGER NOT NULL DEFAULT 0,
  "minimumOrderValue" INTEGER NOT NULL DEFAULT 0,
  "openHoursId" INTEGER REFERENCES "EcommerceOpenHours" ("id") ON DELETE SET NULL ON UPDATE CASCADE,
  "categoryId" INTEGER REFERENCES "ProductCategory" ("id") ON DELETE SET NULL ON UPDATE CASCADE,
  "subcategoryId" INTEGER REFERENCES "ProductCategory" ("id") ON DELETE SET NULL ON UPDATE CASCADE,
  "rank" INTEGER NOT NULL DEFAULT 10,
  "source" TEXT NOT NULL DEFAULT 'chatbotx' CHECK ("source" IN ('chatbotx', 'shopify', 'woocommerce')),
  CONSTRAINT "Vendor_discountPercent_check" CHECK ("discountPercent" >= 0 AND "discountPercent" <= 1000000)
) STRICT;
CREATE INDEX "Vendor_workspaceId_idx" ON "Vendor" ("workspaceId");
CREATE INDEX "Vendor_openHoursId_idx" ON "Vendor" ("openHoursId");
CREATE INDEX "Vendor_categoryId_idx" ON "Vendor" ("categoryId");
CREATE INDEX "Vendor_subcategoryId_idx" ON "Vendor" ("subcategoryId");
CREATE UNIQUE INDEX "Vendor_workspaceId_name_key" ON "Vendor" ("workspaceId", "source", lower_u("name"));

-- Cart  (cart)
-- Partial index: jsonb_array_length -> json_array_length (same expression text in the scan query).
CREATE TABLE "Cart" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "contactId" INTEGER NOT NULL REFERENCES "Contact" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "contactInboxId" INTEGER REFERENCES "ContactInbox" ("id") ON DELETE SET NULL ON UPDATE CASCADE,
  "items" TEXT NOT NULL CHECK (json_valid("items")),
  "discountCode" TEXT,
  "lastActivityAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "abandonedNotifiedAt" INTEGER
) STRICT;
CREATE UNIQUE INDEX "Cart_workspaceId_contactId_key" ON "Cart" ("workspaceId", "contactId");
CREATE INDEX "Cart_contactId_idx" ON "Cart" ("contactId");
CREATE INDEX "Cart_contactInboxId_idx" ON "Cart" ("contactInboxId");
CREATE INDEX "Cart_abandoned_due_idx" ON "Cart" ("lastActivityAt") WHERE "abandonedNotifiedAt" IS NULL AND json_array_length("items") > 0;

-- Order  (order)
-- 7 money columns -> INTEGER x 10,000. "number" comes from "EcommerceOrderCounter" (upsert ... RETURNING).
CREATE TABLE "Order" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "number" INTEGER NOT NULL,
  "contactId" INTEGER REFERENCES "Contact" ("id") ON DELETE SET NULL ON UPDATE CASCADE,
  "contactInboxId" INTEGER REFERENCES "ContactInbox" ("id") ON DELETE SET NULL ON UPDATE CASCADE,
  "inboxId" INTEGER REFERENCES "Inbox" ("id") ON DELETE SET NULL ON UPDATE CASCADE,
  "conversationId" INTEGER REFERENCES "Conversation" ("id") ON DELETE SET NULL ON UPDATE CASCADE,
  "channel" TEXT,
  "source" TEXT NOT NULL CHECK ("source" IN ('webshop', 'api', 'manual', 'shopify', 'woocommerce')),
  "status" TEXT NOT NULL DEFAULT 'pending' CHECK ("status" IN ('pending', 'accepted', 'shipped', 'concluded', 'cancelled')),
  "paymentMethod" TEXT NOT NULL DEFAULT 'cod' CHECK ("paymentMethod" IN ('cod', 'stripe', 'paypal')),
  "paymentStatus" TEXT NOT NULL DEFAULT 'unpaid' CHECK ("paymentStatus" IN ('unpaid', 'paid', 'refunded')),
  "paymentExpiresAt" INTEGER,
  "paidAt" INTEGER,
  "currency" TEXT NOT NULL,
  "subtotal" INTEGER NOT NULL,
  "discountTotal" INTEGER NOT NULL,
  "shippingTotal" INTEGER NOT NULL,
  "serviceFeeTotal" INTEGER NOT NULL,
  "taxTotal" INTEGER NOT NULL,
  "couponDiscount" INTEGER NOT NULL DEFAULT 0,
  "total" INTEGER NOT NULL,
  "discountCodeId" INTEGER REFERENCES "DiscountCode" ("id") ON DELETE SET NULL ON UPDATE CASCADE,
  "discountCode" TEXT,
  "customerName" TEXT,
  "customerEmail" TEXT,
  "customerPhone" TEXT,
  "shippingType" TEXT CHECK ("shippingType" IN ('delivery', 'pickup')),
  "shippingAddress" TEXT CHECK ("shippingAddress" IS NULL OR json_valid("shippingAddress")),
  "note" TEXT,
  "rating" INTEGER,
  "ratingComment" TEXT,
  "acceptedAt" INTEGER,
  "shippedAt" INTEGER,
  "concludedAt" INTEGER,
  "cancelledAt" INTEGER,
  "cancelReason" TEXT,
  "externalId" TEXT,
  "externalName" TEXT,
  "fulfillmentStatus" TEXT CHECK ("fulfillmentStatus" IN ('unfulfilled', 'partial', 'fulfilled')),
  "trackingCompany" TEXT,
  "trackingNumber" TEXT,
  "trackingUrl" TEXT
) STRICT;
CREATE UNIQUE INDEX "Order_workspaceId_externalId_key" ON "Order" ("workspaceId", "externalId") WHERE "externalId" IS NOT NULL;
CREATE INDEX "Order_workspaceId_source_createdAt_idx" ON "Order" ("workspaceId", "source", "createdAt" DESC);
CREATE UNIQUE INDEX "Order_workspaceId_number_key" ON "Order" ("workspaceId", "number");
CREATE INDEX "Order_workspaceId_createdAt_idx" ON "Order" ("workspaceId", "createdAt" DESC);
CREATE INDEX "Order_workspaceId_status_idx" ON "Order" ("workspaceId", "status");
CREATE INDEX "Order_contactId_idx" ON "Order" ("contactId");
CREATE INDEX "Order_contactInboxId_idx" ON "Order" ("contactInboxId");
CREATE INDEX "Order_inboxId_idx" ON "Order" ("inboxId");
CREATE INDEX "Order_conversationId_idx" ON "Order" ("conversationId");
CREATE INDEX "Order_discountCodeId_idx" ON "Order" ("discountCodeId");
CREATE INDEX "Order_paymentExpiresAt_idx" ON "Order" ("paymentExpiresAt") WHERE "status" = 'pending' AND "paymentStatus" = 'unpaid' AND "paymentExpiresAt" IS NOT NULL;

-- OrderItem  (order)
-- 6 money columns -> INTEGER x 10,000.
CREATE TABLE "OrderItem" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "orderId" INTEGER NOT NULL REFERENCES "Order" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "productId" INTEGER REFERENCES "Product" ("id") ON DELETE SET NULL ON UPDATE CASCADE,
  "variantId" INTEGER REFERENCES "ProductVariant" ("id") ON DELETE SET NULL ON UPDATE CASCADE,
  "vendorId" INTEGER REFERENCES "Vendor" ("id") ON DELETE SET NULL ON UPDATE CASCADE,
  "categoryId" INTEGER REFERENCES "ProductCategory" ("id") ON DELETE SET NULL ON UPDATE CASCADE,
  "name" TEXT NOT NULL,
  "variantLabel" TEXT,
  "combination" TEXT CHECK ("combination" IS NULL OR json_valid("combination")),
  "sku" TEXT,
  "retailerId" TEXT,
  "imageUrl" TEXT,
  "vendorName" TEXT,
  "unitPrice" INTEGER NOT NULL,
  "quantity" INTEGER NOT NULL,
  "stockTaken" INTEGER NOT NULL DEFAULT 0,
  "discountPercent" INTEGER NOT NULL,
  "taxPercent" INTEGER NOT NULL,
  "discountTotal" INTEGER NOT NULL,
  "taxTotal" INTEGER NOT NULL,
  "lineTotal" INTEGER NOT NULL
) STRICT;
CREATE INDEX "OrderItem_orderId_idx" ON "OrderItem" ("orderId");
CREATE INDEX "OrderItem_workspaceId_productId_idx" ON "OrderItem" ("workspaceId", "productId");
CREATE INDEX "OrderItem_workspaceId_vendorId_idx" ON "OrderItem" ("workspaceId", "vendorId");
CREATE INDEX "OrderItem_variantId_idx" ON "OrderItem" ("variantId");
CREATE INDEX "OrderItem_categoryId_idx" ON "OrderItem" ("categoryId");

-- OrderStatusHistory  (order)
CREATE TABLE "OrderStatusHistory" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "orderId" INTEGER NOT NULL REFERENCES "Order" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "fromStatus" TEXT CHECK ("fromStatus" IN ('pending', 'accepted', 'shipped', 'concluded', 'cancelled')),
  "toStatus" TEXT NOT NULL CHECK ("toStatus" IN ('pending', 'accepted', 'shipped', 'concluded', 'cancelled')),
  "actorType" TEXT NOT NULL CHECK ("actorType" IN ('user', 'apiToken', 'system', 'contact')),
  "actorUserId" INTEGER REFERENCES "User" ("id") ON DELETE SET NULL ON UPDATE CASCADE,
  "note" TEXT
) STRICT;
CREATE INDEX "OrderStatusHistory_orderId_createdAt_idx" ON "OrderStatusHistory" ("orderId", "createdAt");
CREATE INDEX "OrderStatusHistory_workspaceId_idx" ON "OrderStatusHistory" ("workspaceId");
CREATE INDEX "OrderStatusHistory_actorUserId_idx" ON "OrderStatusHistory" ("actorUserId");

-- OrderPayment  (ecommerce-payment)
CREATE TABLE "OrderPayment" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "orderId" INTEGER NOT NULL REFERENCES "Order" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "provider" TEXT NOT NULL CHECK ("provider" IN ('stripe', 'paypal')),
  "providerReference" TEXT NOT NULL,
  "transactionId" TEXT,
  "refundId" TEXT,
  "amount" INTEGER NOT NULL,
  "currency" TEXT NOT NULL,
  "status" TEXT NOT NULL DEFAULT 'open' CHECK ("status" IN ('open', 'paid', 'expired', 'refunded')),
  "livemode" INTEGER NOT NULL CHECK ("livemode" IN (0, 1)),
  "checkoutUrl" TEXT NOT NULL,
  "expiresAt" INTEGER NOT NULL,
  "paidAt" INTEGER,
  "refundedAt" INTEGER
) STRICT;
CREATE UNIQUE INDEX "OrderPayment_provider_providerReference_key" ON "OrderPayment" ("provider", "providerReference");
CREATE INDEX "OrderPayment_orderId_idx" ON "OrderPayment" ("orderId");
CREATE INDEX "OrderPayment_workspaceId_idx" ON "OrderPayment" ("workspaceId");
CREATE INDEX "OrderPayment_open_expiresAt_idx" ON "OrderPayment" ("expiresAt") WHERE "status" = 'open';

-- EcommercePaymentProvider  (ecommerce-payment)
CREATE TABLE "EcommercePaymentProvider" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "provider" TEXT NOT NULL CHECK ("provider" IN ('stripe', 'paypal')),
  "livemode" INTEGER NOT NULL CHECK ("livemode" IN (0, 1)),
  "accountLabel" TEXT,
  "externalAccountId" TEXT,
  "platformCredentialId" INTEGER REFERENCES "PlatformCredential" ("id") ON DELETE SET NULL ON UPDATE CASCADE,
  "secret" TEXT CHECK ("secret" IS NULL OR json_valid("secret")),
  "webhookEndpointId" TEXT
) STRICT;
CREATE UNIQUE INDEX "EcommercePaymentProvider_workspaceId_provider_key" ON "EcommercePaymentProvider" ("workspaceId", "provider");
CREATE UNIQUE INDEX "EcommercePaymentProvider_platformCredentialId_account_key" ON "EcommercePaymentProvider" ("platformCredentialId", "externalAccountId") WHERE "externalAccountId" IS NOT NULL;
CREATE INDEX "EcommercePaymentProvider_externalAccountId_idx" ON "EcommercePaymentProvider" ("externalAccountId");

-- EcommerceSettings  (ecommerce-settings)
-- 3 money columns -> INTEGER x 10,000; CHECK bound 100 -> 1,000,000.
CREATE TABLE "EcommerceSettings" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "storeSource" TEXT NOT NULL DEFAULT 'chatbotx' CHECK ("storeSource" IN ('chatbotx', 'shopify', 'woocommerce')),
  "orderingAvailable" INTEGER NOT NULL CHECK ("orderingAvailable" IN (0, 1)),
  "webshopProductView" TEXT NOT NULL CHECK ("webshopProductView" IN ('list', 'listWithoutImage', 'card', 'cardSquare', 'image', 'imageSquare')),
  "openHoursId" INTEGER REFERENCES "EcommerceOpenHours" ("id") ON DELETE SET NULL ON UPDATE CASCADE,
  "searchProducts" INTEGER NOT NULL CHECK ("searchProducts" IN (0, 1)),
  "facebookPixelId" TEXT,
  "abandonedCartDelayMinutes" INTEGER,
  "abandonedCartFlowId" INTEGER REFERENCES "Flow" ("id") ON DELETE SET NULL ON UPDATE CASCADE,
  "productButtons" TEXT NOT NULL CHECK (json_valid("productButtons")),
  "productImageSize" TEXT NOT NULL CHECK ("productImageSize" IN ('bigSquare', 'mediumRectangle')),
  "allowRateOrders" INTEGER NOT NULL CHECK ("allowRateOrders" IN (0, 1)),
  "notifyOnCartStatusChange" INTEGER NOT NULL CHECK ("notifyOnCartStatusChange" IN (0, 1)),
  "paymentMethods" TEXT NOT NULL CHECK (json_valid("paymentMethods")),
  "currency" TEXT NOT NULL,
  "currencyPosition" TEXT NOT NULL CHECK ("currencyPosition" IN ('left', 'leftSpace', 'right', 'rightSpace')),
  "currencySymbol" TEXT,
  "serviceFeePercent" INTEGER NOT NULL,
  "shippingType" TEXT NOT NULL CHECK ("shippingType" IN ('none', 'delivery', 'pickup', 'deliveryAndPickup')),
  "storeAddress" TEXT,
  "shippingCountries" TEXT NOT NULL CHECK (json_valid("shippingCountries")),
  "defaultShippingCountry" TEXT,
  "deliveryCost" INTEGER NOT NULL,
  "checkoutMode" TEXT NOT NULL CHECK ("checkoutMode" IN ('default', 'externalLink', 'flow', 'productPage')),
  "checkoutLink" TEXT,
  "checkoutFlowId" INTEGER REFERENCES "Flow" ("id") ON DELETE SET NULL ON UPDATE CASCADE,
  "minimumOrderValue" INTEGER NOT NULL,
  "orderConfirmationMode" TEXT NOT NULL DEFAULT 'default' CHECK ("orderConfirmationMode" IN ('default', 'flow', 'none')),
  "orderConfirmationFlowId" INTEGER REFERENCES "Flow" ("id") ON DELETE SET NULL ON UPDATE CASCADE,
  "requireName" INTEGER NOT NULL CHECK ("requireName" IN (0, 1)),
  "requireEmail" INTEGER NOT NULL CHECK ("requireEmail" IN (0, 1)),
  "requirePhone" INTEGER NOT NULL CHECK ("requirePhone" IN (0, 1)),
  "requireState" INTEGER NOT NULL CHECK ("requireState" IN (0, 1)),
  "requireZip" INTEGER NOT NULL CHECK ("requireZip" IN (0, 1)),
  "externalCheckoutMethod" TEXT NOT NULL DEFAULT 'webCheckout' CHECK ("externalCheckoutMethod" IN ('webCheckout', 'inApp')),
  "externalAutoApplyDiscounts" INTEGER NOT NULL DEFAULT 1 CHECK ("externalAutoApplyDiscounts" IN (0, 1)),
  CONSTRAINT "EcommerceSettings_serviceFeePercent_check" CHECK ("serviceFeePercent" >= 0 AND "serviceFeePercent" <= 1000000)
) STRICT;
CREATE UNIQUE INDEX "EcommerceSettings_workspaceId_key" ON "EcommerceSettings" ("workspaceId");
CREATE INDEX "EcommerceSettings_openHoursId_idx" ON "EcommerceSettings" ("openHoursId");
CREATE INDEX "EcommerceSettings_abandonedCartFlowId_idx" ON "EcommerceSettings" ("abandonedCartFlowId");
CREATE INDEX "EcommerceSettings_checkoutFlowId_idx" ON "EcommerceSettings" ("checkoutFlowId");
CREATE INDEX "EcommerceSettings_orderConfirmationFlowId_idx" ON "EcommerceSettings" ("orderConfirmationFlowId");

-- EcommerceOrderCounter  (ecommerce-settings)
CREATE TABLE "EcommerceOrderCounter" (
  "workspaceId" INTEGER PRIMARY KEY NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "lastNumber" INTEGER NOT NULL DEFAULT 0
) STRICT;

-- EcommerceOpenHours  (ecommerce-open-hours)
CREATE TABLE "EcommerceOpenHours" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "name" TEXT NOT NULL,
  "ranges" TEXT NOT NULL CHECK (json_valid("ranges"))
) STRICT;
CREATE INDEX "EcommerceOpenHours_workspaceId_idx" ON "EcommerceOpenHours" ("workspaceId");

-- DiscountCode  (discount-code)
-- Unique on lower("code") -> lower_u("code") (app function).
CREATE TABLE "DiscountCode" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "code" TEXT NOT NULL,
  "autoApply" INTEGER NOT NULL DEFAULT 0 CHECK ("autoApply" IN (0, 1)),
  "discountType" TEXT NOT NULL CHECK ("discountType" IN ('percentage', 'fixedAmount', 'freeShipping')),
  "discountValue" INTEGER NOT NULL DEFAULT 0,
  "appliesTo" TEXT NOT NULL DEFAULT 'entireOrder' CHECK ("appliesTo" IN ('entireOrder', 'category', 'products')),
  "categoryId" INTEGER REFERENCES "ProductCategory" ("id") ON DELETE SET NULL ON UPDATE CASCADE,
  "productIds" TEXT NOT NULL CHECK (json_valid("productIds")),
  "minimumType" TEXT NOT NULL DEFAULT 'none' CHECK ("minimumType" IN ('none', 'amount', 'quantity')),
  "minimumValue" INTEGER NOT NULL DEFAULT 0,
  "usageLimit" INTEGER,
  "usedCount" INTEGER NOT NULL DEFAULT 0,
  "oncePerCustomer" INTEGER NOT NULL DEFAULT 0 CHECK ("oncePerCustomer" IN (0, 1)),
  "eligibility" TEXT NOT NULL DEFAULT 'everyone' CHECK ("eligibility" IN ('everyone', 'specificCustomers')),
  "customerFilter" TEXT CHECK ("customerFilter" IS NULL OR json_valid("customerFilter")),
  "startsAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "endsAt" INTEGER,
  "isActive" INTEGER NOT NULL DEFAULT 1 CHECK ("isActive" IN (0, 1)),
  "source" TEXT NOT NULL DEFAULT 'chatbotx' CHECK ("source" IN ('chatbotx', 'shopify', 'woocommerce')),
  "externalId" TEXT,
  CONSTRAINT "DiscountCode_usedCount_check" CHECK ("usedCount" >= 0 AND ("usageLimit" IS NULL OR "usedCount" <= "usageLimit"))
) STRICT;
CREATE UNIQUE INDEX "DiscountCode_workspaceId_code_key" ON "DiscountCode" ("workspaceId", "source", lower_u("code"));
CREATE UNIQUE INDEX "DiscountCode_workspaceId_source_externalId_key" ON "DiscountCode" ("workspaceId", "source", "externalId") WHERE "externalId" IS NOT NULL;
CREATE INDEX "DiscountCode_categoryId_idx" ON "DiscountCode" ("categoryId");

-- DiscountCodeRedemption  (order)
CREATE TABLE "DiscountCodeRedemption" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "discountCodeId" INTEGER NOT NULL REFERENCES "DiscountCode" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "orderId" INTEGER NOT NULL REFERENCES "Order" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "contactId" INTEGER REFERENCES "Contact" ("id") ON DELETE SET NULL ON UPDATE CASCADE,
  "amount" INTEGER NOT NULL
) STRICT;
CREATE UNIQUE INDEX "DiscountCodeRedemption_orderId_key" ON "DiscountCodeRedemption" ("orderId");
CREATE INDEX "DiscountCodeRedemption_discountCodeId_contactId_idx" ON "DiscountCodeRedemption" ("discountCodeId", "contactId");
CREATE INDEX "DiscountCodeRedemption_contactId_idx" ON "DiscountCodeRedemption" ("contactId");

-- Coupon  (coupon)
CREATE TABLE "Coupon" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "topicId" INTEGER NOT NULL REFERENCES "CouponTopic" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "code" TEXT NOT NULL,
  "issuedContactId" INTEGER REFERENCES "Contact" ("id") ON DELETE SET NULL ON UPDATE CASCADE,
  "issuedAt" INTEGER,
  "usedAt" INTEGER
) STRICT;
CREATE UNIQUE INDEX "Coupon_workspaceId_code_key" ON "Coupon" ("workspaceId", "code");
CREATE UNIQUE INDEX "Coupon_workspaceId_topicId_issuedContactId_key" ON "Coupon" ("workspaceId", "topicId", "issuedContactId") WHERE "issuedContactId" IS NOT NULL;
CREATE INDEX "Coupon_workspaceId_topicId_idx" ON "Coupon" ("workspaceId", "topicId");
CREATE INDEX "Coupon_workspaceId_topicId_issuedContactId_idx" ON "Coupon" ("workspaceId", "topicId", "issuedContactId");
CREATE INDEX "Coupon_issuedContactId_idx" ON "Coupon" ("issuedContactId");
CREATE INDEX "Coupon_issuedAt_idx" ON "Coupon" ("issuedAt");
CREATE INDEX "Coupon_usedAt_idx" ON "Coupon" ("usedAt");
CREATE INDEX "Coupon_workspaceId_code_idx" ON "Coupon" ("workspaceId", "code");
CREATE INDEX "Coupon_workspaceId_topicId_issuedContactId_usedAt_idx" ON "Coupon" ("workspaceId", "topicId", "issuedContactId", "usedAt");

-- CouponTopic  (coupon)
-- Partial unique on lower("name") -> lower_u("name") (app function).
CREATE TABLE "CouponTopic" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "name" TEXT NOT NULL,
  "description" TEXT,
  "expiresAt" INTEGER,
  "status" TEXT NOT NULL DEFAULT 'active' CHECK ("status" IN ('active', 'archived')),
  "deletedAt" INTEGER,
  "hasEverHadCoupon" INTEGER NOT NULL DEFAULT 0 CHECK ("hasEverHadCoupon" IN (0, 1)),
  "createdById" INTEGER REFERENCES "User" ("id") ON DELETE SET NULL ON UPDATE CASCADE
) STRICT;
CREATE INDEX "CouponTopic_workspaceId_status_deletedAt_idx" ON "CouponTopic" ("workspaceId", "status", "deletedAt");
CREATE INDEX "CouponTopic_workspaceId_expiresAt_idx" ON "CouponTopic" ("workspaceId", "expiresAt");
CREATE UNIQUE INDEX "CouponTopic_workspaceId_name_idx" ON "CouponTopic" ("workspaceId", lower_u("name")) WHERE "deletedAt" IS NULL;

-- IntegrationShopify  (integration-shopify)
CREATE TABLE "IntegrationShopify" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "shopDomain" TEXT NOT NULL,
  "shopName" TEXT,
  "currency" TEXT NOT NULL,
  "authKind" TEXT NOT NULL CHECK ("authKind" IN ('token', 'clientCredentials')),
  "encryptedAuth" TEXT NOT NULL CHECK (json_valid("encryptedAuth")),
  "webhookSubscriptionIds" TEXT NOT NULL CHECK (json_valid("webhookSubscriptionIds")),
  "status" TEXT NOT NULL DEFAULT 'active' CHECK ("status" IN ('active', 'invalid')),
  "statusReason" TEXT,
  "syncStatus" TEXT NOT NULL DEFAULT 'idle' CHECK ("syncStatus" IN ('idle', 'queued', 'running', 'succeeded', 'failed')),
  "syncError" TEXT,
  "lastSyncedAt" INTEGER,
  "productCount" INTEGER NOT NULL DEFAULT 0,
  "orderCount" INTEGER NOT NULL DEFAULT 0
) STRICT;
CREATE UNIQUE INDEX "IntegrationShopify_workspaceId_key" ON "IntegrationShopify" ("workspaceId");

-- IntegrationWoocommerce  (integration-woocommerce)
CREATE TABLE "IntegrationWoocommerce" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "siteUrl" TEXT NOT NULL,
  "storeName" TEXT,
  "currency" TEXT NOT NULL,
  "encryptedAuth" TEXT NOT NULL CHECK (json_valid("encryptedAuth")),
  "webhookIds" TEXT NOT NULL CHECK (json_valid("webhookIds")),
  "status" TEXT NOT NULL DEFAULT 'active' CHECK ("status" IN ('active', 'invalid')),
  "statusReason" TEXT,
  "syncStatus" TEXT NOT NULL DEFAULT 'idle' CHECK ("syncStatus" IN ('idle', 'queued', 'running', 'succeeded', 'failed')),
  "syncError" TEXT,
  "lastSyncedAt" INTEGER,
  "productCount" INTEGER NOT NULL DEFAULT 0,
  "orderCount" INTEGER NOT NULL DEFAULT 0
) STRICT;
CREATE UNIQUE INDEX "IntegrationWoocommerce_workspaceId_key" ON "IntegrationWoocommerce" ("workspaceId");

-- IntegrationMetaCatalog  (integration-meta-catalog)
CREATE TABLE "IntegrationMetaCatalog" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "integrationId" INTEGER NOT NULL REFERENCES "Integration" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "catalogId" TEXT,
  "catalogName" TEXT,
  "businessId" TEXT,
  "encryptedAuth" TEXT CHECK ("encryptedAuth" IS NULL OR json_valid("encryptedAuth")),
  "authMode" TEXT NOT NULL DEFAULT 'oauth' CHECK ("authMode" IN ('oauth', 'fbe')),
  "tokenExpiresAt" INTEGER,
  "status" TEXT NOT NULL DEFAULT 'active' CHECK ("status" IN ('active', 'invalid')),
  "deletedAt" INTEGER,
  "importStatus" TEXT NOT NULL DEFAULT 'idle' CHECK ("importStatus" IN ('idle', 'queued', 'running', 'succeeded', 'partial', 'failed')),
  "importTotalCount" INTEGER NOT NULL DEFAULT 0,
  "importedCount" INTEGER NOT NULL DEFAULT 0,
  "importFailedCount" INTEGER NOT NULL DEFAULT 0,
  "importError" TEXT,
  "lastImportedAt" INTEGER,
  "currency" TEXT NOT NULL DEFAULT 'VND',
  "storeUrl" TEXT
) STRICT;
CREATE UNIQUE INDEX "IntegrationMetaCatalog_integrationId_key" ON "IntegrationMetaCatalog" ("integrationId");
CREATE UNIQUE INDEX "IntegrationMetaCatalog_workspaceId_key" ON "IntegrationMetaCatalog" ("workspaceId");
CREATE INDEX "IntegrationMetaCatalog_deletedAt_idx" ON "IntegrationMetaCatalog" ("deletedAt");

-- MetaCatalogItem  (meta-catalog-item)
CREATE TABLE "MetaCatalogItem" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "integrationMetaCatalogId" INTEGER NOT NULL REFERENCES "IntegrationMetaCatalog" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "productId" INTEGER NOT NULL REFERENCES "Product" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "catalogId" TEXT NOT NULL,
  "direction" TEXT NOT NULL DEFAULT 'push' CHECK ("direction" IN ('push', 'import')),
  "retailerId" TEXT NOT NULL,
  "lastSyncedFingerprint" TEXT,
  "lastSyncedAt" INTEGER
) STRICT;
CREATE UNIQUE INDEX "MetaCatalogItem_integration_retailer_key" ON "MetaCatalogItem" ("integrationMetaCatalogId", "catalogId", "retailerId");
CREATE UNIQUE INDEX "MetaCatalogItem_integration_product_key" ON "MetaCatalogItem" ("integrationMetaCatalogId", "catalogId", "productId");
CREATE INDEX "MetaCatalogItem_productId_idx" ON "MetaCatalogItem" ("productId");

-- MetaCatalogSyncRun  (meta-catalog-sync-run)
CREATE TABLE "MetaCatalogSyncRun" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "integrationMetaCatalogId" INTEGER NOT NULL REFERENCES "IntegrationMetaCatalog" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "status" TEXT NOT NULL DEFAULT 'queued' CHECK ("status" IN ('queued', 'running', 'succeeded', 'partial', 'failed')),
  "direction" TEXT NOT NULL DEFAULT 'push' CHECK ("direction" IN ('push', 'import')),
  "catalogId" TEXT,
  "scope" TEXT NOT NULL DEFAULT 'all' CHECK ("scope" IN ('all', 'category', 'selected')),
  "categoryId" INTEGER REFERENCES "ProductCategory" ("id") ON DELETE SET NULL ON UPDATE CASCADE,
  "selectedProductIds" TEXT NOT NULL DEFAULT '[]' CHECK (json_valid("selectedProductIds")),
  "handles" TEXT NOT NULL DEFAULT '[]' CHECK (json_valid("handles")),
  "submissionLeaseId" TEXT,
  "totalCount" INTEGER NOT NULL DEFAULT 0,
  "succeededCount" INTEGER NOT NULL DEFAULT 0,
  "failedCount" INTEGER NOT NULL DEFAULT 0,
  "skippedCount" INTEGER NOT NULL DEFAULT 0,
  "itemErrors" TEXT NOT NULL DEFAULT '[]' CHECK (json_valid("itemErrors")),
  "skippedItems" TEXT NOT NULL DEFAULT '[]' CHECK (json_valid("skippedItems")),
  "pollAttempt" INTEGER NOT NULL DEFAULT 0,
  "error" TEXT,
  "startedAt" INTEGER,
  "finishedAt" INTEGER
) STRICT;
CREATE UNIQUE INDEX "MetaCatalogSyncRun_active_idx" ON "MetaCatalogSyncRun" ("workspaceId") WHERE "status" IN ('queued', 'running');

-- ======================== Ads and attribution ========================

-- AdsConversionEvent  (ads-conversion-event)
-- "value" numeric (unbounded) -> TEXT canonical decimal (rust_decimal).
-- Two unique indexes on the same 4 columns (one full, one partial WHERE "channel" = 'whatsapp'): kept as Postgres.
-- DbError constraint-name lookup for a UNIQUE error on these columns returns the non-partial one (P2.3 2.4).
CREATE TABLE "AdsConversionEvent" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "channel" TEXT NOT NULL DEFAULT 'whatsapp' CHECK ("channel" IN ('whatsapp', 'facebook', 'messenger', 'instagram')),
  "integrationWhatsappId" INTEGER REFERENCES "IntegrationWhatsapp" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "integrationMessengerId" INTEGER REFERENCES "IntegrationMessenger" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "integrationInstagramId" INTEGER REFERENCES "IntegrationInstagram" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "wabaId" TEXT,
  "source" TEXT NOT NULL CHECK ("source" IN ('automatic', 'rule', 'trigger')),
  "eventType" TEXT NOT NULL CHECK ("eventType" IN ('lead', 'purchase')),
  "ctwaClid" TEXT,
  "adId" TEXT,
  "contactInboxId" INTEGER REFERENCES "ContactInbox" ("id") ON DELETE SET NULL ON UPDATE CASCADE,
  "referralEventId" INTEGER REFERENCES "ContactReferralEvent" ("id") ON DELETE SET NULL ON UPDATE CASCADE,
  "currency" TEXT,
  "value" TEXT CHECK ("value" IS NULL OR ("value" GLOB '*[0-9]*' AND "value" NOT GLOB '*[^-0-9.]*')),
  "orderId" TEXT,
  "contents" TEXT CHECK ("contents" IS NULL OR json_valid("contents")),
  "occurredAt" INTEGER NOT NULL,
  "sourceEventId" TEXT NOT NULL,
  "capiStatus" TEXT NOT NULL DEFAULT 'pending' CHECK ("capiStatus" IN ('pending', 'sent', 'failed', 'skipped_no_scope', 'skipped_region')),
  "capiSentAt" INTEGER,
  CONSTRAINT "AdsConversionEvent_channel_integration_check" CHECK (("channel" = 'whatsapp' AND "integrationWhatsappId" IS NOT NULL AND "ctwaClid" IS NOT NULL AND "wabaId" IS NOT NULL) OR ("channel" = 'messenger' AND "integrationMessengerId" IS NOT NULL) OR ("channel" = 'instagram' AND "integrationInstagramId" IS NOT NULL))
) STRICT;
CREATE UNIQUE INDEX "AdsConversionEvent_workspace_integration_source_sourceEventId_key" ON "AdsConversionEvent" ("workspaceId", "integrationWhatsappId", "source", "sourceEventId");
CREATE UNIQUE INDEX "AdsConversionEvent_ws_whatsapp_source_sourceEventId_key" ON "AdsConversionEvent" ("workspaceId", "integrationWhatsappId", "source", "sourceEventId") WHERE "channel" = 'whatsapp';
CREATE UNIQUE INDEX "AdsConversionEvent_ws_messenger_source_sourceEventId_key" ON "AdsConversionEvent" ("workspaceId", "integrationMessengerId", "source", "sourceEventId") WHERE "channel" = 'messenger';
CREATE UNIQUE INDEX "AdsConversionEvent_ws_instagram_source_sourceEventId_key" ON "AdsConversionEvent" ("workspaceId", "integrationInstagramId", "source", "sourceEventId") WHERE "channel" = 'instagram';
CREATE INDEX "AdsConversionEvent_workspaceId_eventType_occurredAt_idx" ON "AdsConversionEvent" ("workspaceId", "eventType", "occurredAt");
CREATE INDEX "AdsConversionEvent_contactInboxId_idx" ON "AdsConversionEvent" ("contactInboxId");
CREATE INDEX "AdsConversionEvent_referralEventId_idx" ON "AdsConversionEvent" ("referralEventId") WHERE "referralEventId" IS NOT NULL;
CREATE INDEX "AdsConversionEvent_workspaceId_occurredAt_idx" ON "AdsConversionEvent" ("workspaceId", "occurredAt");
CREATE INDEX "AdsConversionEvent_workspaceId_adId_occurredAt_idx" ON "AdsConversionEvent" ("workspaceId", "adId", "occurredAt");

-- AdsConversionRule  (ads-conversion-rule)
CREATE TABLE "AdsConversionRule" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "channel" TEXT NOT NULL CHECK ("channel" IN ('whatsapp', 'facebook', 'messenger', 'instagram')),
  "integrationWhatsappId" INTEGER REFERENCES "IntegrationWhatsapp" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "integrationFacebookAdsId" INTEGER REFERENCES "IntegrationFacebookAds" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "integrationMessengerId" INTEGER REFERENCES "IntegrationMessenger" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "integrationInstagramId" INTEGER REFERENCES "IntegrationInstagram" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "adAccountId" TEXT,
  "eventType" TEXT NOT NULL CHECK ("eventType" IN ('lead', 'purchase')),
  "trigger" TEXT NOT NULL CHECK (json_valid("trigger")),
  "markAs" TEXT,
  "enabled" INTEGER NOT NULL DEFAULT 1 CHECK ("enabled" IN (0, 1))
) STRICT;
CREATE INDEX "AdsConversionRule_workspaceId_channel_enabled_idx" ON "AdsConversionRule" ("workspaceId", "channel", "enabled");

-- MetaCapiEvent  (meta-capi-event)
-- "value" numeric (unbounded) -> TEXT canonical decimal.
CREATE TABLE "MetaCapiEvent" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "channel" TEXT NOT NULL,
  "integrationId" INTEGER NOT NULL,
  "contactInboxId" INTEGER NOT NULL REFERENCES "ContactInbox" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "eventName" TEXT NOT NULL,
  "currency" TEXT,
  "contentCategory" TEXT,
  "contentName" TEXT,
  "value" TEXT CHECK ("value" IS NULL OR ("value" GLOB '*[0-9]*' AND "value" NOT GLOB '*[^-0-9.]*')),
  "actionSource" TEXT NOT NULL DEFAULT 'business_messaging',
  "contentType" TEXT,
  "contentIds" TEXT CHECK ("contentIds" IS NULL OR json_valid("contentIds")),
  "source" TEXT NOT NULL,
  "sourceKey" TEXT NOT NULL,
  "occurredAt" INTEGER NOT NULL,
  "capiStatus" TEXT NOT NULL DEFAULT 'pending',
  "capiSentAt" INTEGER,
  "capiError" TEXT,
  CONSTRAINT "MetaCapiEvent_channel_check" CHECK ("channel" IN ('messenger', 'instagram', 'whatsapp')),
  CONSTRAINT "MetaCapiEvent_actionSource_check" CHECK ("actionSource" IN ('business_messaging', 'email', 'phone_call', 'chat', 'physical_store', 'system_generated', 'other')),
  CONSTRAINT "MetaCapiEvent_contentType_check" CHECK ("contentType" IN ('product', 'product_group'))
) STRICT;
CREATE UNIQUE INDEX "MetaCapiEvent_workspaceId_channel_sourceKey_key" ON "MetaCapiEvent" ("workspaceId", "channel", "sourceKey");
CREATE INDEX "MetaCapiEvent_contactInboxId_idx" ON "MetaCapiEvent" ("contactInboxId");
CREATE INDEX "MetaCapiEvent_channel_integrationId_idx" ON "MetaCapiEvent" ("channel", "integrationId");

-- GoogleAdsConversionEvent  (google-ads-conversion-event)
-- "value" numeric (unbounded) -> TEXT canonical decimal; GoogleAdsConversionEvent_value_non_negative_check compares CAST("value" AS REAL).
-- "uploadMethod": NOT NULL, no DB default (frozen test), CHECK by name.
CREATE TABLE "GoogleAdsConversionEvent" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "integrationGoogleAdsId" INTEGER REFERENCES "IntegrationGoogleAds" ("id") ON DELETE SET NULL ON UPDATE CASCADE,
  "contactInboxId" INTEGER REFERENCES "ContactInbox" ("id") ON DELETE SET NULL ON UPDATE CASCADE,
  "referralEventId" INTEGER REFERENCES "ContactReferralEvent" ("id") ON DELETE SET NULL ON UPDATE CASCADE,
  "customerId" TEXT NOT NULL,
  "loginCustomerId" TEXT,
  "conversionCustomerId" TEXT NOT NULL,
  "conversionActionId" TEXT NOT NULL,
  "conversionActionName" TEXT,
  "conversionActionCategory" TEXT NOT NULL,
  "lookbackWindowDays" INTEGER,
  "channel" TEXT NOT NULL,
  "source" TEXT NOT NULL CHECK ("source" IN ('flowStep', 'triggerAction')),
  "scopeId" TEXT NOT NULL,
  "clickIdType" TEXT NOT NULL CHECK ("clickIdType" IN ('gclid', 'gbraid')),
  "clickId" TEXT NOT NULL,
  "googleClickReceivedAt" INTEGER NOT NULL,
  "occurredAt" INTEGER NOT NULL,
  "value" TEXT CHECK ("value" IS NULL OR ("value" GLOB '*[0-9]*' AND "value" NOT GLOB '*[^-0-9.]*')),
  "currency" TEXT,
  "transactionId" TEXT NOT NULL,
  "options" TEXT CHECK ("options" IS NULL OR json_valid("options")),
  "uploadMethod" TEXT NOT NULL,
  "status" TEXT NOT NULL CHECK ("status" IN ('pending', 'sending', 'sent', 'processed', 'failed', 'skipped_no_account', 'skipped_expired')),
  "attempt" INTEGER NOT NULL,
  "claimToken" TEXT,
  "claimedAt" INTEGER,
  "requestId" TEXT,
  "sentAt" INTEGER,
  "error" TEXT,
  "failureStage" TEXT CHECK ("failureStage" IN ('delivery', 'processing', 'timeout')),
  "processingStatus" TEXT CHECK ("processingStatus" IN ('processing', 'success', 'partial_success', 'failed', 'unknown', 'timed_out')),
  "processingCheckedAt" INTEGER,
  "processingAttempts" INTEGER NOT NULL,
  "nextProcessingCheckAt" INTEGER,
  "processingDetail" TEXT CHECK ("processingDetail" IS NULL OR json_valid("processingDetail")),
  CONSTRAINT "GoogleAdsConversionEvent_sent_requires_request_check" CHECK ("status" NOT IN ('sent', 'processed') OR ("requestId" IS NOT NULL AND "sentAt" IS NOT NULL)),
  CONSTRAINT "GoogleAdsConversionEvent_sending_requires_claim_check" CHECK ("status" <> 'sending' OR ("claimToken" IS NOT NULL AND "claimedAt" IS NOT NULL)),
  CONSTRAINT "GoogleAdsConversionEvent_processing_status_check" CHECK ("processingStatus" IS NULL OR "status" IN ('sent', 'processed', 'failed')),
  CONSTRAINT "GoogleAdsConversionEvent_failure_stage_check" CHECK ("failureStage" IS NULL OR "status" = 'failed'),
  CONSTRAINT "GoogleAdsConversionEvent_value_currency_check" CHECK (("value" IS NULL) = ("currency" IS NULL)),
  CONSTRAINT "GoogleAdsConversionEvent_value_non_negative_check" CHECK ("value" IS NULL OR CAST("value" AS REAL) >= 0),
  CONSTRAINT "GoogleAdsConversionEvent_clickId_length_check" CHECK (length("clickId") BETWEEN 10 AND 512),
  CONSTRAINT "GoogleAdsConversionEvent_uploadMethod_check" CHECK ("uploadMethod" IN ('dataManager', 'legacy')),
  CONSTRAINT "GoogleAdsConversionEvent_attempt_check" CHECK ("attempt" >= 0 AND "processingAttempts" >= 0)
) STRICT;
CREATE UNIQUE INDEX "GoogleAdsConversionEvent_workspaceId_transactionId_key" ON "GoogleAdsConversionEvent" ("workspaceId", "transactionId");
CREATE INDEX "GoogleAdsConversionEvent_workspaceId_occurredAt_idx" ON "GoogleAdsConversionEvent" ("workspaceId", "occurredAt");
CREATE INDEX "GoogleAdsConversionEvent_workspaceId_status_idx" ON "GoogleAdsConversionEvent" ("workspaceId", "status");
CREATE INDEX "GoogleAdsConversionEvent_contactInboxId_idx" ON "GoogleAdsConversionEvent" ("contactInboxId");
CREATE INDEX "GoogleAdsConversionEvent_referralEventId_idx" ON "GoogleAdsConversionEvent" ("referralEventId") WHERE "referralEventId" IS NOT NULL;
CREATE INDEX "GoogleAdsConversionEvent_status_nextProcessingCheckAt_idx" ON "GoogleAdsConversionEvent" ("status", "nextProcessingCheckAt");
CREATE INDEX "GoogleAdsConversionEvent_status_claimedAt_idx" ON "GoogleAdsConversionEvent" ("status", "claimedAt");

-- GoogleAdsSettings  (google-ads-settings)
CREATE TABLE "GoogleAdsSettings" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "settings" TEXT NOT NULL CHECK (json_valid("settings"))
) STRICT;
CREATE UNIQUE INDEX "GoogleAdsSettings_workspaceId_key" ON "GoogleAdsSettings" ("workspaceId");

-- IntegrationGoogleAds  (integration-google-ads)
CREATE TABLE "IntegrationGoogleAds" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "integrationId" INTEGER NOT NULL REFERENCES "Integration" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "auth" TEXT NOT NULL CHECK (json_valid("auth")),
  "customerId" TEXT NOT NULL,
  "loginCustomerId" TEXT,
  "descriptiveName" TEXT,
  "currencyCode" TEXT,
  "conversionCustomerId" TEXT,
  "acceptedCustomerDataTerms" INTEGER CHECK ("acceptedCustomerDataTerms" IN (0, 1)),
  "conversionActions" TEXT CHECK ("conversionActions" IS NULL OR json_valid("conversionActions")),
  "conversionActionsSyncedAt" INTEGER,
  "setupError" TEXT,
  "setupErrorAt" INTEGER
) STRICT;
CREATE UNIQUE INDEX "IntegrationGoogleAds_integrationId_key" ON "IntegrationGoogleAds" ("integrationId");
CREATE UNIQUE INDEX "IntegrationGoogleAds_workspaceId_key" ON "IntegrationGoogleAds" ("workspaceId");

-- IntegrationFacebookAds  (integration-facebook-ads)
CREATE TABLE "IntegrationFacebookAds" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "integrationId" INTEGER NOT NULL REFERENCES "Integration" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "auth" TEXT NOT NULL CHECK (json_valid("auth")),
  "tokenExpiresAt" INTEGER,
  "status" TEXT NOT NULL DEFAULT 'active'
) STRICT;
CREATE UNIQUE INDEX "IntegrationFacebookAds_integrationId_key" ON "IntegrationFacebookAds" ("integrationId");
CREATE UNIQUE INDEX "IntegrationFacebookAds_workspaceId_key" ON "IntegrationFacebookAds" ("workspaceId");

-- MessagingAdsConnection  (messaging-ads-connection)
CREATE TABLE "MessagingAdsConnection" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "channel" TEXT NOT NULL CHECK ("channel" IN ('whatsapp', 'messenger', 'instagram')),
  "integrationWhatsappId" INTEGER REFERENCES "IntegrationWhatsapp" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "integrationMessengerId" INTEGER REFERENCES "IntegrationMessenger" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "integrationInstagramId" INTEGER REFERENCES "IntegrationInstagram" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "auth" TEXT NOT NULL CHECK (json_valid("auth")),
  "status" TEXT NOT NULL DEFAULT 'active',
  CONSTRAINT "MessagingAdsConnection_channel_integration_check" CHECK (("channel" = 'whatsapp' AND "integrationWhatsappId" IS NOT NULL AND "integrationMessengerId" IS NULL AND "integrationInstagramId" IS NULL) OR ("channel" = 'messenger' AND "integrationMessengerId" IS NOT NULL AND "integrationWhatsappId" IS NULL AND "integrationInstagramId" IS NULL) OR ("channel" = 'instagram' AND "integrationInstagramId" IS NOT NULL AND "integrationWhatsappId" IS NULL AND "integrationMessengerId" IS NULL))
) STRICT;
CREATE INDEX "MessagingAdsConnection_workspaceId_idx" ON "MessagingAdsConnection" ("workspaceId");
CREATE UNIQUE INDEX "MessagingAdsConnection_integrationWhatsappId_key" ON "MessagingAdsConnection" ("integrationWhatsappId");
CREATE UNIQUE INDEX "MessagingAdsConnection_integrationMessengerId_key" ON "MessagingAdsConnection" ("integrationMessengerId");
CREATE UNIQUE INDEX "MessagingAdsConnection_integrationInstagramId_key" ON "MessagingAdsConnection" ("integrationInstagramId");

-- MessagingAdOperation  (messaging-ad-operation)
CREATE TABLE "MessagingAdOperation" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "channel" TEXT NOT NULL CHECK ("channel" IN ('whatsapp', 'messenger', 'instagram')),
  "integrationWhatsappId" INTEGER REFERENCES "IntegrationWhatsapp" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "integrationMessengerId" INTEGER REFERENCES "IntegrationMessenger" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "integrationInstagramId" INTEGER REFERENCES "IntegrationInstagram" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "adAccountId" TEXT NOT NULL,
  "name" TEXT NOT NULL,
  "createState" TEXT NOT NULL DEFAULT 'pending' CHECK ("createState" IN ('pending', 'campaignCreated', 'adSetCreated', 'creativeCreated', 'adCreated', 'failed')),
  "publishState" TEXT NOT NULL DEFAULT 'draft' CHECK ("publishState" IN ('draft', 'publishing', 'published', 'pausing', 'paused', 'deleting', 'deleted', 'publishFailed')),
  "metaCampaignId" TEXT,
  "metaAdSetId" TEXT,
  "metaAdCreativeId" TEXT,
  "metaAdId" TEXT,
  "input" TEXT NOT NULL CHECK (json_valid("input")),
  "lastError" TEXT,
  "cleanupError" TEXT,
  "createdBy" INTEGER REFERENCES "User" ("id") ON DELETE SET NULL ON UPDATE CASCADE,
  CONSTRAINT "MessagingAdOperation_channel_integration_check" CHECK (("channel" = 'whatsapp' AND "integrationWhatsappId" IS NOT NULL AND "integrationMessengerId" IS NULL AND "integrationInstagramId" IS NULL) OR ("channel" = 'messenger' AND "integrationMessengerId" IS NOT NULL AND "integrationWhatsappId" IS NULL AND "integrationInstagramId" IS NULL) OR ("channel" = 'instagram' AND "integrationInstagramId" IS NOT NULL AND "integrationWhatsappId" IS NULL AND "integrationMessengerId" IS NULL))
) STRICT;
CREATE INDEX "MessagingAdOperation_workspaceId_idx" ON "MessagingAdOperation" ("workspaceId");
CREATE INDEX "MessagingAdOperation_integrationWhatsappId_idx" ON "MessagingAdOperation" ("integrationWhatsappId");
CREATE INDEX "MessagingAdOperation_integrationMessengerId_idx" ON "MessagingAdOperation" ("integrationMessengerId");
CREATE INDEX "MessagingAdOperation_integrationInstagramId_idx" ON "MessagingAdOperation" ("integrationInstagramId");

-- FacebookLeadAdsAutomation  (facebook-lead-ads-automation)
CREATE TABLE "FacebookLeadAdsAutomation" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "name" TEXT NOT NULL,
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "pageId" TEXT NOT NULL,
  "pageName" TEXT,
  "formId" TEXT NOT NULL,
  "formName" TEXT,
  "fieldMapping" TEXT NOT NULL DEFAULT '[]' CHECK (json_valid("fieldMapping")),
  "leadsHandledCount" INTEGER NOT NULL DEFAULT 0
) STRICT;
CREATE UNIQUE INDEX "FacebookLeadAdsAutomation_workspaceId_pageId_formId_key" ON "FacebookLeadAdsAutomation" ("workspaceId", "pageId", "formId");

-- FacebookLeadAdsLead  (facebook-lead-ads-lead)
CREATE TABLE "FacebookLeadAdsLead" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "automationId" INTEGER NOT NULL REFERENCES "FacebookLeadAdsAutomation" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "leadgenId" TEXT NOT NULL,
  "formId" TEXT,
  "formName" TEXT,
  "adgroupId" TEXT,
  "contactId" INTEGER REFERENCES "Contact" ("id") ON DELETE SET NULL ON UPDATE CASCADE
) STRICT;
CREATE UNIQUE INDEX "FacebookLeadAdsLead_automationId_leadgenId_key" ON "FacebookLeadAdsLead" ("automationId", "leadgenId");
CREATE INDEX "FacebookLeadAdsLead_leadgenId_idx" ON "FacebookLeadAdsLead" ("leadgenId");
CREATE INDEX "FacebookLeadAdsLead_contactId_idx" ON "FacebookLeadAdsLead" ("contactId") WHERE "contactId" IS NOT NULL;
CREATE INDEX "FacebookLeadAdsLead_contactId_createdAt_idx" ON "FacebookLeadAdsLead" ("contactId", "createdAt" DESC);

-- FacebookMarketingMessage  (facebook-marketing-message)
CREATE TABLE "FacebookMarketingMessage" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "name" TEXT NOT NULL,
  "pageId" TEXT NOT NULL,
  "adAccountId" TEXT NOT NULL,
  "currency" TEXT NOT NULL,
  "currencyOffset" INTEGER NOT NULL,
  "budgetType" TEXT NOT NULL,
  "budgetMinorUnits" INTEGER NOT NULL,
  "content" TEXT NOT NULL CHECK (json_valid("content")),
  "targetType" TEXT NOT NULL DEFAULT 'optIn',
  "audienceId" INTEGER REFERENCES "FacebookMarketingMessageAudience" ("id") ON DELETE RESTRICT ON UPDATE CASCADE,
  "campaignId" TEXT NOT NULL,
  "facebookUserId" TEXT,
  "status" TEXT NOT NULL DEFAULT 'scheduled',
  "statusReason" TEXT,
  "statusErrorCode" INTEGER,
  "statusErrorMessage" TEXT,
  "startsAt" INTEGER,
  "startedAt" INTEGER,
  "firstSentAt" INTEGER,
  "completedAt" INTEGER,
  "nextDayAt" INTEGER,
  "metaEndsAt" INTEGER,
  "timezoneName" TEXT,
  "adSetId" TEXT,
  "tokenCount" INTEGER,
  "dayTarget" INTEGER,
  "daySent" INTEGER NOT NULL DEFAULT 0,
  "resumeCount" INTEGER NOT NULL DEFAULT 0
) STRICT;
CREATE INDEX "FacebookMarketingMessage_workspaceId_idx" ON "FacebookMarketingMessage" ("workspaceId");
CREATE INDEX "FacebookMarketingMessage_audienceId_idx" ON "FacebookMarketingMessage" ("audienceId");
CREATE INDEX "FacebookMarketingMessage_status_startsAt_idx" ON "FacebookMarketingMessage" ("status", "startsAt");
CREATE UNIQUE INDEX "FacebookMarketingMessage_campaignId_key" ON "FacebookMarketingMessage" ("campaignId");

-- FacebookMarketingMessageAudience  (facebook-marketing-message-audience)
CREATE TABLE "FacebookMarketingMessageAudience" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "customAudienceId" TEXT NOT NULL,
  "adAccountId" TEXT NOT NULL,
  "pageId" TEXT NOT NULL,
  "name" TEXT NOT NULL,
  "description" TEXT,
  "rowsUploaded" INTEGER NOT NULL,
  "readyAt" INTEGER,
  "facebookUserId" TEXT
) STRICT;
CREATE INDEX "FacebookMarketingMessageAudience_workspaceId_idx" ON "FacebookMarketingMessageAudience" ("workspaceId");
CREATE UNIQUE INDEX "FacebookMarketingMessageAudience_customAudienceId_key" ON "FacebookMarketingMessageAudience" ("customAudienceId");

-- FacebookMarketingMessageRecipient  (facebook-marketing-message-recipient)
CREATE TABLE "FacebookMarketingMessageRecipient" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "campaignRowId" INTEGER NOT NULL REFERENCES "FacebookMarketingMessage" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "subscriptionToken" TEXT NOT NULL,
  "psid" TEXT,
  "mid" TEXT,
  "trackingId" TEXT,
  "status" TEXT NOT NULL,
  "attempts" INTEGER NOT NULL DEFAULT 0,
  "nextAttemptAt" INTEGER,
  "nextEligibleAt" INTEGER,
  "sentAt" INTEGER,
  "deliveredAt" INTEGER,
  "failedAt" INTEGER,
  "errorCode" INTEGER,
  "errorMessage" TEXT,
  "errorSource" TEXT
) STRICT;
CREATE UNIQUE INDEX "FacebookMarketingMessageRecipient_campaign_token_key" ON "FacebookMarketingMessageRecipient" ("campaignRowId", "subscriptionToken");
CREATE INDEX "FacebookMarketingMessageRecipient_trackingId_idx" ON "FacebookMarketingMessageRecipient" ("trackingId");
CREATE INDEX "FacebookMarketingMessageRecipient_due_idx" ON "FacebookMarketingMessageRecipient" ("campaignRowId", "nextAttemptAt", "id") WHERE "status" = 'pending';

-- FacebookMarketingMessagesAuth  (facebook-marketing-messages-auth)
CREATE TABLE "FacebookMarketingMessagesAuth" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "auth" TEXT NOT NULL CHECK (json_valid("auth")),
  "tokenExpiresAt" INTEGER,
  "status" TEXT NOT NULL DEFAULT 'active',
  "facebookUserId" TEXT
) STRICT;
CREATE UNIQUE INDEX "FacebookMarketingMessagesAuth_workspaceId_key" ON "FacebookMarketingMessagesAuth" ("workspaceId");

-- ContactReferralEvent  (contact-referral-event)
CREATE TABLE "ContactReferralEvent" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "contactId" INTEGER NOT NULL,
  "contactInboxId" INTEGER NOT NULL REFERENCES "ContactInbox" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "channel" TEXT NOT NULL,
  "type" TEXT NOT NULL,
  "provider" TEXT,
  "occurredAt" INTEGER NOT NULL,
  "adId" TEXT,
  "campaignId" TEXT,
  "adGroupId" TEXT,
  "clickIdType" TEXT,
  "clickId" TEXT,
  "ref" TEXT,
  "source" TEXT,
  "sourceUrl" TEXT,
  "metadata" TEXT CHECK ("metadata" IS NULL OR json_valid("metadata")),
  "dedupKey" TEXT NOT NULL,
  CONSTRAINT "ContactReferralEvent_type_check" CHECK ("type" IN ('ad', 'refLink', 'shortLink', 'qr')),
  CONSTRAINT "ContactReferralEvent_provider_check" CHECK ("provider" IS NULL OR "provider" IN ('meta', 'google')),
  CONSTRAINT "ContactReferralEvent_clickIdType_check" CHECK ("clickIdType" IS NULL OR "clickIdType" IN ('ctwa', 'gclid', 'gbraid')),
  CONSTRAINT "ContactReferralEvent_clickId_check" CHECK (("clickId" IS NULL) = ("clickIdType" IS NULL))
) STRICT;
CREATE UNIQUE INDEX "ContactReferralEvent_workspaceId_contactInboxId_dedupKey_key" ON "ContactReferralEvent" ("workspaceId", "contactInboxId", "dedupKey");
CREATE INDEX "ContactReferralEvent_contactInboxId_idx" ON "ContactReferralEvent" ("contactInboxId");
CREATE INDEX "ContactReferralEvent_workspaceId_contactId_occurredAt_idx" ON "ContactReferralEvent" ("workspaceId", "contactId", "occurredAt");
CREATE INDEX "ContactReferralEvent_workspaceId_type_occurredAt_idx" ON "ContactReferralEvent" ("workspaceId", "type", "occurredAt") WHERE "adId" IS NOT NULL;

-- Reflink  (reflink)
CREATE TABLE "Reflink" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "name" TEXT NOT NULL,
  "type" TEXT NOT NULL DEFAULT 'refLink' CHECK ("type" IN ('refLink', 'qrCode')),
  "flowId" INTEGER NOT NULL REFERENCES "Flow" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "customFieldId" INTEGER REFERENCES "CustomField" ("id") ON DELETE SET NULL ON UPDATE CASCADE,
  "qrStyles" TEXT CHECK ("qrStyles" IS NULL OR json_valid("qrStyles")),
  "widgetAuthorizedDomains" TEXT NOT NULL DEFAULT '[]' CHECK (json_valid("widgetAuthorizedDomains") AND json_type("widgetAuthorizedDomains") = 'array'),
  "widgetHiddenInboxIds" TEXT NOT NULL DEFAULT '[]' CHECK (json_valid("widgetHiddenInboxIds") AND json_type("widgetHiddenInboxIds") = 'array'),
  "widgetLogoFileId" INTEGER REFERENCES "MediaLibraryFile" ("id") ON DELETE SET NULL ON UPDATE CASCADE,
  "widgetBrandName" TEXT,
  "widgetBrandUrl" TEXT,
  "widgetLogoBackgroundColor" TEXT
) STRICT;
CREATE UNIQUE INDEX "Reflink_workspaceId_name_key" ON "Reflink" ("workspaceId", "name");

-- RefLinkStat  (ref-link-stat)
-- No PK in Postgres (unique key only) -> rowid table.
CREATE TABLE "RefLinkStat" (
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "linkId" INTEGER NOT NULL REFERENCES "Reflink" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "contactId" INTEGER NOT NULL,
  "contactInboxId" INTEGER NOT NULL,
  "occurredAt" INTEGER NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER))
) STRICT;
CREATE INDEX "RefLinkStat_contactId_idx" ON "RefLinkStat" ("contactId");
CREATE UNIQUE INDEX "RefLinkStat_workspaceId_linkId_occurredAt_contactInboxId_key" ON "RefLinkStat" ("workspaceId", "linkId", "occurredAt", "contactInboxId");

-- MagicLink  (magic-link)
CREATE TABLE "MagicLink" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "name" TEXT NOT NULL,
  "url" TEXT NOT NULL
) STRICT;
CREATE UNIQUE INDEX "MagicLink_workspaceId_name_key" ON "MagicLink" ("workspaceId", "name");
CREATE INDEX "MagicLink_workspaceId_idx" ON "MagicLink" ("workspaceId");

-- MagicLinkStat  (magic-link-stat)
-- No PK in Postgres (unique key only) -> rowid table.
CREATE TABLE "MagicLinkStat" (
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "linkId" INTEGER NOT NULL REFERENCES "MagicLink" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "contactId" INTEGER NOT NULL,
  "contactInboxId" INTEGER NOT NULL,
  "occurredAt" INTEGER NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER))
) STRICT;
CREATE UNIQUE INDEX "MagicLinkStat_workspaceId_linkId_occurredAt_contactInboxId_key" ON "MagicLinkStat" ("workspaceId", "linkId", "occurredAt", "contactInboxId");

-- ======================== Analytics, usage and billing ========================

-- AnalyticsContactEvent  (analytics/contact-event)
-- TimescaleDB hypertable (30-day chunks, compression 30 d, retention 10 y) -> plain table, PK ("occurredAt", "eventId") kept
-- (insert is ON CONFLICT DO NOTHING on it = event-bus dedup). Retention: P2.6 schedule job purgeAnalyticsEvents.
-- Rollup "analytics_contact_events_hourly" is kept by the triggers below (same transaction as the event write).
CREATE TABLE "AnalyticsContactEvent" (
  "eventId" TEXT NOT NULL,
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE NO ACTION,
  "contactId" INTEGER NOT NULL,
  "eventType" TEXT NOT NULL CHECK ("eventType" IN ('contact_created', 'contact_deleted', 'contact_blocked')),
  "occurredAt" INTEGER NOT NULL,
  "source" TEXT,
  "sourceId" TEXT,
  "channel" TEXT,
  "inboxId" INTEGER,
  "language" TEXT,
  "country" TEXT,
  "senderType" TEXT CHECK ("senderType" IN ('bot', 'human')),
  "adminId" INTEGER,
  "metadata" TEXT CHECK ("metadata" IS NULL OR json_valid("metadata")),
  "insertedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  CONSTRAINT "AnalyticsContactEvent_pkey" PRIMARY KEY ("occurredAt", "eventId")
) STRICT;
CREATE INDEX "AnalyticsContactEvent_workspaceId_occurredAt_eventType_idx" ON "AnalyticsContactEvent" ("workspaceId", "occurredAt", "eventType");
CREATE INDEX "AnalyticsContactEvent_workspaceId_eventType_occurredAt_idx" ON "AnalyticsContactEvent" ("workspaceId", "eventType", "occurredAt");
CREATE INDEX "AnalyticsContactEvent_workspaceId_adminId_occurredAt_idx" ON "AnalyticsContactEvent" ("workspaceId", "adminId", "occurredAt");
-- SQL-only in Postgres (migration 20260517074023): ... ("workspaceId", "eventType", "occurredAt" DESC) INCLUDE ("contactId",
-- "channel", "country", "source") -> the INCLUDE columns appended to the key (P2.2 0).
CREATE INDEX "AnalyticsContactEvent_workspaceId_eventType_occurredAt_covering_idx" ON "AnalyticsContactEvent" ("workspaceId", "eventType", "occurredAt" DESC, "contactId", "channel", "country", "source");

-- AnalyticsBotMessageEvent  (analytics/bot-message-event)
-- Hypertable -> plain table (as AnalyticsContactEvent). Rollup "analytics_bot_message_events_hourly".
CREATE TABLE "AnalyticsBotMessageEvent" (
  "eventId" TEXT NOT NULL,
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE NO ACTION,
  "messageId" INTEGER NOT NULL,
  "conversationId" INTEGER NOT NULL,
  "occurredAt" INTEGER NOT NULL,
  "hasResponse" INTEGER NOT NULL DEFAULT 0 CHECK ("hasResponse" IN (0, 1)),
  "responseType" TEXT CHECK ("responseType" IN ('automated_response', 'ai_agent', 'flow', 'none')),
  "routeType" TEXT CHECK ("routeType" IN ('flow', 'agent', 'fallback')),
  "result" TEXT CHECK ("result" IN ('success', 'fallback')),
  "aiProvider" TEXT,
  "channel" TEXT,
  "inboxId" INTEGER,
  "source" TEXT,
  "metadata" TEXT CHECK ("metadata" IS NULL OR json_valid("metadata")),
  "insertedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  CONSTRAINT "AnalyticsBotMessageEvent_pkey" PRIMARY KEY ("occurredAt", "eventId")
) STRICT;
CREATE INDEX "AnalyticsBotMessageEvent_workspaceId_occurredAt_idx" ON "AnalyticsBotMessageEvent" ("workspaceId", "occurredAt");
CREATE INDEX "AnalyticsBotMessageEvent_workspaceId_aiProvider_occurredAt_idx" ON "AnalyticsBotMessageEvent" ("workspaceId", "aiProvider", "occurredAt");
CREATE INDEX "AnalyticsBotMessageEvent_workspaceId_hasResponse_result_occurredAt_idx" ON "AnalyticsBotMessageEvent" ("workspaceId", "hasResponse", "result", "occurredAt");

-- AnalyticsConversationEvent  (analytics/conversation-event)
-- Hypertable -> plain table. Rollup "analytics_conversation_events_hourly".
CREATE TABLE "AnalyticsConversationEvent" (
  "eventId" TEXT NOT NULL,
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE NO ACTION,
  "conversationId" INTEGER NOT NULL,
  "eventType" TEXT NOT NULL CHECK ("eventType" IN ('conversation_created', 'conversation_assigned', 'conversation_unassigned', 'conversation_transferred_to_human', 'conversation_transferred_to_bot', 'conversation_followed', 'conversation_unfollowed', 'conversation_archived', 'conversation_unarchived')),
  "occurredAt" INTEGER NOT NULL,
  "fromAssignee" INTEGER,
  "toAssignee" INTEGER,
  "channel" TEXT,
  "inboxId" INTEGER,
  "metadata" TEXT CHECK ("metadata" IS NULL OR json_valid("metadata")),
  "insertedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  CONSTRAINT "AnalyticsConversationEvent_pkey" PRIMARY KEY ("occurredAt", "eventId")
) STRICT;
CREATE INDEX "AnalyticsConversationEvent_workspaceId_occurredAt_eventType_idx" ON "AnalyticsConversationEvent" ("workspaceId", "occurredAt", "eventType");
CREATE INDEX "AnalyticsConversationEvent_workspaceId_toAssignee_occurredAt_idx" ON "AnalyticsConversationEvent" ("workspaceId", "toAssignee", "occurredAt");

-- AnalyticsMessageEvent  (analytics/message-event)
-- Hypertable -> plain table. Rollup "analytics_message_events_hourly".
CREATE TABLE "AnalyticsMessageEvent" (
  "eventId" TEXT NOT NULL,
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE NO ACTION,
  "contactId" INTEGER NOT NULL,
  "eventType" TEXT NOT NULL CHECK ("eventType" IN ('message_human_sent', 'message_bot_sent')),
  "occurredAt" INTEGER NOT NULL,
  "senderType" TEXT CHECK ("senderType" IN ('bot', 'human')),
  "adminId" INTEGER,
  "channel" TEXT,
  "inboxId" INTEGER,
  "source" TEXT,
  "sourceId" TEXT,
  "metadata" TEXT CHECK ("metadata" IS NULL OR json_valid("metadata")),
  "insertedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  CONSTRAINT "AnalyticsMessageEvent_pkey" PRIMARY KEY ("occurredAt", "eventId")
) STRICT;
CREATE INDEX "AnalyticsMessageEvent_workspaceId_occurredAt_eventType_idx" ON "AnalyticsMessageEvent" ("workspaceId", "occurredAt", "eventType");
CREATE INDEX "AnalyticsMessageEvent_workspaceId_eventType_occurredAt_idx" ON "AnalyticsMessageEvent" ("workspaceId", "eventType", "occurredAt");
CREATE INDEX "AnalyticsMessageEvent_workspaceId_adminId_occurredAt_idx" ON "AnalyticsMessageEvent" ("workspaceId", "adminId", "occurredAt");
CREATE INDEX "AnalyticsMessageEvent_workspaceId_senderType_occurredAt_idx" ON "AnalyticsMessageEvent" ("workspaceId", "senderType", "occurredAt");

-- AnalyticsAgentResponseEvent  (analytics/agent-response-event)
-- Hypertable (no continuous aggregate) -> plain table; retention 10 y by purgeAnalyticsEvents.
CREATE TABLE "AnalyticsAgentResponseEvent" (
  "eventId" TEXT NOT NULL,
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE NO ACTION,
  "conversationId" INTEGER NOT NULL,
  "contactId" INTEGER NOT NULL,
  "adminId" INTEGER NOT NULL,
  "occurredAt" INTEGER NOT NULL,
  "awaitingSince" INTEGER NOT NULL,
  "measuredFrom" INTEGER NOT NULL,
  "responseSeconds" INTEGER NOT NULL,
  "workingResponseSeconds" INTEGER,
  "isFirstResponse" INTEGER NOT NULL CHECK ("isFirstResponse" IN (0, 1)),
  "workingHoursId" INTEGER,
  "slaTargetSeconds" INTEGER,
  "slaMet" INTEGER CHECK ("slaMet" IN (0, 1)),
  "channel" TEXT,
  "inboxId" INTEGER,
  "metadata" TEXT CHECK ("metadata" IS NULL OR json_valid("metadata")),
  "insertedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  CONSTRAINT "AnalyticsAgentResponseEvent_pkey" PRIMARY KEY ("occurredAt", "eventId")
) STRICT;
CREATE INDEX "AnalyticsAgentResponseEvent_workspaceId_occurredAt_idx" ON "AnalyticsAgentResponseEvent" ("workspaceId", "occurredAt");

-- ======================== Rollup tables (replace the 4 continuous aggregates) ========================

-- analytics_contact_events_hourly: replaces the TimescaleDB continuous aggregate of the same name (real-time, materialized_only = FALSE;
-- refresh every 5 min over the last 7 d; Postgres retention 3 years). Same columns and names, so the ported analytics SQL
-- keeps "FROM <name>" / "bucket" / "count". "bucket" = UTC hour start in ms = time_bucket('1 hour', "occurredAt").
-- Kept exact in the same transaction as every event write by the 3 triggers below (insert, delete, update of a key column);
-- an ON CONFLICT DO NOTHING duplicate fires no trigger. NULL group values stay NULL; the unique key maps NULL to x''
-- (a BLOB never equals a TEXT or INTEGER value, so NULL and '' / 0 stay separate groups, as in GROUP BY). No row has count 0.
CREATE TABLE "analytics_contact_events_hourly" (
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE NO ACTION,
  "bucket" INTEGER NOT NULL,
  "eventType" TEXT NOT NULL CHECK ("eventType" IN ('contact_created', 'contact_deleted', 'contact_blocked')),
  "channel" TEXT,
  "senderType" TEXT CHECK ("senderType" IN ('bot', 'human')),
  "country" TEXT,
  "source" TEXT,
  "adminId" INTEGER,
  "count" INTEGER NOT NULL CHECK ("count" > 0)
) STRICT;
CREATE UNIQUE INDEX "analytics_contact_events_hourly_key" ON "analytics_contact_events_hourly" ("workspaceId", "bucket", "eventType", ifnull("channel", x''), ifnull("senderType", x''), ifnull("country", x''), ifnull("source", x''), ifnull("adminId", x''));
CREATE TRIGGER "AnalyticsContactEvent_rollup_ai" AFTER INSERT ON "AnalyticsContactEvent" BEGIN
  INSERT INTO "analytics_contact_events_hourly" ("workspaceId", "bucket", "eventType", "channel", "senderType", "country", "source", "adminId", "count")
  VALUES (NEW."workspaceId", (NEW."occurredAt" / 3600000) * 3600000, NEW."eventType", NEW."channel", NEW."senderType", NEW."country", NEW."source", NEW."adminId", 1)
  ON CONFLICT ("workspaceId", "bucket", "eventType", ifnull("channel", x''), ifnull("senderType", x''), ifnull("country", x''), ifnull("source", x''), ifnull("adminId", x'')) DO UPDATE SET "count" = "count" + 1;
END;
CREATE TRIGGER "AnalyticsContactEvent_rollup_ad" AFTER DELETE ON "AnalyticsContactEvent" BEGIN
  DELETE FROM "analytics_contact_events_hourly" WHERE "workspaceId" = OLD."workspaceId" AND "bucket" = (OLD."occurredAt" / 3600000) * 3600000 AND "eventType" = OLD."eventType" AND ifnull("channel", x'') = ifnull(OLD."channel", x'') AND ifnull("senderType", x'') = ifnull(OLD."senderType", x'') AND ifnull("country", x'') = ifnull(OLD."country", x'') AND ifnull("source", x'') = ifnull(OLD."source", x'') AND ifnull("adminId", x'') = ifnull(OLD."adminId", x'') AND "count" = 1;
  UPDATE "analytics_contact_events_hourly" SET "count" = "count" - 1 WHERE "workspaceId" = OLD."workspaceId" AND "bucket" = (OLD."occurredAt" / 3600000) * 3600000 AND "eventType" = OLD."eventType" AND ifnull("channel", x'') = ifnull(OLD."channel", x'') AND ifnull("senderType", x'') = ifnull(OLD."senderType", x'') AND ifnull("country", x'') = ifnull(OLD."country", x'') AND ifnull("source", x'') = ifnull(OLD."source", x'') AND ifnull("adminId", x'') = ifnull(OLD."adminId", x'');
END;
CREATE TRIGGER "AnalyticsContactEvent_rollup_au" AFTER UPDATE OF "workspaceId", "occurredAt", "eventType", "channel", "senderType", "country", "source", "adminId" ON "AnalyticsContactEvent" BEGIN
  DELETE FROM "analytics_contact_events_hourly" WHERE "workspaceId" = OLD."workspaceId" AND "bucket" = (OLD."occurredAt" / 3600000) * 3600000 AND "eventType" = OLD."eventType" AND ifnull("channel", x'') = ifnull(OLD."channel", x'') AND ifnull("senderType", x'') = ifnull(OLD."senderType", x'') AND ifnull("country", x'') = ifnull(OLD."country", x'') AND ifnull("source", x'') = ifnull(OLD."source", x'') AND ifnull("adminId", x'') = ifnull(OLD."adminId", x'') AND "count" = 1;
  UPDATE "analytics_contact_events_hourly" SET "count" = "count" - 1 WHERE "workspaceId" = OLD."workspaceId" AND "bucket" = (OLD."occurredAt" / 3600000) * 3600000 AND "eventType" = OLD."eventType" AND ifnull("channel", x'') = ifnull(OLD."channel", x'') AND ifnull("senderType", x'') = ifnull(OLD."senderType", x'') AND ifnull("country", x'') = ifnull(OLD."country", x'') AND ifnull("source", x'') = ifnull(OLD."source", x'') AND ifnull("adminId", x'') = ifnull(OLD."adminId", x'');
  INSERT INTO "analytics_contact_events_hourly" ("workspaceId", "bucket", "eventType", "channel", "senderType", "country", "source", "adminId", "count")
  VALUES (NEW."workspaceId", (NEW."occurredAt" / 3600000) * 3600000, NEW."eventType", NEW."channel", NEW."senderType", NEW."country", NEW."source", NEW."adminId", 1)
  ON CONFLICT ("workspaceId", "bucket", "eventType", ifnull("channel", x''), ifnull("senderType", x''), ifnull("country", x''), ifnull("source", x''), ifnull("adminId", x'')) DO UPDATE SET "count" = "count" + 1;
END;

-- analytics_bot_message_events_hourly: replaces the TimescaleDB continuous aggregate of the same name (real-time, materialized_only = FALSE;
-- refresh every 5 min over the last 7 d; Postgres retention 3 years). Same columns and names, so the ported analytics SQL
-- keeps "FROM <name>" / "bucket" / "count". "bucket" = UTC hour start in ms = time_bucket('1 hour', "occurredAt").
-- Kept exact in the same transaction as every event write by the 3 triggers below (insert, delete, update of a key column);
-- an ON CONFLICT DO NOTHING duplicate fires no trigger. NULL group values stay NULL; the unique key maps NULL to x''
-- (a BLOB never equals a TEXT or INTEGER value, so NULL and '' / 0 stay separate groups, as in GROUP BY). No row has count 0.
CREATE TABLE "analytics_bot_message_events_hourly" (
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE NO ACTION,
  "bucket" INTEGER NOT NULL,
  "hasResponse" INTEGER NOT NULL CHECK ("hasResponse" IN (0, 1)),
  "responseType" TEXT CHECK ("responseType" IN ('automated_response', 'ai_agent', 'flow', 'none')),
  "routeType" TEXT CHECK ("routeType" IN ('flow', 'agent', 'fallback')),
  "result" TEXT CHECK ("result" IN ('success', 'fallback')),
  "aiProvider" TEXT,
  "count" INTEGER NOT NULL CHECK ("count" > 0)
) STRICT;
CREATE UNIQUE INDEX "analytics_bot_message_events_hourly_key" ON "analytics_bot_message_events_hourly" ("workspaceId", "bucket", "hasResponse", ifnull("responseType", x''), ifnull("routeType", x''), ifnull("result", x''), ifnull("aiProvider", x''));
CREATE TRIGGER "AnalyticsBotMessageEvent_rollup_ai" AFTER INSERT ON "AnalyticsBotMessageEvent" BEGIN
  INSERT INTO "analytics_bot_message_events_hourly" ("workspaceId", "bucket", "hasResponse", "responseType", "routeType", "result", "aiProvider", "count")
  VALUES (NEW."workspaceId", (NEW."occurredAt" / 3600000) * 3600000, NEW."hasResponse", NEW."responseType", NEW."routeType", NEW."result", NEW."aiProvider", 1)
  ON CONFLICT ("workspaceId", "bucket", "hasResponse", ifnull("responseType", x''), ifnull("routeType", x''), ifnull("result", x''), ifnull("aiProvider", x'')) DO UPDATE SET "count" = "count" + 1;
END;
CREATE TRIGGER "AnalyticsBotMessageEvent_rollup_ad" AFTER DELETE ON "AnalyticsBotMessageEvent" BEGIN
  DELETE FROM "analytics_bot_message_events_hourly" WHERE "workspaceId" = OLD."workspaceId" AND "bucket" = (OLD."occurredAt" / 3600000) * 3600000 AND "hasResponse" = OLD."hasResponse" AND ifnull("responseType", x'') = ifnull(OLD."responseType", x'') AND ifnull("routeType", x'') = ifnull(OLD."routeType", x'') AND ifnull("result", x'') = ifnull(OLD."result", x'') AND ifnull("aiProvider", x'') = ifnull(OLD."aiProvider", x'') AND "count" = 1;
  UPDATE "analytics_bot_message_events_hourly" SET "count" = "count" - 1 WHERE "workspaceId" = OLD."workspaceId" AND "bucket" = (OLD."occurredAt" / 3600000) * 3600000 AND "hasResponse" = OLD."hasResponse" AND ifnull("responseType", x'') = ifnull(OLD."responseType", x'') AND ifnull("routeType", x'') = ifnull(OLD."routeType", x'') AND ifnull("result", x'') = ifnull(OLD."result", x'') AND ifnull("aiProvider", x'') = ifnull(OLD."aiProvider", x'');
END;
CREATE TRIGGER "AnalyticsBotMessageEvent_rollup_au" AFTER UPDATE OF "workspaceId", "occurredAt", "hasResponse", "responseType", "routeType", "result", "aiProvider" ON "AnalyticsBotMessageEvent" BEGIN
  DELETE FROM "analytics_bot_message_events_hourly" WHERE "workspaceId" = OLD."workspaceId" AND "bucket" = (OLD."occurredAt" / 3600000) * 3600000 AND "hasResponse" = OLD."hasResponse" AND ifnull("responseType", x'') = ifnull(OLD."responseType", x'') AND ifnull("routeType", x'') = ifnull(OLD."routeType", x'') AND ifnull("result", x'') = ifnull(OLD."result", x'') AND ifnull("aiProvider", x'') = ifnull(OLD."aiProvider", x'') AND "count" = 1;
  UPDATE "analytics_bot_message_events_hourly" SET "count" = "count" - 1 WHERE "workspaceId" = OLD."workspaceId" AND "bucket" = (OLD."occurredAt" / 3600000) * 3600000 AND "hasResponse" = OLD."hasResponse" AND ifnull("responseType", x'') = ifnull(OLD."responseType", x'') AND ifnull("routeType", x'') = ifnull(OLD."routeType", x'') AND ifnull("result", x'') = ifnull(OLD."result", x'') AND ifnull("aiProvider", x'') = ifnull(OLD."aiProvider", x'');
  INSERT INTO "analytics_bot_message_events_hourly" ("workspaceId", "bucket", "hasResponse", "responseType", "routeType", "result", "aiProvider", "count")
  VALUES (NEW."workspaceId", (NEW."occurredAt" / 3600000) * 3600000, NEW."hasResponse", NEW."responseType", NEW."routeType", NEW."result", NEW."aiProvider", 1)
  ON CONFLICT ("workspaceId", "bucket", "hasResponse", ifnull("responseType", x''), ifnull("routeType", x''), ifnull("result", x''), ifnull("aiProvider", x'')) DO UPDATE SET "count" = "count" + 1;
END;

-- analytics_conversation_events_hourly: replaces the TimescaleDB continuous aggregate of the same name (real-time, materialized_only = FALSE;
-- refresh every 5 min over the last 7 d; Postgres retention 3 years). Same columns and names, so the ported analytics SQL
-- keeps "FROM <name>" / "bucket" / "count". "bucket" = UTC hour start in ms = time_bucket('1 hour', "occurredAt").
-- Kept exact in the same transaction as every event write by the 3 triggers below (insert, delete, update of a key column);
-- an ON CONFLICT DO NOTHING duplicate fires no trigger. NULL group values stay NULL; the unique key maps NULL to x''
-- (a BLOB never equals a TEXT or INTEGER value, so NULL and '' / 0 stay separate groups, as in GROUP BY). No row has count 0.
CREATE TABLE "analytics_conversation_events_hourly" (
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE NO ACTION,
  "bucket" INTEGER NOT NULL,
  "eventType" TEXT NOT NULL CHECK ("eventType" IN ('conversation_created', 'conversation_assigned', 'conversation_unassigned', 'conversation_transferred_to_human', 'conversation_transferred_to_bot', 'conversation_followed', 'conversation_unfollowed', 'conversation_archived', 'conversation_unarchived')),
  "toAssignee" INTEGER,
  "count" INTEGER NOT NULL CHECK ("count" > 0)
) STRICT;
CREATE UNIQUE INDEX "analytics_conversation_events_hourly_key" ON "analytics_conversation_events_hourly" ("workspaceId", "bucket", "eventType", ifnull("toAssignee", x''));
CREATE TRIGGER "AnalyticsConversationEvent_rollup_ai" AFTER INSERT ON "AnalyticsConversationEvent" BEGIN
  INSERT INTO "analytics_conversation_events_hourly" ("workspaceId", "bucket", "eventType", "toAssignee", "count")
  VALUES (NEW."workspaceId", (NEW."occurredAt" / 3600000) * 3600000, NEW."eventType", NEW."toAssignee", 1)
  ON CONFLICT ("workspaceId", "bucket", "eventType", ifnull("toAssignee", x'')) DO UPDATE SET "count" = "count" + 1;
END;
CREATE TRIGGER "AnalyticsConversationEvent_rollup_ad" AFTER DELETE ON "AnalyticsConversationEvent" BEGIN
  DELETE FROM "analytics_conversation_events_hourly" WHERE "workspaceId" = OLD."workspaceId" AND "bucket" = (OLD."occurredAt" / 3600000) * 3600000 AND "eventType" = OLD."eventType" AND ifnull("toAssignee", x'') = ifnull(OLD."toAssignee", x'') AND "count" = 1;
  UPDATE "analytics_conversation_events_hourly" SET "count" = "count" - 1 WHERE "workspaceId" = OLD."workspaceId" AND "bucket" = (OLD."occurredAt" / 3600000) * 3600000 AND "eventType" = OLD."eventType" AND ifnull("toAssignee", x'') = ifnull(OLD."toAssignee", x'');
END;
CREATE TRIGGER "AnalyticsConversationEvent_rollup_au" AFTER UPDATE OF "workspaceId", "occurredAt", "eventType", "toAssignee" ON "AnalyticsConversationEvent" BEGIN
  DELETE FROM "analytics_conversation_events_hourly" WHERE "workspaceId" = OLD."workspaceId" AND "bucket" = (OLD."occurredAt" / 3600000) * 3600000 AND "eventType" = OLD."eventType" AND ifnull("toAssignee", x'') = ifnull(OLD."toAssignee", x'') AND "count" = 1;
  UPDATE "analytics_conversation_events_hourly" SET "count" = "count" - 1 WHERE "workspaceId" = OLD."workspaceId" AND "bucket" = (OLD."occurredAt" / 3600000) * 3600000 AND "eventType" = OLD."eventType" AND ifnull("toAssignee", x'') = ifnull(OLD."toAssignee", x'');
  INSERT INTO "analytics_conversation_events_hourly" ("workspaceId", "bucket", "eventType", "toAssignee", "count")
  VALUES (NEW."workspaceId", (NEW."occurredAt" / 3600000) * 3600000, NEW."eventType", NEW."toAssignee", 1)
  ON CONFLICT ("workspaceId", "bucket", "eventType", ifnull("toAssignee", x'')) DO UPDATE SET "count" = "count" + 1;
END;

-- analytics_message_events_hourly: replaces the TimescaleDB continuous aggregate of the same name (real-time, materialized_only = FALSE;
-- refresh every 5 min over the last 7 d; Postgres retention 10 years). Same columns and names, so the ported analytics SQL
-- keeps "FROM <name>" / "bucket" / "count". "bucket" = UTC hour start in ms = time_bucket('1 hour', "occurredAt").
-- Kept exact in the same transaction as every event write by the 3 triggers below (insert, delete, update of a key column);
-- an ON CONFLICT DO NOTHING duplicate fires no trigger. NULL group values stay NULL; the unique key maps NULL to x''
-- (a BLOB never equals a TEXT or INTEGER value, so NULL and '' / 0 stay separate groups, as in GROUP BY). No row has count 0.
CREATE TABLE "analytics_message_events_hourly" (
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE NO ACTION,
  "bucket" INTEGER NOT NULL,
  "eventType" TEXT NOT NULL CHECK ("eventType" IN ('message_human_sent', 'message_bot_sent')),
  "channel" TEXT,
  "senderType" TEXT CHECK ("senderType" IN ('bot', 'human')),
  "adminId" INTEGER,
  "count" INTEGER NOT NULL CHECK ("count" > 0)
) STRICT;
CREATE UNIQUE INDEX "analytics_message_events_hourly_key" ON "analytics_message_events_hourly" ("workspaceId", "bucket", "eventType", ifnull("channel", x''), ifnull("senderType", x''), ifnull("adminId", x''));
CREATE TRIGGER "AnalyticsMessageEvent_rollup_ai" AFTER INSERT ON "AnalyticsMessageEvent" BEGIN
  INSERT INTO "analytics_message_events_hourly" ("workspaceId", "bucket", "eventType", "channel", "senderType", "adminId", "count")
  VALUES (NEW."workspaceId", (NEW."occurredAt" / 3600000) * 3600000, NEW."eventType", NEW."channel", NEW."senderType", NEW."adminId", 1)
  ON CONFLICT ("workspaceId", "bucket", "eventType", ifnull("channel", x''), ifnull("senderType", x''), ifnull("adminId", x'')) DO UPDATE SET "count" = "count" + 1;
END;
CREATE TRIGGER "AnalyticsMessageEvent_rollup_ad" AFTER DELETE ON "AnalyticsMessageEvent" BEGIN
  DELETE FROM "analytics_message_events_hourly" WHERE "workspaceId" = OLD."workspaceId" AND "bucket" = (OLD."occurredAt" / 3600000) * 3600000 AND "eventType" = OLD."eventType" AND ifnull("channel", x'') = ifnull(OLD."channel", x'') AND ifnull("senderType", x'') = ifnull(OLD."senderType", x'') AND ifnull("adminId", x'') = ifnull(OLD."adminId", x'') AND "count" = 1;
  UPDATE "analytics_message_events_hourly" SET "count" = "count" - 1 WHERE "workspaceId" = OLD."workspaceId" AND "bucket" = (OLD."occurredAt" / 3600000) * 3600000 AND "eventType" = OLD."eventType" AND ifnull("channel", x'') = ifnull(OLD."channel", x'') AND ifnull("senderType", x'') = ifnull(OLD."senderType", x'') AND ifnull("adminId", x'') = ifnull(OLD."adminId", x'');
END;
CREATE TRIGGER "AnalyticsMessageEvent_rollup_au" AFTER UPDATE OF "workspaceId", "occurredAt", "eventType", "channel", "senderType", "adminId" ON "AnalyticsMessageEvent" BEGIN
  DELETE FROM "analytics_message_events_hourly" WHERE "workspaceId" = OLD."workspaceId" AND "bucket" = (OLD."occurredAt" / 3600000) * 3600000 AND "eventType" = OLD."eventType" AND ifnull("channel", x'') = ifnull(OLD."channel", x'') AND ifnull("senderType", x'') = ifnull(OLD."senderType", x'') AND ifnull("adminId", x'') = ifnull(OLD."adminId", x'') AND "count" = 1;
  UPDATE "analytics_message_events_hourly" SET "count" = "count" - 1 WHERE "workspaceId" = OLD."workspaceId" AND "bucket" = (OLD."occurredAt" / 3600000) * 3600000 AND "eventType" = OLD."eventType" AND ifnull("channel", x'') = ifnull(OLD."channel", x'') AND ifnull("senderType", x'') = ifnull(OLD."senderType", x'') AND ifnull("adminId", x'') = ifnull(OLD."adminId", x'');
  INSERT INTO "analytics_message_events_hourly" ("workspaceId", "bucket", "eventType", "channel", "senderType", "adminId", "count")
  VALUES (NEW."workspaceId", (NEW."occurredAt" / 3600000) * 3600000, NEW."eventType", NEW."channel", NEW."senderType", NEW."adminId", 1)
  ON CONFLICT ("workspaceId", "bucket", "eventType", ifnull("channel", x''), ifnull("senderType", x''), ifnull("adminId", x'')) DO UPDATE SET "count" = "count" + 1;
END;

-- AnalyticsEmailTopic  (analytics-email-topic)
CREATE TABLE "AnalyticsEmailTopic" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "topicId" INTEGER REFERENCES "EmailTopic" ("id") ON DELETE SET NULL ON UPDATE CASCADE,
  "topicReferenceId" INTEGER,
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "contactId" INTEGER REFERENCES "Contact" ("id") ON DELETE SET NULL ON UPDATE CASCADE,
  "conversationId" INTEGER REFERENCES "Conversation" ("id") ON DELETE SET NULL ON UPDATE CASCADE,
  "contactInboxId" INTEGER REFERENCES "ContactInbox" ("id") ON DELETE SET NULL ON UPDATE CASCADE,
  "email" TEXT NOT NULL,
  "token" TEXT NOT NULL,
  "deliveredAt" INTEGER,
  "failedAt" INTEGER,
  "firstSeenAt" INTEGER,
  "lastSeenAt" INTEGER,
  "seenCount" INTEGER NOT NULL DEFAULT 0,
  "firstClickedAt" INTEGER,
  "lastClickedAt" INTEGER,
  "clickCount" INTEGER NOT NULL DEFAULT 0
) STRICT;
CREATE UNIQUE INDEX "AnalyticsEmailTopic_token_key" ON "AnalyticsEmailTopic" ("token");
CREATE INDEX "AnalyticsEmailTopic_topicId_idx" ON "AnalyticsEmailTopic" ("topicId");
CREATE INDEX "AnalyticsEmailTopic_workspaceId_topicId_idx" ON "AnalyticsEmailTopic" ("workspaceId", "topicId");
CREATE INDEX "AnalyticsEmailTopic_contactId_idx" ON "AnalyticsEmailTopic" ("contactId") WHERE "contactId" IS NOT NULL;
CREATE INDEX "AnalyticsEmailTopic_conversationId_idx" ON "AnalyticsEmailTopic" ("conversationId") WHERE "conversationId" IS NOT NULL;
CREATE INDEX "AnalyticsEmailTopic_contactInboxId_idx" ON "AnalyticsEmailTopic" ("contactInboxId") WHERE "contactInboxId" IS NOT NULL;

-- AnalyticsManifestStatus  (analytics-manifest-status)
-- No PK in Postgres (unique "objectKey" only) -> rowid table. "ingestedAt" is timestamp WITHOUT time zone -> INTEGER ms UTC.
CREATE TABLE "AnalyticsManifestStatus" (
  "objectKey" TEXT NOT NULL,
  "status" TEXT NOT NULL CHECK ("status" IN ('processing', 'ingested', 'failed')),
  "attempts" INTEGER NOT NULL DEFAULT 0,
  "ingestedAt" INTEGER,
  "lastError" TEXT
) STRICT;
CREATE UNIQUE INDEX "AnalyticsManifestStatus_objectKey_key" ON "AnalyticsManifestStatus" ("objectKey");

-- ContactActiveHourly  (contact-active-hourly)
-- RANGE partitioned monthly by "hourBucket" (+ DEFAULT partition) -> one table, same PK. maintainMacPartitions has nothing to create.
CREATE TABLE "ContactActiveHourly" (
  "workspaceId" INTEGER NOT NULL,
  "contactId" INTEGER NOT NULL,
  "contactInboxId" INTEGER NOT NULL,
  "hourBucket" INTEGER NOT NULL,
  "inboxId" INTEGER NOT NULL,
  CONSTRAINT "ContactActiveHourly_pkey" PRIMARY KEY ("workspaceId", "hourBucket", "contactInboxId")
) STRICT;

-- ContactActiveMonthly  (contact-active-monthly)
-- RANGE partitioned yearly by "periodStart" -> one table, same PK.
CREATE TABLE "ContactActiveMonthly" (
  "workspaceId" INTEGER NOT NULL,
  "contactId" INTEGER NOT NULL,
  "contactInboxId" INTEGER NOT NULL,
  "periodStart" INTEGER NOT NULL,
  "inboxId" INTEGER NOT NULL,
  "workspaceMacId" INTEGER NOT NULL,
  CONSTRAINT "ContactActiveMonthly_pkey" PRIMARY KEY ("workspaceId", "periodStart", "contactInboxId")
) STRICT;

-- WorkspaceMac  (workspace-mac)
CREATE TABLE "WorkspaceMac" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "workspaceId" INTEGER NOT NULL,
  "ownerId" INTEGER REFERENCES "User" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "tenantId" INTEGER,
  "periodStart" INTEGER NOT NULL,
  "periodEnd" INTEGER NOT NULL,
  "macCount" INTEGER NOT NULL DEFAULT 0,
  CONSTRAINT "WorkspaceMac_workspaceId_periodStart_periodEnd_unique" UNIQUE ("workspaceId", "periodStart", "periodEnd")
) STRICT;
CREATE INDEX "WorkspaceMac_ownerId_periodStart_idx" ON "WorkspaceMac" ("ownerId", "periodStart");
CREATE INDEX "WorkspaceMac_tenantId_periodStart_idx" ON "WorkspaceMac" ("tenantId", "periodStart");

-- WorkspaceBotMessageUsage  (workspace-bot-message-usage)
CREATE TABLE "WorkspaceBotMessageUsage" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "workspaceId" INTEGER NOT NULL,
  "ownerId" INTEGER NOT NULL REFERENCES "User" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "tenantId" INTEGER NOT NULL,
  "periodStart" INTEGER NOT NULL,
  "periodEnd" INTEGER NOT NULL,
  "count" INTEGER NOT NULL DEFAULT 0,
  CONSTRAINT "WorkspaceBotMessageUsage_workspaceId_periodStart_key" UNIQUE ("workspaceId", "periodStart")
) STRICT;
CREATE INDEX "WorkspaceBotMessageUsage_ownerId_periodStart_idx" ON "WorkspaceBotMessageUsage" ("ownerId", "periodStart");
CREATE INDEX "WorkspaceBotMessageUsage_tenantId_periodStart_idx" ON "WorkspaceBotMessageUsage" ("tenantId", "periodStart");

-- ======================== AI and RAG ========================

-- AIAgent  (ai-agent)
-- "actionRules": DB default ARRAY[]::jsonb[] -> DEFAULT '[]' (frozen schema-default-parity :118).
-- "webSearchAuthorizedDomains": the migration 20260518143845 created DEFAULT '{}'::text[] (the snapshot shows '' = drift);
-- the frozen DEFAULTS_MISSING_FROM_DATABASE list does not name it, so the DB has the default -> DEFAULT '[]'.
CREATE TABLE "AIAgent" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "name" TEXT NOT NULL,
  "prompt" TEXT,
  "actionPrompt" TEXT,
  "actionRules" TEXT NOT NULL DEFAULT '[]' CHECK (json_valid("actionRules") AND json_type("actionRules") = 'array'),
  "messages" TEXT NOT NULL CHECK (json_valid("messages") AND json_type("messages") = 'array'),  -- NO DB default (Postgres parity: drizzle .default(sql`[]`) is app-side only). Rust always writes it.
  "isDefault" INTEGER NOT NULL DEFAULT 0 CHECK ("isDefault" IN (0, 1)),
  "isRichResponse" INTEGER NOT NULL DEFAULT 0 CHECK ("isRichResponse" IN (0, 1)),
  "tools" TEXT NOT NULL CHECK (json_valid("tools") AND json_type("tools") = 'array'),  -- NO DB default (Postgres parity: drizzle .default(sql`[]`) is app-side only). Rust always writes it.
  "webSearchAuthorizedDomains" TEXT NOT NULL DEFAULT '[]' CHECK (json_valid("webSearchAuthorizedDomains") AND json_type("webSearchAuthorizedDomains") = 'array'),
  "models" TEXT NOT NULL CHECK (json_valid("models") AND json_type("models") = 'array'),  -- NO DB default (Postgres parity: drizzle .default(sql`[]`) is app-side only). Rust always writes it.
  "temperature" REAL NOT NULL,
  "maxOutputTokens" INTEGER NOT NULL
) STRICT;
CREATE INDEX "AIAgent_workspaceId_idx" ON "AIAgent" ("workspaceId");

-- AIFunction  (ai-function)
CREATE TABLE "AIFunction" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "name" TEXT NOT NULL,
  "purpose" TEXT,
  "dataCollect" TEXT CHECK ("dataCollect" IS NULL OR json_valid("dataCollect")),
  "outputMessage" TEXT,
  "triggerFlowId" INTEGER REFERENCES "Flow" ("id") ON DELETE SET NULL ON UPDATE CASCADE,
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE
) STRICT;

-- AIMCPServer  (ai-mcp-server)
CREATE TABLE "AIMCPServer" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "name" TEXT NOT NULL,
  "url" TEXT NOT NULL,
  "auth" TEXT NOT NULL CHECK (json_valid("auth")),
  "availableTools" TEXT NOT NULL CHECK (json_valid("availableTools")),
  "selectedTools" TEXT NOT NULL CHECK (json_valid("selectedTools") AND json_type("selectedTools") = 'array'),  -- NO DB default (Postgres parity: drizzle .default(sql`[]`) is app-side only). Rust always writes it.
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE
) STRICT;

-- AIFile  (ai-file)
-- Expression partial index on "processingRun"->>... works as is (SQLite >= 3.38); queries must use the same expression text.
CREATE TABLE "AIFile" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "name" TEXT NOT NULL,
  "path" TEXT NOT NULL,
  "size" INTEGER NOT NULL,
  "mimeType" TEXT NOT NULL,
  "chunking" TEXT CHECK ("chunking" IS NULL OR json_valid("chunking")),
  "chunkingConfig" TEXT CHECK ("chunkingConfig" IS NULL OR json_valid("chunkingConfig")),
  "processingRun" TEXT CHECK ("processingRun" IS NULL OR json_valid("processingRun")),
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE
) STRICT;
CREATE INDEX "AIFile_workspaceId_idx" ON "AIFile" ("workspaceId");
CREATE INDEX "AIFile_queuedAt_queued_idx" ON "AIFile" (("processingRun"->>'queuedAt')) WHERE "processingRun"->>'status' = 'queued';

-- AIEmbedding  (ai-embedding)
-- Vector column: section 2 of the P2.5 spec. Exact filtered cosine scan with vector_distance_cos (P2.3 3.6); no ANN index.
CREATE TABLE "AIEmbedding" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "content" TEXT NOT NULL,
  "embedding" BLOB CHECK ("embedding" IS NULL OR length("embedding") = 6144),  -- pgvector vector(1536) -> 1536 little-endian f32 = 6144 bytes (libSQL F32_BLOB layout); STRICT allows BLOB
  "status" TEXT NOT NULL DEFAULT 'pending' CHECK ("status" IN ('pending', 'success', 'error', 'processing')),
  "chunkIndex" INTEGER,
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "aiFileId" INTEGER NOT NULL REFERENCES "AIFile" ("id") ON DELETE CASCADE ON UPDATE CASCADE
) STRICT;
CREATE INDEX "AIEmbedding_workspaceId_idx" ON "AIEmbedding" ("workspaceId");
CREATE INDEX "AIEmbedding_aiFileId_chunkIndex_idx" ON "AIEmbedding" ("aiFileId", "chunkIndex");

-- AIConversationSource  (ai-conversation-source)
CREATE TABLE "AIConversationSource" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "conversationId" INTEGER NOT NULL REFERENCES "Conversation" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "messageId" INTEGER NOT NULL,
  "attachmentId" INTEGER,
  "sourceType" TEXT NOT NULL CHECK ("sourceType" IN ('document', 'image', 'url', 'web_search')),
  "status" TEXT NOT NULL DEFAULT 'pending' CHECK ("status" IN ('pending', 'processing', 'success', 'error')),
  "sourceKey" TEXT NOT NULL,
  "contentHash" TEXT,
  "mimeType" TEXT,
  "title" TEXT,
  "metadata" TEXT CHECK ("metadata" IS NULL OR json_valid("metadata")),
  "summary" TEXT,
  "errorMessage" TEXT
) STRICT;
CREATE INDEX "AIConversationSource_lookup_idx" ON "AIConversationSource" ("workspaceId", "conversationId", "sourceType", "status");
CREATE UNIQUE INDEX "AIConversationSource_workspaceId_sourceType_sourceKey_key" ON "AIConversationSource" ("workspaceId", "sourceType", "sourceKey");
CREATE INDEX "AIConversationSource_messageId_idx" ON "AIConversationSource" ("messageId");
CREATE INDEX "AIConversationSource_conversationId_idx" ON "AIConversationSource" ("conversationId");

-- AIConversationEmbedding  (ai-conversation-embedding)
-- Vector column: as AIEmbedding. The snapshot HNSW index AIConversationEmbedding_embedding_idx was never created by a
-- migration (drift, P2.2 0) and is not created here.
CREATE TABLE "AIConversationEmbedding" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "sourceId" INTEGER NOT NULL REFERENCES "AIConversationSource" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "conversationId" INTEGER NOT NULL REFERENCES "Conversation" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "chunkIndex" INTEGER NOT NULL,
  "content" TEXT NOT NULL,
  "embedding" BLOB CHECK ("embedding" IS NULL OR length("embedding") = 6144),  -- pgvector vector(1536) -> 1536 little-endian f32 = 6144 bytes (libSQL F32_BLOB layout); STRICT allows BLOB
  "status" TEXT NOT NULL DEFAULT 'pending' CHECK ("status" IN ('pending', 'success', 'error', 'processing')),
  "errorMessage" TEXT
) STRICT;
CREATE INDEX "AIConversationEmbedding_sourceId_status_idx" ON "AIConversationEmbedding" ("sourceId", "status");
CREATE UNIQUE INDEX "AIConversationEmbedding_sourceId_chunkIndex_key" ON "AIConversationEmbedding" ("sourceId", "chunkIndex");
CREATE INDEX "AIConversationEmbedding_conversationId_idx" ON "AIConversationEmbedding" ("conversationId");
CREATE INDEX "AIConversationEmbedding_lookup_idx" ON "AIConversationEmbedding" ("workspaceId", "conversationId", "sourceId", "status");

-- AIHandoverSettings  (ai-handover-settings)
CREATE TABLE "AIHandoverSettings" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "inboxId" INTEGER NOT NULL REFERENCES "Inbox" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "channel" TEXT NOT NULL,
  "enabled" INTEGER NOT NULL DEFAULT 0 CHECK ("enabled" IN (0, 1)),
  "scheduleEnabled" INTEGER NOT NULL DEFAULT 0 CHECK ("scheduleEnabled" IN (0, 1)),
  "timeRanges" TEXT NOT NULL CHECK (json_valid("timeRanges")),
  "gotoFlowId" INTEGER REFERENCES "Flow" ("id") ON DELETE SET NULL ON UPDATE CASCADE,
  "returnMessage" TEXT,
  "pauseBotWaitingForStaff" INTEGER NOT NULL DEFAULT 0 CHECK ("pauseBotWaitingForStaff" IN (0, 1)),
  "applyToAllCustomers" INTEGER NOT NULL DEFAULT 0 CHECK ("applyToAllCustomers" IN (0, 1)),
  "applyToAllRevision" INTEGER NOT NULL DEFAULT 0,
  "applyToAllMessage" TEXT,
  "applyToAllRequestedByUserId" INTEGER REFERENCES "User" ("id") ON DELETE SET NULL ON UPDATE CASCADE
) STRICT;
CREATE UNIQUE INDEX "AIHandoverSettings_inboxId_key" ON "AIHandoverSettings" ("inboxId");
CREATE INDEX "AIHandoverSettings_workspaceId_idx" ON "AIHandoverSettings" ("workspaceId");
CREATE INDEX "AIHandoverSettings_applyToAllRequestedByUserId_idx" ON "AIHandoverSettings" ("applyToAllRequestedByUserId");
CREATE INDEX "AIHandoverSettings_gotoFlowId_idx" ON "AIHandoverSettings" ("gotoFlowId");

-- AIHandoverBulkRun  (ai-handover-bulk-run)
CREATE TABLE "AIHandoverBulkRun" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "inboxId" INTEGER NOT NULL REFERENCES "Inbox" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "channel" TEXT NOT NULL,
  "revision" INTEGER NOT NULL,
  "action" TEXT NOT NULL CHECK ("action" IN ('enable', 'disable')),
  "status" TEXT NOT NULL CHECK ("status" IN ('pending', 'running', 'cancelling', 'completed', 'failed', 'cancelled')),
  "message" TEXT,
  "requestedByUserId" INTEGER REFERENCES "User" ("id") ON DELETE SET NULL ON UPDATE CASCADE,
  "requestedAt" INTEGER NOT NULL,
  "startedAt" INTEGER,
  "finishedAt" INTEGER,
  "lastHeartbeatAt" INTEGER,
  "pausedUntil" INTEGER,
  "claimToken" TEXT,
  "attempts" INTEGER NOT NULL,
  "chunkSeq" INTEGER NOT NULL,
  "totalCount" INTEGER,
  "processedCount" INTEGER NOT NULL,
  "skippedCount" INTEGER NOT NULL,
  "failedCount" INTEGER NOT NULL,
  "cursorContactInboxId" TEXT,
  "inFlightFromId" TEXT,
  "inFlightToId" TEXT,
  "currentError" TEXT
) STRICT;
CREATE UNIQUE INDEX "AIHandoverBulkRun_inbox_live_uq" ON "AIHandoverBulkRun" ("inboxId") WHERE status IN ('pending', 'running', 'cancelling');
CREATE UNIQUE INDEX "AIHandoverBulkRun_inbox_revision_uq" ON "AIHandoverBulkRun" ("inboxId", "revision");
CREATE INDEX "AIHandoverBulkRun_workspace_idx" ON "AIHandoverBulkRun" ("workspaceId");
CREATE INDEX "AIHandoverBulkRun_inbox_createdAt_idx" ON "AIHandoverBulkRun" ("inboxId", "createdAt" DESC);
CREATE INDEX "AIHandoverBulkRun_live_idx" ON "AIHandoverBulkRun" ("status", "lastHeartbeatAt") WHERE status IN ('pending', 'running', 'cancelling');
CREATE INDEX "AIHandoverBulkRun_requestedByUserId_idx" ON "AIHandoverBulkRun" ("requestedByUserId");

-- DecisionConnection  (decision-connection)
CREATE TABLE "DecisionConnection" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "name" TEXT NOT NULL,
  "providerKind" TEXT NOT NULL CHECK ("providerKind" IN ('typesafe', 'systemOneCompatible', 'openrouterDecision')),
  "endpoint" TEXT,
  "credential" TEXT NOT NULL CHECK (json_valid("credential")),
  "modelCatalog" TEXT NOT NULL CHECK (json_valid("modelCatalog")),
  "defaultModel" TEXT,
  "status" TEXT NOT NULL DEFAULT 'enabled' CHECK ("status" IN ('enabled', 'disabled')),
  "lastTestStatus" TEXT CHECK ("lastTestStatus" IN ('passed', 'failed')),
  "lastTestedAt" INTEGER
) STRICT;
CREATE INDEX "DecisionConnection_workspaceId_idx" ON "DecisionConnection" ("workspaceId");
CREATE UNIQUE INDEX "DecisionConnection_workspaceId_name_key" ON "DecisionConnection" ("workspaceId", "name");

-- DecisionProfile  (decision-profile)
-- Partial unique WHERE "globalRouterEnabled" = true -> = 1.
CREATE TABLE "DecisionProfile" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "name" TEXT NOT NULL,
  "description" TEXT,
  "status" TEXT NOT NULL DEFAULT 'enabled' CHECK ("status" IN ('enabled', 'disabled')),
  "connectionId" INTEGER NOT NULL REFERENCES "DecisionConnection" ("id") ON DELETE RESTRICT ON UPDATE CASCADE,
  "model" TEXT NOT NULL,
  "contract" TEXT NOT NULL CHECK (json_valid("contract")),
  "thresholdConfig" TEXT CHECK ("thresholdConfig" IS NULL OR json_valid("thresholdConfig")),
  "globalRouterEnabled" INTEGER NOT NULL DEFAULT 0 CHECK ("globalRouterEnabled" IN (0, 1)),
  "globalRouterConfig" TEXT CHECK ("globalRouterConfig" IS NULL OR json_valid("globalRouterConfig"))
) STRICT;
CREATE INDEX "DecisionProfile_workspaceId_idx" ON "DecisionProfile" ("workspaceId");
CREATE INDEX "DecisionProfile_connectionId_idx" ON "DecisionProfile" ("connectionId");
CREATE UNIQUE INDEX "DecisionProfile_workspaceId_name_key" ON "DecisionProfile" ("workspaceId", "name");
CREATE UNIQUE INDEX "DecisionProfile_workspaceId_globalRouter_key" ON "DecisionProfile" ("workspaceId") WHERE "globalRouterEnabled" = 1;

-- IntegrationOpenai  (integration-openai)
CREATE TABLE "IntegrationOpenai" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "auth" TEXT NOT NULL CHECK (json_valid("auth")),
  "autoReply" INTEGER NOT NULL DEFAULT 1 CHECK ("autoReply" IN (0, 1)),
  "autoReplyVoice" INTEGER NOT NULL DEFAULT 0 CHECK ("autoReplyVoice" IN (0, 1)),
  "voice" TEXT,
  "prompt" TEXT,
  "model" TEXT NOT NULL,
  "temperature" REAL,
  "maxOutputTokens" INTEGER NOT NULL,
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "integrationId" INTEGER NOT NULL REFERENCES "Integration" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "aiAgentId" INTEGER REFERENCES "AIAgent" ("id") ON DELETE SET NULL ON UPDATE CASCADE
) STRICT;
CREATE UNIQUE INDEX "IntegrationOpenAI_workspaceId_key" ON "IntegrationOpenai" ("workspaceId");
CREATE UNIQUE INDEX "IntegrationOpenAI_integrationId_key" ON "IntegrationOpenai" ("integrationId");

-- IntegrationOpenaiCompatible  (integration-openai-compatible)
CREATE TABLE "IntegrationOpenaiCompatible" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "auth" TEXT CHECK ("auth" IS NULL OR json_valid("auth")),
  "autoReply" INTEGER NOT NULL DEFAULT 0 CHECK ("autoReply" IN (0, 1)),
  "baseURL" TEXT NOT NULL,
  "defaultModel" TEXT NOT NULL,
  "enabled" INTEGER NOT NULL DEFAULT 1 CHECK ("enabled" IN (0, 1)),
  "integrationId" INTEGER NOT NULL REFERENCES "Integration" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "name" TEXT NOT NULL,
  "preset" TEXT NOT NULL,
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE
) STRICT;
CREATE INDEX "IntegrationOpenaiCompatible_workspaceId_idx" ON "IntegrationOpenaiCompatible" ("workspaceId");
CREATE UNIQUE INDEX "IntegrationOpenaiCompatible_integrationId_key" ON "IntegrationOpenaiCompatible" ("integrationId");
CREATE UNIQUE INDEX "IntegrationOpenaiCompatible_workspaceId_preset_key" ON "IntegrationOpenaiCompatible" ("workspaceId", "preset") WHERE "preset" <> 'custom';

-- IntegrationClaude  (integration-claude)
CREATE TABLE "IntegrationClaude" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "auth" TEXT NOT NULL CHECK (json_valid("auth")),
  "autoReply" INTEGER NOT NULL DEFAULT 0 CHECK ("autoReply" IN (0, 1)),
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "integrationId" INTEGER NOT NULL REFERENCES "Integration" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "maxOutputTokens" INTEGER NOT NULL,
  "model" TEXT NOT NULL,
  "prompt" TEXT,
  "temperature" REAL
) STRICT;
CREATE UNIQUE INDEX "IntegrationClaude_workspaceId_key" ON "IntegrationClaude" ("workspaceId");
CREATE UNIQUE INDEX "IntegrationClaude_integrationId_key" ON "IntegrationClaude" ("integrationId");

-- IntegrationDeepseek  (integration-deepseek)
CREATE TABLE "IntegrationDeepseek" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "auth" TEXT NOT NULL CHECK (json_valid("auth")),
  "autoReply" INTEGER NOT NULL DEFAULT 0 CHECK ("autoReply" IN (0, 1)),
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "integrationId" INTEGER NOT NULL REFERENCES "Integration" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "maxOutputTokens" INTEGER NOT NULL,
  "model" TEXT NOT NULL,
  "prompt" TEXT,
  "temperature" REAL
) STRICT;
CREATE UNIQUE INDEX "IntegrationDeepseek_workspaceId_key" ON "IntegrationDeepseek" ("workspaceId");
CREATE UNIQUE INDEX "IntegrationDeepseek_integrationId_key" ON "IntegrationDeepseek" ("integrationId");

-- IntegrationGemini  (integration-gemini)
CREATE TABLE "IntegrationGemini" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "auth" TEXT NOT NULL CHECK (json_valid("auth")),
  "autoReply" INTEGER NOT NULL DEFAULT 0 CHECK ("autoReply" IN (0, 1)),
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "integrationId" INTEGER NOT NULL REFERENCES "Integration" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "maxOutputTokens" INTEGER NOT NULL,
  "model" TEXT NOT NULL,
  "prompt" TEXT,
  "temperature" REAL
) STRICT;
CREATE UNIQUE INDEX "IntegrationGemini_workspaceId_key" ON "IntegrationGemini" ("workspaceId");
CREATE UNIQUE INDEX "IntegrationGemini_integrationId_key" ON "IntegrationGemini" ("integrationId");

-- IntegrationOpenrouter  (integration-openrouter)
CREATE TABLE "IntegrationOpenrouter" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "auth" TEXT NOT NULL CHECK (json_valid("auth")),
  "autoReply" INTEGER NOT NULL DEFAULT 0 CHECK ("autoReply" IN (0, 1)),
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "integrationId" INTEGER NOT NULL REFERENCES "Integration" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "maxOutputTokens" INTEGER NOT NULL,
  "model" TEXT NOT NULL,
  "prompt" TEXT,
  "temperature" REAL
) STRICT;
CREATE UNIQUE INDEX "IntegrationOpenrouter_workspaceId_key" ON "IntegrationOpenrouter" ("workspaceId");
CREATE UNIQUE INDEX "IntegrationOpenrouter_integrationId_key" ON "IntegrationOpenrouter" ("integrationId");

-- IntegrationMuse  (integration-muse)
CREATE TABLE "IntegrationMuse" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "auth" TEXT NOT NULL CHECK (json_valid("auth")),
  "autoReply" INTEGER NOT NULL DEFAULT 0 CHECK ("autoReply" IN (0, 1)),
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "integrationId" INTEGER NOT NULL REFERENCES "Integration" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "maxOutputTokens" INTEGER NOT NULL,
  "model" TEXT NOT NULL,
  "prompt" TEXT,
  "temperature" REAL
) STRICT;
CREATE UNIQUE INDEX "IntegrationMuse_workspaceId_key" ON "IntegrationMuse" ("workspaceId");
CREATE UNIQUE INDEX "IntegrationMuse_integrationId_key" ON "IntegrationMuse" ("integrationId");

-- ======================== Minigames, leaderboards, polls, questionnaires, mini apps ========================

-- Minigame  (minigame)
CREATE TABLE "Minigame" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "name" TEXT NOT NULL,
  "type" TEXT NOT NULL CHECK ("type" IN ('luckyWheel', 'jackpot', 'gashapon', 'drawLots', 'scratchOff')),
  "enabled" INTEGER NOT NULL DEFAULT 1 CHECK ("enabled" IN (0, 1)),
  "generalSettings" TEXT NOT NULL CHECK (json_valid("generalSettings")),
  "appearance" TEXT NOT NULL CHECK (json_valid("appearance")),
  "playerSettings" TEXT NOT NULL CHECK (json_valid("playerSettings")),
  "prizeSettings" TEXT NOT NULL CHECK (json_valid("prizeSettings")),
  "winningMessageSettings" TEXT NOT NULL CHECK (json_valid("winningMessageSettings")),
  "nonWinningMessageSettings" TEXT NOT NULL CHECK (json_valid("nonWinningMessageSettings")),
  "playsCount" INTEGER NOT NULL DEFAULT 0,
  "participantsCount" INTEGER NOT NULL DEFAULT 0,
  "winnersCount" INTEGER NOT NULL DEFAULT 0,
  "sharesCount" INTEGER NOT NULL DEFAULT 0
) STRICT;
CREATE INDEX "Minigame_workspaceId_idx" ON "Minigame" ("workspaceId");
CREATE UNIQUE INDEX "Minigame_workspaceId_name_key" ON "Minigame" ("workspaceId", "name");

-- MinigameContact  (minigame-contact)
CREATE TABLE "MinigameContact" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "minigameId" INTEGER NOT NULL REFERENCES "Minigame" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "contactId" INTEGER NOT NULL REFERENCES "Contact" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "openedAt" INTEGER NOT NULL,
  "played" INTEGER NOT NULL DEFAULT 0,
  "remaining" INTEGER NOT NULL DEFAULT 0,
  "sharesCount" INTEGER NOT NULL DEFAULT 0,
  "referrerContactId" INTEGER REFERENCES "Contact" ("id") ON DELETE SET NULL ON UPDATE CASCADE,
  "contactInboxId" INTEGER REFERENCES "ContactInbox" ("id") ON DELETE SET NULL ON UPDATE CASCADE
) STRICT;
CREATE INDEX "MinigameContact_minigameId_idx" ON "MinigameContact" ("minigameId");
CREATE INDEX "MinigameContact_contactId_idx" ON "MinigameContact" ("contactId");
CREATE UNIQUE INDEX "MinigameContact_minigameId_contactId_key" ON "MinigameContact" ("minigameId", "contactId");
CREATE INDEX "MinigameContact_referrerContactId_idx" ON "MinigameContact" ("referrerContactId") WHERE "referrerContactId" IS NOT NULL;
CREATE INDEX "MinigameContact_contactInboxId_idx" ON "MinigameContact" ("contactInboxId") WHERE "contactInboxId" IS NOT NULL;

-- MinigamePlay  (minigame-play)
CREATE TABLE "MinigamePlay" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "minigameId" INTEGER NOT NULL REFERENCES "Minigame" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "contactId" INTEGER NOT NULL REFERENCES "Contact" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "isWinning" INTEGER NOT NULL CHECK ("isWinning" IN (0, 1)),
  "prizeId" TEXT,
  "prizeName" TEXT
) STRICT;
CREATE INDEX "MinigamePlay_minigameId_contactId_idx" ON "MinigamePlay" ("minigameId", "contactId");
CREATE INDEX "MinigamePlay_contactId_idx" ON "MinigamePlay" ("contactId");

-- Leaderboard  (leaderboard)
CREATE TABLE "Leaderboard" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "name" TEXT NOT NULL,
  "description" TEXT NOT NULL DEFAULT '',
  "coverImageUrl" TEXT NOT NULL DEFAULT '',
  "startsAt" INTEGER NOT NULL,
  "endsAt" INTEGER NOT NULL,
  "status" TEXT NOT NULL DEFAULT 'draft' CHECK ("status" IN ('draft', 'active', 'paused', 'finished')),
  "platform" TEXT NOT NULL CHECK ("platform" IN ('messenger', 'instagram', 'whatsapp', 'zalo', 'telegram', 'tiktok', 'threads', 'email', 'gamification')),
  "inboxId" INTEGER REFERENCES "Inbox" ("id") ON DELETE SET NULL ON UPDATE CASCADE,
  "minigameId" INTEGER REFERENCES "Minigame" ("id") ON DELETE SET NULL ON UPDATE CASCADE,
  "scope" TEXT NOT NULL CHECK (json_valid("scope")),
  "rules" TEXT NOT NULL CHECK (json_valid("rules")),
  "followersOnly" INTEGER NOT NULL DEFAULT 0 CHECK ("followersOnly" IN (0, 1)),
  "tagIds" TEXT NOT NULL CHECK (json_valid("tagIds")),
  "resetTime" TEXT NOT NULL DEFAULT '22:00',
  "cooldownSeconds" INTEGER NOT NULL DEFAULT 0,
  "referralWelcomeFlowId" INTEGER REFERENCES "Flow" ("id") ON DELETE SET NULL ON UPDATE CASCADE,
  "rewardSettings" TEXT NOT NULL CHECK (json_valid("rewardSettings")),
  "rewardsTriggeredAt" INTEGER,
  "rewardRecipients" TEXT CHECK ("rewardRecipients" IS NULL OR json_valid("rewardRecipients")),
  "finishedAt" INTEGER,
  "deletedAt" INTEGER
) STRICT;
CREATE INDEX "Leaderboard_workspaceId_status_idx" ON "Leaderboard" ("workspaceId", "status");
CREATE INDEX "Leaderboard_inboxId_idx" ON "Leaderboard" ("inboxId");
CREATE INDEX "Leaderboard_minigameId_idx" ON "Leaderboard" ("minigameId");

-- LeaderboardParticipant  (leaderboard-participant)
-- "dailyBucket" date (mode string) -> TEXT YYYY-MM-DD. Partial WHERE NOT "isBanned" works as is (0/1).
CREATE TABLE "LeaderboardParticipant" (
  "leaderboardId" INTEGER NOT NULL REFERENCES "Leaderboard" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "contactId" INTEGER NOT NULL REFERENCES "Contact" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "score" INTEGER NOT NULL DEFAULT 0,
  "isBanned" INTEGER NOT NULL DEFAULT 0 CHECK ("isBanned" IN (0, 1)),
  "referrerContactId" INTEGER REFERENCES "Contact" ("id") ON DELETE SET NULL ON UPDATE CASCADE,
  "scoredAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "dailyBucket" TEXT CHECK ("dailyBucket" IS NULL OR "dailyBucket" GLOB '[0-9][0-9][0-9][0-9]-[0-1][0-9]-[0-3][0-9]'),
  "dailyCounts" TEXT NOT NULL CHECK (json_valid("dailyCounts")),
  "breakdown" TEXT NOT NULL CHECK (json_valid("breakdown")),
  "lastActivityAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  CONSTRAINT "LeaderboardParticipant_pkey" PRIMARY KEY ("leaderboardId", "contactId")
) STRICT;
CREATE INDEX "LeaderboardParticipant_ranking_idx" ON "LeaderboardParticipant" ("leaderboardId", "score" DESC, "scoredAt") WHERE NOT "isBanned";
CREATE INDEX "LeaderboardParticipant_leaderboardId_referrerContactId_idx" ON "LeaderboardParticipant" ("leaderboardId", "referrerContactId");
CREATE INDEX "LeaderboardParticipant_contactId_idx" ON "LeaderboardParticipant" ("contactId");
CREATE INDEX "LeaderboardParticipant_workspaceId_idx" ON "LeaderboardParticipant" ("workspaceId");
CREATE INDEX "LeaderboardParticipant_referrerContactId_idx" ON "LeaderboardParticipant" ("referrerContactId") WHERE "referrerContactId" IS NOT NULL;

-- LeaderboardScoreEvent  (leaderboard-score-event)
-- RANGE partitioned monthly by "createdAt" (+ DEFAULT) -> one table. PK ("createdAt", "id") kept: it is the "createdAt"-leading
-- index the maintainLeaderboardPartitions retention DELETE needs (no extra index). No FKs (as Postgres).
CREATE TABLE "LeaderboardScoreEvent" (
  "id" INTEGER NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "leaderboardId" INTEGER NOT NULL,
  "workspaceId" INTEGER NOT NULL,
  "contactId" INTEGER NOT NULL,
  "delta" INTEGER NOT NULL,
  "source" TEXT NOT NULL CHECK ("source" IN ('rule', 'flow', 'referral', 'manual', 'reset')),
  "rule" TEXT,
  "sourceRef" TEXT,
  CONSTRAINT "LeaderboardScoreEvent_pkey" PRIMARY KEY ("createdAt", "id")
) STRICT;
CREATE INDEX "LeaderboardScoreEvent_leaderboardId_createdAt_idx" ON "LeaderboardScoreEvent" ("leaderboardId", "createdAt" DESC);
CREATE INDEX "LeaderboardScoreEvent_leaderboardId_contactId_createdAt_idx" ON "LeaderboardScoreEvent" ("leaderboardId", "contactId", "createdAt" DESC);
CREATE INDEX "LeaderboardScoreEvent_workspaceId_idx" ON "LeaderboardScoreEvent" ("workspaceId");

-- LeaderboardAuditLog  (leaderboard-audit-log)
CREATE TABLE "LeaderboardAuditLog" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "leaderboardId" INTEGER NOT NULL REFERENCES "Leaderboard" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "contactId" INTEGER REFERENCES "Contact" ("id") ON DELETE SET NULL ON UPDATE CASCADE,
  "action" TEXT NOT NULL CHECK ("action" IN ('adjust', 'resetOne', 'resetAll', 'ban', 'unban')),
  "delta" INTEGER NOT NULL DEFAULT 0,
  "actorUserId" INTEGER REFERENCES "User" ("id") ON DELETE SET NULL ON UPDATE CASCADE,
  "reason" TEXT NOT NULL DEFAULT ''
) STRICT;
CREATE INDEX "LeaderboardAuditLog_leaderboardId_createdAt_idx" ON "LeaderboardAuditLog" ("leaderboardId", "createdAt" DESC);
CREATE INDEX "LeaderboardAuditLog_workspaceId_idx" ON "LeaderboardAuditLog" ("workspaceId");

-- Poll  (poll)
CREATE TABLE "Poll" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "name" TEXT NOT NULL,
  "imageUrl" TEXT NOT NULL,
  "startsAt" INTEGER NOT NULL,
  "endsAt" INTEGER NOT NULL,
  "afterVoteFlowId" INTEGER REFERENCES "Flow" ("id") ON DELETE SET NULL ON UPDATE CASCADE
) STRICT;
CREATE INDEX "Poll_workspaceId_createdAt_idx" ON "Poll" ("workspaceId", "createdAt" DESC);
CREATE INDEX "Poll_afterVoteFlowId_idx" ON "Poll" ("afterVoteFlowId");

-- PollCandidate  (poll)
CREATE TABLE "PollCandidate" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "pollId" INTEGER NOT NULL REFERENCES "Poll" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "position" INTEGER NOT NULL,
  "name" TEXT NOT NULL,
  "description" TEXT,
  "imageUrl" TEXT
) STRICT;
CREATE INDEX "PollCandidate_pollId_position_idx" ON "PollCandidate" ("pollId", "position");
CREATE INDEX "PollCandidate_workspaceId_idx" ON "PollCandidate" ("workspaceId");

-- PollVote  (poll)
-- xmax = 0 insert detection -> Rust (P2.2 3.5). FK error names are not reported by SQLite: poll/vote.service CANDIDATE_FOREIGN_KEY must check the candidate row in the tx.
CREATE TABLE "PollVote" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "pollId" INTEGER NOT NULL REFERENCES "Poll" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "candidateId" INTEGER NOT NULL REFERENCES "PollCandidate" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "contactId" INTEGER NOT NULL REFERENCES "Contact" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "contactInboxId" INTEGER REFERENCES "ContactInbox" ("id") ON DELETE SET NULL ON UPDATE CASCADE
) STRICT;
CREATE UNIQUE INDEX "PollVote_pollId_contactId_key" ON "PollVote" ("pollId", "contactId");
CREATE INDEX "PollVote_workspaceId_idx" ON "PollVote" ("workspaceId");
CREATE INDEX "PollVote_candidateId_idx" ON "PollVote" ("candidateId");
CREATE INDEX "PollVote_contactId_idx" ON "PollVote" ("contactId");
CREATE INDEX "PollVote_contactInboxId_idx" ON "PollVote" ("contactInboxId");

-- Questionnaire  (questionnaire)
CREATE TABLE "Questionnaire" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "name" TEXT NOT NULL,
  "enableScore" INTEGER NOT NULL DEFAULT 0 CHECK ("enableScore" IN (0, 1)),
  "enableRetryMessages" INTEGER NOT NULL DEFAULT 0 CHECK ("enableRetryMessages" IN (0, 1)),
  "enableCustomFieldMapping" INTEGER NOT NULL DEFAULT 1 CHECK ("enableCustomFieldMapping" IN (0, 1)),
  "deletedAt" INTEGER,
  "triggerFlowId" INTEGER REFERENCES "Flow" ("id") ON DELETE SET NULL ON UPDATE CASCADE,
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE
) STRICT;
CREATE INDEX "Questionnaire_workspaceId_idx" ON "Questionnaire" ("workspaceId");
CREATE UNIQUE INDEX "Questionnaire_workspaceId_name_key" ON "Questionnaire" ("workspaceId", "name") WHERE ("deletedAt" is null);

-- QuestionnaireQuestion  (questionnaire)
CREATE TABLE "QuestionnaireQuestion" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "questionnaireId" INTEGER NOT NULL REFERENCES "Questionnaire" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "title" TEXT NOT NULL,
  "type" TEXT NOT NULL CHECK ("type" IN ('text', 'number', 'email', 'phone', 'multipleChoice', 'date', 'datetime', 'image', 'file', 'location', 'websiteLink')),
  "active" INTEGER NOT NULL DEFAULT 1 CHECK ("active" IN (0, 1)),
  "image" TEXT CHECK ("image" IS NULL OR json_valid("image")),
  "orderNo" INTEGER NOT NULL DEFAULT 0,
  "point" INTEGER NOT NULL DEFAULT 1,
  "retryMessage" TEXT,
  "customFieldId" INTEGER REFERENCES "CustomField" ("id") ON DELETE SET NULL ON UPDATE CASCADE,
  "systemFieldKey" TEXT,
  "config" TEXT CHECK ("config" IS NULL OR json_valid("config")),
  "deletedAt" INTEGER,
  CONSTRAINT "QuestionnaireQuestion_customFieldId_systemFieldKey_exclusive" CHECK (("customFieldId" IS NULL) OR ("systemFieldKey" IS NULL))
) STRICT;
CREATE INDEX "QuestionnaireQuestion_questionnaireId_orderNo_idx" ON "QuestionnaireQuestion" ("questionnaireId", "orderNo");
CREATE INDEX "QuestionnaireQuestion_customFieldId_idx" ON "QuestionnaireQuestion" ("customFieldId");

-- QuestionnaireSubmission  (questionnaire)
CREATE TABLE "QuestionnaireSubmission" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "questionnaireId" INTEGER NOT NULL REFERENCES "Questionnaire" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "contactId" INTEGER NOT NULL REFERENCES "Contact" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "conversationId" INTEGER REFERENCES "Conversation" ("id") ON DELETE SET NULL ON UPDATE CASCADE,
  "status" TEXT NOT NULL DEFAULT 'inProgress' CHECK ("status" IN ('inProgress', 'completed', 'cancelled', 'failed', 'timeout')),
  "totalPoints" INTEGER,
  "currentQuestionId" INTEGER REFERENCES "QuestionnaireQuestion" ("id") ON DELETE SET NULL ON UPDATE CASCADE,
  "currentQuestionSentAt" INTEGER,
  "lastAnsweredMessageId" INTEGER,
  "startedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "completedAt" INTEGER,
  "cancelledAt" INTEGER
) STRICT;
CREATE INDEX "QuestionnaireSubmission_workspaceId_idx" ON "QuestionnaireSubmission" ("workspaceId");
CREATE INDEX "QuestionnaireSubmission_questionnaireId_status_idx" ON "QuestionnaireSubmission" ("questionnaireId", "status");
CREATE INDEX "QuestionnaireSubmission_questionnaireId_contactId_idx" ON "QuestionnaireSubmission" ("questionnaireId", "contactId");
CREATE INDEX "QuestionnaireSubmission_contactId_idx" ON "QuestionnaireSubmission" ("contactId");
CREATE INDEX "QuestionnaireSubmission_conversationId_idx" ON "QuestionnaireSubmission" ("conversationId") WHERE "conversationId" IS NOT NULL;
CREATE INDEX "QuestionnaireSubmission_currentQuestionId_idx" ON "QuestionnaireSubmission" ("currentQuestionId") WHERE "currentQuestionId" IS NOT NULL;
CREATE INDEX "QuestionnaireSubmission_workspaceId_contactId_completedAt_idx" ON "QuestionnaireSubmission" ("workspaceId", "contactId", "completedAt" DESC);
CREATE UNIQUE INDEX "QuestionnaireSubmission_workspaceId_contactId_active_key" ON "QuestionnaireSubmission" ("workspaceId", "contactId") WHERE "status" = 'inProgress';

-- QuestionnaireAnswer  (questionnaire)
CREATE TABLE "QuestionnaireAnswer" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "submissionId" INTEGER NOT NULL REFERENCES "QuestionnaireSubmission" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "questionId" INTEGER NOT NULL REFERENCES "QuestionnaireQuestion" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "questionIdSnapshot" TEXT NOT NULL,
  "questionTitleSnapshot" TEXT NOT NULL,
  "questionTypeSnapshot" TEXT NOT NULL,
  "labelSnapshot" TEXT NOT NULL,
  "value" TEXT CHECK ("value" IS NULL OR json_valid("value")),
  "pointsEarned" INTEGER,
  "attemptCount" INTEGER NOT NULL DEFAULT 1,
  "answeredAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER))
) STRICT;
CREATE UNIQUE INDEX "QuestionnaireAnswer_submissionId_questionIdSnapshot_key" ON "QuestionnaireAnswer" ("submissionId", "questionIdSnapshot");
CREATE INDEX "QuestionnaireAnswer_questionId_idx" ON "QuestionnaireAnswer" ("questionId");

-- MiniApp  (mini-app)
CREATE TABLE "MiniApp" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "name" TEXT NOT NULL,
  "enabled" INTEGER NOT NULL DEFAULT 1 CHECK ("enabled" IN (0, 1)),
  "definition" TEXT NOT NULL CHECK (json_valid("definition")),
  "flowJson" TEXT NOT NULL CHECK (json_valid("flowJson")),
  "submissionsCount" INTEGER NOT NULL DEFAULT 0
) STRICT;
CREATE INDEX "MiniApp_workspaceId_idx" ON "MiniApp" ("workspaceId");
CREATE UNIQUE INDEX "MiniApp_workspaceId_name_key" ON "MiniApp" ("workspaceId", "name");

-- MiniAppPublication  (mini-app-publication)
CREATE TABLE "MiniAppPublication" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "miniAppId" INTEGER NOT NULL REFERENCES "MiniApp" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "integrationWhatsappId" INTEGER NOT NULL REFERENCES "IntegrationWhatsapp" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "whatsappFlowId" INTEGER REFERENCES "WhatsappFlow" ("id") ON DELETE SET NULL ON UPDATE CASCADE,
  "sourceId" TEXT NOT NULL,
  "status" TEXT NOT NULL,
  "validationErrors" TEXT NOT NULL CHECK (json_valid("validationErrors")),
  "publishedAt" INTEGER
) STRICT;
CREATE UNIQUE INDEX "MiniAppPublication_miniAppId_integrationWhatsappId_key" ON "MiniAppPublication" ("miniAppId", "integrationWhatsappId");

-- MiniAppSubmission  (mini-app-submission)
CREATE TABLE "MiniAppSubmission" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "miniAppId" INTEGER NOT NULL REFERENCES "MiniApp" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "contactId" INTEGER REFERENCES "Contact" ("id") ON DELETE SET NULL ON UPDATE CASCADE,
  "source" TEXT NOT NULL CHECK ("source" IN ('web')),
  "answers" TEXT NOT NULL CHECK (json_valid("answers"))
) STRICT;
CREATE INDEX "MiniAppSubmission_miniAppId_createdAt_idx" ON "MiniAppSubmission" ("miniAppId", "createdAt" DESC);
CREATE INDEX "MiniAppSubmission_contactId_idx" ON "MiniAppSubmission" ("contactId");

-- MiniAppUpload  (mini-app-upload)
CREATE TABLE "MiniAppUpload" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "miniAppId" INTEGER NOT NULL REFERENCES "MiniApp" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "contactId" INTEGER REFERENCES "Contact" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "submissionId" INTEGER REFERENCES "MiniAppSubmission" ("id") ON DELETE SET NULL ON UPDATE CASCADE,
  "inputName" TEXT NOT NULL,
  "uploadToken" TEXT NOT NULL,
  "path" TEXT NOT NULL,
  "fileName" TEXT NOT NULL,
  "mimeType" TEXT NOT NULL,
  "size" INTEGER NOT NULL,
  "status" TEXT NOT NULL CHECK ("status" IN ('pending', 'submitted'))
) STRICT;
CREATE UNIQUE INDEX "MiniAppUpload_uploadToken_key" ON "MiniAppUpload" ("uploadToken");
CREATE UNIQUE INDEX "MiniAppUpload_path_key" ON "MiniAppUpload" ("path");
CREATE INDEX "MiniAppUpload_miniAppId_inputName_status_idx" ON "MiniAppUpload" ("miniAppId", "inputName", "status");
CREATE INDEX "MiniAppUpload_submissionId_idx" ON "MiniAppUpload" ("submissionId");
CREATE INDEX "MiniAppUpload_contactId_idx" ON "MiniAppUpload" ("contactId") WHERE "contactId" IS NOT NULL;

-- DynamicImage  (dynamic-image)
CREATE TABLE "DynamicImage" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "name" TEXT NOT NULL,
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "customFieldId" INTEGER REFERENCES "CustomField" ("id") ON DELETE SET NULL ON UPDATE CASCADE,
  "data" TEXT NOT NULL CHECK (json_valid("data")),
  "backgroundUrl" TEXT,
  "enabled" INTEGER NOT NULL DEFAULT 1 CHECK ("enabled" IN (0, 1))
) STRICT;
CREATE UNIQUE INDEX "DynamicImage_workspaceId_name_key" ON "DynamicImage" ("workspaceId", "name");

-- ======================== Appointments and calendars ========================

-- AppointmentCalendar  (appointment-calendar)
CREATE TABLE "AppointmentCalendar" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "name" TEXT NOT NULL,
  "description" TEXT,
  "active" INTEGER NOT NULL DEFAULT 0 CHECK ("active" IN (0, 1)),
  "timezone" TEXT NOT NULL,
  "durationMinutes" INTEGER NOT NULL DEFAULT 30,
  "bufferAfterMinutes" INTEGER,
  "locationType" TEXT NOT NULL CHECK ("locationType" IN ('inPerson', 'phoneCall', 'onlineMeeting')),
  "locationDetail" TEXT,
  "scheduleWindowType" TEXT NOT NULL DEFAULT 'rollingDays' CHECK ("scheduleWindowType" IN ('rollingDays', 'dateRange', 'specificDay', 'anyFutureDate')),
  "scheduleWindowConfig" TEXT NOT NULL DEFAULT '{}' CHECK (json_valid("scheduleWindowConfig")),
  "maxAppointmentsPerUser" INTEGER,
  "dailyLimitEnabled" INTEGER NOT NULL DEFAULT 0 CHECK ("dailyLimitEnabled" IN (0, 1)),
  "maxPerDay" INTEGER,
  "allowGroupMeeting" INTEGER NOT NULL DEFAULT 0 CHECK ("allowGroupMeeting" IN (0, 1)),
  "maxPerSlot" INTEGER,
  "confirmationMessage" TEXT,
  "confirmationFlowId" INTEGER REFERENCES "Flow" ("id") ON DELETE SET NULL ON UPDATE CASCADE,
  "cancellationFlowId" INTEGER REFERENCES "Flow" ("id") ON DELETE SET NULL ON UPDATE CASCADE,
  "externalConnectionId" INTEGER REFERENCES "Integration" ("id") ON DELETE SET NULL ON UPDATE CASCADE,
  "externalEventTitleTemplate" TEXT,
  "externalEventDescriptionTemplate" TEXT,
  "externalEventAttendeesTemplate" TEXT,
  "publicLinkSlug" TEXT NOT NULL,
  "deletedAt" INTEGER
) STRICT;
CREATE INDEX "AppointmentCalendar_workspaceId_idx" ON "AppointmentCalendar" ("workspaceId");
CREATE UNIQUE INDEX "AppointmentCalendar_workspaceId_name_key" ON "AppointmentCalendar" ("workspaceId", "name") WHERE ("deletedAt" is null);
CREATE UNIQUE INDEX "AppointmentCalendar_publicLinkSlug_key" ON "AppointmentCalendar" ("publicLinkSlug");

-- AppointmentCalendarAvailability  (appointment-calendar-availability)
CREATE TABLE "AppointmentCalendarAvailability" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "calendarId" INTEGER NOT NULL REFERENCES "AppointmentCalendar" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "weekday" INTEGER NOT NULL,
  "startMinute" INTEGER NOT NULL,
  "endMinute" INTEGER NOT NULL
) STRICT;
CREATE INDEX "AppointmentCalendarAvailability_calendarId_idx" ON "AppointmentCalendarAvailability" ("calendarId");

-- AppointmentCalendarReminder  (appointment-calendar-reminder)
CREATE TABLE "AppointmentCalendarReminder" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "calendarId" INTEGER NOT NULL REFERENCES "AppointmentCalendar" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "flowId" INTEGER NOT NULL REFERENCES "Flow" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "timingValue" INTEGER NOT NULL,
  "timingUnit" TEXT NOT NULL CHECK ("timingUnit" IN ('minutes', 'hours', 'days'))
) STRICT;
CREATE UNIQUE INDEX "AppointmentCalendarReminder_dedupe_key" ON "AppointmentCalendarReminder" ("calendarId", "flowId", "timingValue", "timingUnit");

-- Appointment  (appointment)
CREATE TABLE "Appointment" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "calendarId" INTEGER NOT NULL REFERENCES "AppointmentCalendar" ("id") ON DELETE RESTRICT ON UPDATE CASCADE,
  "contactId" INTEGER NOT NULL REFERENCES "Contact" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "conversationId" INTEGER REFERENCES "Conversation" ("id") ON DELETE SET NULL ON UPDATE CASCADE,
  "contactInboxId" INTEGER REFERENCES "ContactInbox" ("id") ON DELETE SET NULL ON UPDATE CASCADE,
  "startAt" INTEGER NOT NULL,
  "endAt" INTEGER NOT NULL,
  "inviteeTimezone" TEXT NOT NULL,
  "status" TEXT NOT NULL DEFAULT 'scheduled' CHECK ("status" IN ('scheduled', 'cancelled')),
  "locationType" TEXT NOT NULL CHECK ("locationType" IN ('inPerson', 'phoneCall', 'onlineMeeting')),
  "locationDetail" TEXT,
  "externalEventId" TEXT,
  "externalEventIntegrationId" INTEGER,
  "externalEventProviderCalendarId" TEXT,
  "externalSyncStatus" TEXT CHECK ("externalSyncStatus" IN ('pending', 'synced', 'failed')),
  "cancelledAt" INTEGER,
  "deletedAt" INTEGER
) STRICT;
CREATE INDEX "Appointment_workspaceId_idx" ON "Appointment" ("workspaceId");
CREATE INDEX "Appointment_workspaceId_startAt_idx" ON "Appointment" ("workspaceId", "startAt");
CREATE INDEX "Appointment_calendarId_status_startAt_idx" ON "Appointment" ("calendarId", "status", "startAt");
CREATE INDEX "Appointment_contactId_calendarId_status_idx" ON "Appointment" ("contactId", "calendarId", "status");

-- Part-2 column with a FK to "Appointment" (P2.4 spec 1.5). ADD COLUMN ... REFERENCES is allowed: the default is NULL.
-- It becomes the last column of "ContactOnSmartDelay" (snapshot position: after "metadata"; Rust names columns).
ALTER TABLE "ContactOnSmartDelay" ADD COLUMN "appointmentId" INTEGER REFERENCES "Appointment" ("id") ON DELETE SET NULL ON UPDATE CASCADE;
CREATE INDEX "ContactOnSmartDelay_appointmentId_idx" ON "ContactOnSmartDelay" ("appointmentId");

-- AppointmentReminderDispatch  (appointment-reminder-dispatch)
CREATE TABLE "AppointmentReminderDispatch" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "appointmentId" INTEGER NOT NULL REFERENCES "Appointment" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "reminderConfigId" INTEGER NOT NULL REFERENCES "AppointmentCalendarReminder" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "contactInboxId" INTEGER REFERENCES "ContactInbox" ("id") ON DELETE SET NULL ON UPDATE CASCADE,
  "sendAt" INTEGER NOT NULL,
  "status" TEXT NOT NULL DEFAULT 'pending' CHECK ("status" IN ('pending', 'sent', 'cancelled', 'failed')),
  "jobId" TEXT NOT NULL,
  "sentAt" INTEGER,
  "cancelledAt" INTEGER,
  "failedReason" TEXT
) STRICT;
CREATE UNIQUE INDEX "AppointmentReminderDispatch_jobId_key" ON "AppointmentReminderDispatch" ("jobId");
CREATE INDEX "AppointmentReminderDispatch_status_sendAt_idx" ON "AppointmentReminderDispatch" ("status", "sendAt");
CREATE INDEX "AppointmentReminderDispatch_appointmentId_idx" ON "AppointmentReminderDispatch" ("appointmentId");
CREATE INDEX "AppointmentReminderDispatch_contactInboxId_idx" ON "AppointmentReminderDispatch" ("contactInboxId");

-- IntegrationGoogleCalendar  (integration-google-calendar)
CREATE TABLE "IntegrationGoogleCalendar" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "integrationId" INTEGER NOT NULL REFERENCES "Integration" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "auth" TEXT NOT NULL CHECK (json_valid("auth")),
  "providerCalendarId" TEXT NOT NULL DEFAULT 'primary',
  "email" TEXT
) STRICT;
CREATE UNIQUE INDEX "IntegrationGoogleCalendar_integrationId_key" ON "IntegrationGoogleCalendar" ("integrationId");

-- IntegrationOutlookCalendar  (integration-outlook-calendar)
CREATE TABLE "IntegrationOutlookCalendar" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "integrationId" INTEGER NOT NULL REFERENCES "Integration" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "auth" TEXT NOT NULL CHECK (json_valid("auth")),
  "providerCalendarId" TEXT NOT NULL DEFAULT 'primary',
  "email" TEXT
) STRICT;
CREATE UNIQUE INDEX "IntegrationOutlookCalendar_integrationId_key" ON "IntegrationOutlookCalendar" ("integrationId");

-- ======================== Pipelines (CRM) ========================

-- Pipeline  (pipeline)
CREATE TABLE "Pipeline" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "name" TEXT NOT NULL,
  "type" TEXT NOT NULL CHECK ("type" IN ('opportunity', 'ticket', 'custom')),
  "referenceName" TEXT NOT NULL,
  "currency" TEXT,
  "settings" TEXT NOT NULL CHECK (json_valid("settings")),
  "createdById" INTEGER REFERENCES "User" ("id") ON DELETE SET NULL ON UPDATE CASCADE
) STRICT;
CREATE UNIQUE INDEX "Pipeline_workspaceId_referenceName_key" ON "Pipeline" ("workspaceId", "referenceName");
CREATE UNIQUE INDEX "Pipeline_workspaceId_name_key" ON "Pipeline" ("workspaceId", "name");
CREATE INDEX "Pipeline_createdById_idx" ON "Pipeline" ("createdById") WHERE "createdById" IS NOT NULL;

-- PipelineStage  (pipeline)
CREATE TABLE "PipelineStage" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "pipelineId" INTEGER NOT NULL REFERENCES "Pipeline" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "name" TEXT NOT NULL,
  "description" TEXT,
  "textColor" TEXT,
  "backgroundColor" TEXT,
  "position" INTEGER NOT NULL,
  "bulkJobId" TEXT
) STRICT;
CREATE INDEX "PipelineStage_pipelineId_position_idx" ON "PipelineStage" ("pipelineId", "position");
CREATE INDEX "PipelineStage_workspaceId_idx" ON "PipelineStage" ("workspaceId");

-- PipelineField  (pipeline)
CREATE TABLE "PipelineField" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "pipelineId" INTEGER NOT NULL REFERENCES "Pipeline" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "name" TEXT NOT NULL,
  "referenceName" TEXT NOT NULL,
  "type" TEXT NOT NULL CHECK ("type" IN ('shortText', 'email', 'phoneNumber', 'number', 'date', 'datetime', 'boolean', 'longText')),
  "required" INTEGER NOT NULL DEFAULT 0 CHECK ("required" IN (0, 1)),
  "position" INTEGER NOT NULL
) STRICT;
CREATE UNIQUE INDEX "PipelineField_pipelineId_referenceName_key" ON "PipelineField" ("pipelineId", "referenceName");
CREATE INDEX "PipelineField_workspaceId_idx" ON "PipelineField" ("workspaceId");

-- PipelineMember  (pipeline)
CREATE TABLE "PipelineMember" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "pipelineId" INTEGER NOT NULL REFERENCES "Pipeline" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "workspaceMemberId" INTEGER NOT NULL REFERENCES "WorkspaceMember" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "canManageSettings" INTEGER NOT NULL DEFAULT 0 CHECK ("canManageSettings" IN (0, 1)),
  "canView" INTEGER NOT NULL DEFAULT 1 CHECK ("canView" IN (0, 1)),
  "canEdit" INTEGER NOT NULL DEFAULT 0 CHECK ("canEdit" IN (0, 1)),
  "canDelete" INTEGER NOT NULL DEFAULT 0 CHECK ("canDelete" IN (0, 1))
) STRICT;
CREATE UNIQUE INDEX "PipelineMember_pipeline_member_key" ON "PipelineMember" ("pipelineId", "workspaceMemberId");
CREATE INDEX "PipelineMember_member_idx" ON "PipelineMember" ("workspaceId", "workspaceMemberId");
CREATE INDEX "PipelineMember_workspaceMemberId_idx" ON "PipelineMember" ("workspaceMemberId");

-- PipelineItemCounter  (pipeline)
CREATE TABLE "PipelineItemCounter" (
  "workspaceId" INTEGER PRIMARY KEY NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "lastNumber" INTEGER NOT NULL DEFAULT 0
) STRICT;

-- PipelineItem  (pipeline)
-- "value" money -> INTEGER x 10,000. "number" from "PipelineItemCounter".
CREATE TABLE "PipelineItem" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "pipelineId" INTEGER NOT NULL REFERENCES "Pipeline" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "stageId" INTEGER NOT NULL REFERENCES "PipelineStage" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "contactId" INTEGER NOT NULL REFERENCES "Contact" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "number" INTEGER NOT NULL,
  "title" TEXT NOT NULL,
  "description" TEXT,
  "value" INTEGER,
  "status" TEXT NOT NULL CHECK ("status" IN ('open', 'won', 'lost')),
  "priority" TEXT NOT NULL CHECK ("priority" IN ('low', 'medium', 'high')),
  "dueDate" INTEGER,
  "fieldValues" TEXT NOT NULL CHECK (json_valid("fieldValues")),
  "createdById" INTEGER REFERENCES "User" ("id") ON DELETE SET NULL ON UPDATE CASCADE,
  "closedById" INTEGER REFERENCES "User" ("id") ON DELETE SET NULL ON UPDATE CASCADE,
  "stageEnteredAt" INTEGER NOT NULL,
  "statusChangedAt" INTEGER,
  "closedAt" INTEGER,
  "lastCommentAt" INTEGER,
  "idempotencyKey" TEXT
) STRICT;
CREATE UNIQUE INDEX "PipelineItem_workspaceId_number_key" ON "PipelineItem" ("workspaceId", "number");
CREATE UNIQUE INDEX "PipelineItem_pipeline_idempotencyKey_key" ON "PipelineItem" ("workspaceId", "pipelineId", "idempotencyKey") WHERE "idempotencyKey" IS NOT NULL;
CREATE INDEX "PipelineItem_contact_status_idx" ON "PipelineItem" ("workspaceId", "pipelineId", "contactId", "status", "createdAt" DESC);
CREATE INDEX "PipelineItem_stage_createdAt_idx" ON "PipelineItem" ("pipelineId", "stageId", "createdAt" DESC, "id" DESC);
CREATE INDEX "PipelineItem_pipeline_createdAt_idx" ON "PipelineItem" ("pipelineId", "createdAt" DESC, "id" DESC);
CREATE INDEX "PipelineItem_pipeline_id_idx" ON "PipelineItem" ("pipelineId", "id");
CREATE INDEX "PipelineItem_contactId_idx" ON "PipelineItem" ("contactId");
CREATE INDEX "PipelineItem_stageId_idx" ON "PipelineItem" ("stageId");
CREATE INDEX "PipelineItem_createdById_idx" ON "PipelineItem" ("createdById") WHERE "createdById" IS NOT NULL;
CREATE INDEX "PipelineItem_closedById_idx" ON "PipelineItem" ("closedById") WHERE "closedById" IS NOT NULL;

-- PipelineItemAssignee  (pipeline)
CREATE TABLE "PipelineItemAssignee" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "itemId" INTEGER NOT NULL REFERENCES "PipelineItem" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "workspaceMemberId" INTEGER REFERENCES "WorkspaceMember" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "inboxTeamId" INTEGER REFERENCES "InboxTeam" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  CONSTRAINT "PipelineItemAssignee_one_target_check" CHECK (("workspaceMemberId" IS NULL) <> ("inboxTeamId" IS NULL))
) STRICT;
CREATE UNIQUE INDEX "PipelineItemAssignee_item_member_key" ON "PipelineItemAssignee" ("itemId", "workspaceMemberId");
CREATE UNIQUE INDEX "PipelineItemAssignee_item_team_key" ON "PipelineItemAssignee" ("itemId", "inboxTeamId");
CREATE INDEX "PipelineItemAssignee_member_idx" ON "PipelineItemAssignee" ("workspaceId", "workspaceMemberId");
CREATE INDEX "PipelineItemAssignee_team_idx" ON "PipelineItemAssignee" ("workspaceId", "inboxTeamId");
CREATE INDEX "PipelineItemAssignee_workspaceMemberId_idx" ON "PipelineItemAssignee" ("workspaceMemberId") WHERE "workspaceMemberId" IS NOT NULL;
CREATE INDEX "PipelineItemAssignee_inboxTeamId_idx" ON "PipelineItemAssignee" ("inboxTeamId") WHERE "inboxTeamId" IS NOT NULL;

-- PipelineItemComment  (pipeline)
CREATE TABLE "PipelineItemComment" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "itemId" INTEGER NOT NULL REFERENCES "PipelineItem" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "text" TEXT NOT NULL,
  "createdById" INTEGER REFERENCES "User" ("id") ON DELETE SET NULL ON UPDATE CASCADE,
  "idempotencyKey" TEXT
) STRICT;
CREATE INDEX "PipelineItemComment_item_createdAt_idx" ON "PipelineItemComment" ("itemId", "createdAt" DESC);
CREATE INDEX "PipelineItemComment_createdById_idx" ON "PipelineItemComment" ("createdById") WHERE "createdById" IS NOT NULL;
CREATE UNIQUE INDEX "PipelineItemComment_item_idempotencyKey_key" ON "PipelineItemComment" ("workspaceId", "itemId", "idempotencyKey") WHERE "idempotencyKey" IS NOT NULL;

-- PipelineItemAttachment  (pipeline)
CREATE TABLE "PipelineItemAttachment" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "itemId" INTEGER NOT NULL REFERENCES "PipelineItem" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "mediaLibraryFileId" INTEGER NOT NULL REFERENCES "MediaLibraryFile" ("id") ON DELETE CASCADE ON UPDATE CASCADE
) STRICT;
CREATE UNIQUE INDEX "PipelineItemAttachment_item_file_key" ON "PipelineItemAttachment" ("itemId", "mediaLibraryFileId");
CREATE INDEX "PipelineItemAttachment_mediaLibraryFileId_idx" ON "PipelineItemAttachment" ("mediaLibraryFileId");
CREATE INDEX "PipelineItemAttachment_workspaceId_idx" ON "PipelineItemAttachment" ("workspaceId");

-- PipelineItemActivity  (pipeline)
-- HASH(workspaceId) x 32 -> one table; PK ("id", "workspaceId") -> "id" INTEGER PRIMARY KEY (P2.2 3.3; no FK points to it).
CREATE TABLE "PipelineItemActivity" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "pipelineId" INTEGER NOT NULL REFERENCES "Pipeline" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "itemId" INTEGER NOT NULL,
  "itemNumber" INTEGER NOT NULL,
  "itemTitle" TEXT NOT NULL,
  "contactId" INTEGER NOT NULL,
  "kind" TEXT NOT NULL CHECK ("kind" IN ('created', 'stageChanged', 'statusChanged', 'valueChanged', 'priorityChanged', 'dueDateChanged', 'titleChanged', 'descriptionChanged', 'fieldValuesChanged', 'assigneesChanged', 'attachmentAdded', 'attachmentRemoved', 'commentAdded', 'commentDeleted', 'transferred', 'deleted')),
  "actorKind" TEXT NOT NULL CHECK ("actorKind" IN ('user', 'workspaceToken', 'flow', 'trigger', 'system')),
  "actorUserId" INTEGER REFERENCES "User" ("id") ON DELETE SET NULL ON UPDATE CASCADE,
  "actorSourceId" TEXT,
  "fromStageId" INTEGER,
  "toStageId" INTEGER,
  "fromStatus" TEXT CHECK ("fromStatus" IN ('open', 'won', 'lost')),
  "toStatus" TEXT CHECK ("toStatus" IN ('open', 'won', 'lost')),
  "fromValue" INTEGER,
  "toValue" INTEGER,
  "fromPriority" TEXT CHECK ("fromPriority" IN ('low', 'medium', 'high')),
  "toPriority" TEXT CHECK ("toPriority" IN ('low', 'medium', 'high')),
  "fromDueDate" INTEGER,
  "toDueDate" INTEGER,
  "fromTitle" TEXT,
  "toTitle" TEXT,
  "fromPipelineId" INTEGER,
  "version" INTEGER NOT NULL DEFAULT 1,
  "occurredAt" INTEGER NOT NULL
) STRICT;
CREATE INDEX "PipelineItemActivity_pipeline_kind_occurredAt_idx" ON "PipelineItemActivity" ("workspaceId", "pipelineId", "kind", "occurredAt");
CREATE INDEX "PipelineItemActivity_item_occurredAt_idx" ON "PipelineItemActivity" ("workspaceId", "itemId", "occurredAt" DESC);
CREATE INDEX "PipelineItemActivity_contact_idx" ON "PipelineItemActivity" ("workspaceId", "contactId");
CREATE INDEX "PipelineItemActivity_pipelineId_idx" ON "PipelineItemActivity" ("pipelineId");
CREATE INDEX "PipelineItemActivity_actorUserId_idx" ON "PipelineItemActivity" ("actorUserId") WHERE "actorUserId" IS NOT NULL;

-- ======================== Email and marketing integrations ========================

-- EmailTopic  (email-topic)
CREATE TABLE "EmailTopic" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "name" TEXT NOT NULL,
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "folderId" INTEGER REFERENCES "Folder" ("id") ON DELETE SET NULL ON UPDATE CASCADE,
  "sendsTotal" INTEGER NOT NULL DEFAULT 0,
  "deliveredsTotal" INTEGER NOT NULL DEFAULT 0,
  "seensTotal" INTEGER NOT NULL DEFAULT 0,
  "clicksTotal" INTEGER NOT NULL DEFAULT 0
) STRICT;
CREATE UNIQUE INDEX "EmailTopic_workspaceId_name_key" ON "EmailTopic" ("workspaceId", "name");
CREATE INDEX "EmailTopic_folderId_idx" ON "EmailTopic" ("folderId");

-- IntegrationActiveCampaign  (integration-active-campaign)
CREATE TABLE "IntegrationActiveCampaign" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "integrationId" INTEGER NOT NULL REFERENCES "Integration" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "auth" TEXT NOT NULL CHECK (json_valid("auth"))
) STRICT;
CREATE UNIQUE INDEX "IntegrationActiveCampaign_integrationId_key" ON "IntegrationActiveCampaign" ("integrationId");
CREATE UNIQUE INDEX "IntegrationActiveCampaign_workspaceId_key" ON "IntegrationActiveCampaign" ("workspaceId");

-- IntegrationDrip  (integration-drip)
CREATE TABLE "IntegrationDrip" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "integrationId" INTEGER NOT NULL REFERENCES "Integration" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "auth" TEXT NOT NULL CHECK (json_valid("auth"))
) STRICT;
CREATE UNIQUE INDEX "IntegrationDrip_integrationId_key" ON "IntegrationDrip" ("integrationId");
CREATE UNIQUE INDEX "IntegrationDrip_workspaceId_key" ON "IntegrationDrip" ("workspaceId");

-- IntegrationGetResponse  (integration-get-response)
CREATE TABLE "IntegrationGetResponse" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "integrationId" INTEGER NOT NULL REFERENCES "Integration" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "auth" TEXT NOT NULL CHECK (json_valid("auth"))
) STRICT;
CREATE UNIQUE INDEX "IntegrationGetResponse_integrationId_key" ON "IntegrationGetResponse" ("integrationId");
CREATE UNIQUE INDEX "IntegrationGetResponse_workspaceId_key" ON "IntegrationGetResponse" ("workspaceId");

-- IntegrationKlaviyo  (integration-klaviyo)
CREATE TABLE "IntegrationKlaviyo" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "integrationId" INTEGER NOT NULL REFERENCES "Integration" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "auth" TEXT NOT NULL CHECK (json_valid("auth"))
) STRICT;
CREATE UNIQUE INDEX "IntegrationKlaviyo_workspaceId_key" ON "IntegrationKlaviyo" ("workspaceId");
CREATE UNIQUE INDEX "IntegrationKlaviyo_integrationId_key" ON "IntegrationKlaviyo" ("integrationId");

-- IntegrationMailchimp  (integration-mailchimp)
CREATE TABLE "IntegrationMailchimp" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "integrationId" INTEGER NOT NULL REFERENCES "Integration" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "auth" TEXT NOT NULL CHECK (json_valid("auth"))
) STRICT;
CREATE UNIQUE INDEX "IntegrationMailchimp_integrationId_key" ON "IntegrationMailchimp" ("integrationId");
CREATE INDEX "IntegrationMailchimp_workspaceId_idx" ON "IntegrationMailchimp" ("workspaceId");

-- IntegrationMailerLite  (integration-mailer-lite)
CREATE TABLE "IntegrationMailerLite" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "integrationId" INTEGER NOT NULL REFERENCES "Integration" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "auth" TEXT NOT NULL CHECK (json_valid("auth"))
) STRICT;
CREATE UNIQUE INDEX "IntegrationMailerLite_workspaceId_key" ON "IntegrationMailerLite" ("workspaceId");
CREATE UNIQUE INDEX "IntegrationMailerLite_integrationId_key" ON "IntegrationMailerLite" ("integrationId");

-- IntegrationMoosend  (integration-moosend)
CREATE TABLE "IntegrationMoosend" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "integrationId" INTEGER NOT NULL REFERENCES "Integration" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "auth" TEXT NOT NULL CHECK (json_valid("auth"))
) STRICT;
CREATE UNIQUE INDEX "IntegrationMoosend_workspaceId_key" ON "IntegrationMoosend" ("workspaceId");
CREATE UNIQUE INDEX "IntegrationMoosend_integrationId_key" ON "IntegrationMoosend" ("integrationId");

-- IntegrationSendGrid  (integration-sendgrid)
CREATE TABLE "IntegrationSendGrid" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "integrationId" INTEGER NOT NULL REFERENCES "Integration" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "auth" TEXT NOT NULL CHECK (json_valid("auth"))
) STRICT;
CREATE UNIQUE INDEX "IntegrationSendGrid_integrationId_key" ON "IntegrationSendGrid" ("integrationId");
CREATE UNIQUE INDEX "IntegrationSendGrid_workspaceId_key" ON "IntegrationSendGrid" ("workspaceId");

-- IntegrationGoogleSheet  (integration-google-sheets)
CREATE TABLE "IntegrationGoogleSheet" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "integrationId" INTEGER NOT NULL REFERENCES "Integration" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "auth" TEXT NOT NULL CHECK (json_valid("auth"))
) STRICT;
CREATE UNIQUE INDEX "IntegrationGoogleSheet_integrationId_key" ON "IntegrationGoogleSheet" ("integrationId");

-- Spreadsheet  (spreadsheet)
CREATE TABLE "Spreadsheet" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "name" TEXT NOT NULL,
  "url" TEXT NOT NULL,
  "spreadsheetId" TEXT NOT NULL
) STRICT;
CREATE INDEX "Spreadsheet_workspaceId_idx" ON "Spreadsheet" ("workspaceId");
CREATE INDEX "Spreadsheet_workspaceId_spreadsheetId_idx" ON "Spreadsheet" ("workspaceId", "spreadsheetId");
CREATE INDEX "Spreadsheet_spreadsheetId_idx" ON "Spreadsheet" ("spreadsheetId");

-- ======================== Files, places, logs, other ========================

-- File  (file)
CREATE TABLE "File" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "workspaceId" INTEGER REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "userId" INTEGER REFERENCES "User" ("id") ON DELETE SET NULL ON UPDATE CASCADE,
  "contextType" TEXT NOT NULL CHECK ("contextType" IN ('import', 'generic', 'export')),
  "subType" TEXT,
  "path" TEXT NOT NULL,
  "fileName" TEXT NOT NULL,
  "mimeType" TEXT NOT NULL,
  "fileSize" INTEGER,
  "status" TEXT NOT NULL DEFAULT 'pending' CHECK ("status" IN ('pending', 'uploaded', 'failed')),
  "meta" TEXT CHECK ("meta" IS NULL OR json_valid("meta")),
  "uploadedAt" INTEGER
) STRICT;
CREATE UNIQUE INDEX "File_path_key" ON "File" ("path");
CREATE INDEX "File_workspaceId_contextType_idx" ON "File" ("workspaceId", "contextType");
CREATE INDEX "File_workspaceId_subType_idx" ON "File" ("workspaceId", "subType");
CREATE INDEX "File_export_createdAt_idx" ON "File" ("createdAt") WHERE "contextType" = 'export';

-- Import  (import)
CREATE TABLE "Import" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "inboxId" INTEGER REFERENCES "Inbox" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "userId" INTEGER REFERENCES "User" ("id") ON DELETE SET NULL ON UPDATE CASCADE,
  "fileId" INTEGER NOT NULL REFERENCES "File" ("id") ON DELETE RESTRICT ON UPDATE CASCADE,
  "type" TEXT NOT NULL CHECK ("type" IN ('contacts', 'coupons', 'products', 'flow')),
  "format" TEXT NOT NULL CHECK ("format" IN ('csv', 'xlsx', 'xls', 'json')),
  "status" TEXT NOT NULL CHECK ("status" IN ('pending', 'processing', 'completed', 'failed')),
  "meta" TEXT NOT NULL CHECK (json_valid("meta")),
  "totalCount" INTEGER NOT NULL DEFAULT 0,
  "processedCount" INTEGER NOT NULL DEFAULT 0,
  "successCount" INTEGER NOT NULL DEFAULT 0,
  "failedCount" INTEGER NOT NULL DEFAULT 0,
  "errorMessage" TEXT,
  "errorSample" TEXT NOT NULL DEFAULT '[]' CHECK (json_valid("errorSample")),
  "completedAt" INTEGER
) STRICT;
CREATE INDEX "Import_workspaceId_status_idx" ON "Import" ("workspaceId", "status");
CREATE INDEX "Import_workspaceId_type_idx" ON "Import" ("workspaceId", "type");
CREATE INDEX "Import_inboxId_type_idx" ON "Import" ("inboxId", "type");
CREATE INDEX "Import_fileId_idx" ON "Import" ("fileId");
CREATE UNIQUE INDEX "Import_products_active_idx" ON "Import" ("workspaceId") WHERE "type" = 'products' AND "status" IN ('pending', 'processing');

-- ErrorLog  (error-log)
-- Retention: existing cron purgeErrorLogs (0 3 * * *).
CREATE TABLE "ErrorLog" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "action" TEXT NOT NULL,
  "detail" TEXT NOT NULL,
  "httpCode" TEXT,
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "contactId" INTEGER REFERENCES "Contact" ("id") ON DELETE SET NULL ON UPDATE CASCADE,
  "sourceId" TEXT,
  "stackTrace" TEXT
) STRICT;
CREATE INDEX "ErrorLog_workspaceId_createdAt_idx" ON "ErrorLog" ("workspaceId", "createdAt" DESC);
CREATE INDEX "ErrorLog_createdAt_idx" ON "ErrorLog" ("createdAt");
CREATE INDEX "ErrorLog_contactId_idx" ON "ErrorLog" ("contactId");

-- ResourceDeletionLog  (resource-deletion-log)
-- Primary key only (no FK, no index): outlives its workspace (frozen workspace-purge-coverage :224).
CREATE TABLE "ResourceDeletionLog" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "workspaceId" INTEGER NOT NULL,
  "resourceType" TEXT NOT NULL CHECK ("resourceType" IN ('workspace', 'channel')),
  "resourceId" TEXT NOT NULL,
  "resourceSubtype" TEXT,
  "resourceName" TEXT,
  "status" TEXT NOT NULL CHECK ("status" IN ('pending', 'completed', 'canceled')),
  "requestedByUserId" INTEGER,
  "requestSource" TEXT,
  "ipAddress" TEXT,
  "userAgent" TEXT,
  "requestedAt" INTEGER NOT NULL,
  "completedAt" INTEGER,
  "canceledAt" INTEGER
) STRICT;

-- PlaceCategory  (place-near-me)
-- Unique on lower("name") -> lower_u("name") (app function).
CREATE TABLE "PlaceCategory" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "name" TEXT NOT NULL,
  "rank" INTEGER NOT NULL DEFAULT 10
) STRICT;
CREATE INDEX "PlaceCategory_workspaceId_idx" ON "PlaceCategory" ("workspaceId");
CREATE UNIQUE INDEX "PlaceCategory_workspaceId_name_key" ON "PlaceCategory" ("workspaceId", lower_u("name"));

-- PlaceLocation  (place-near-me)
-- latitude/longitude double precision -> REAL; range CHECKs copied.
CREATE TABLE "PlaceLocation" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "name" TEXT NOT NULL,
  "latitude" REAL NOT NULL,
  "longitude" REAL NOT NULL,
  "categoryId" INTEGER REFERENCES "PlaceCategory" ("id") ON DELETE SET NULL ON UPDATE CASCADE,
  CONSTRAINT "PlaceLocation_latitude_check" CHECK ("latitude" BETWEEN -90 AND 90),
  CONSTRAINT "PlaceLocation_longitude_check" CHECK ("longitude" BETWEEN -180 AND 180)
) STRICT;
CREATE INDEX "PlaceLocation_workspaceId_createdAt_idx" ON "PlaceLocation" ("workspaceId", "createdAt");
CREATE INDEX "PlaceLocation_categoryId_idx" ON "PlaceLocation" ("categoryId");
