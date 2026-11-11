-- crates/db/migrations/0001_core.sql  (P2.3: schema part 1, 65 tables)
-- Generated once from packages/database/drizzle/20261110161124_resource_deletion_log/snapshot.json
-- by the P2.2 rules, then reviewed by hand. Do not edit after release: the runner stores a checksum.
-- Needs the app SQL functions of section 3 (regexp_replace is used in one index expression).

-- User
CREATE TABLE "User" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "name" TEXT,
  "email" TEXT NOT NULL,
  "emailVerified" INTEGER NOT NULL DEFAULT 0 CHECK ("emailVerified" IN (0, 1)),
  "image" TEXT,
  "isAnonymous" INTEGER NOT NULL DEFAULT 0 CHECK ("isAnonymous" IN (0, 1)),
  "mustChangePassword" INTEGER NOT NULL DEFAULT 0 CHECK ("mustChangePassword" IN (0, 1)),
  "tenantId" INTEGER NOT NULL DEFAULT 1 REFERENCES "Tenant" ("id") ON DELETE RESTRICT ON UPDATE CASCADE
) STRICT;
CREATE UNIQUE INDEX "User_email_tenant_key" ON "User" ("email", "tenantId");
CREATE INDEX "User_tenantId_idx" ON "User" ("tenantId");

-- Account
CREATE TABLE "Account" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "accountId" TEXT NOT NULL,
  "providerId" TEXT NOT NULL,
  "accessToken" TEXT,
  "accessTokenExpiresAt" INTEGER,
  "refreshToken" TEXT,
  "refreshTokenExpiresAt" INTEGER,
  "scope" TEXT,
  "idToken" TEXT,
  "password" TEXT,
  "userId" INTEGER NOT NULL REFERENCES "User" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "tenantId" INTEGER NOT NULL DEFAULT 1 REFERENCES "Tenant" ("id") ON DELETE RESTRICT ON UPDATE CASCADE
) STRICT;
CREATE INDEX "Account_providerId_accountId_tenantId_idx" ON "Account" ("providerId", "accountId", "tenantId");

-- Session
CREATE TABLE "Session" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "expiresAt" INTEGER NOT NULL,
  "token" TEXT NOT NULL,
  "ipAddress" TEXT,
  "userAgent" TEXT,
  "userId" INTEGER NOT NULL REFERENCES "User" ("id") ON DELETE CASCADE ON UPDATE CASCADE
) STRICT;
CREATE UNIQUE INDEX "Session_token_key" ON "Session" ("token");

-- Verification
CREATE TABLE "Verification" (
  "id" TEXT PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "identifier" TEXT NOT NULL,
  "value" TEXT NOT NULL,
  "expiresAt" INTEGER NOT NULL
) STRICT;
CREATE INDEX "Verification_identifier_idx" ON "Verification" ("identifier");

-- Jwk
CREATE TABLE "Jwk" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "publicKey" TEXT NOT NULL,
  "privateKey" TEXT NOT NULL,
  "alg" TEXT,
  "crv" TEXT,
  "expiresAt" INTEGER
) STRICT;

-- Invitation
CREATE TABLE "Invitation" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "code" TEXT NOT NULL,
  "permissions" TEXT NOT NULL CHECK (json_valid("permissions")),
  "expiresAt" INTEGER NOT NULL,
  "workspaceId" INTEGER REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "invitedBy" INTEGER NOT NULL REFERENCES "User" ("id") ON DELETE CASCADE ON UPDATE CASCADE
) STRICT;
CREATE UNIQUE INDEX "Invitation_code_key" ON "Invitation" ("code");

-- OAuthAccessToken
CREATE TABLE "OAuthAccessToken" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "token" TEXT NOT NULL,
  "clientId" TEXT NOT NULL REFERENCES "OAuthClient" ("clientId") ON DELETE CASCADE ON UPDATE CASCADE,
  "sessionId" INTEGER REFERENCES "Session" ("id") ON DELETE SET NULL ON UPDATE CASCADE,
  "userId" INTEGER REFERENCES "User" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "referenceId" TEXT,
  "authorizationCodeId" TEXT,
  "resources" TEXT CHECK ("resources" IS NULL OR (json_valid("resources") AND json_type("resources") = 'array')),
  "requestedUserInfoClaims" TEXT CHECK ("requestedUserInfoClaims" IS NULL OR (json_valid("requestedUserInfoClaims") AND json_type("requestedUserInfoClaims") = 'array')),
  "refreshId" INTEGER REFERENCES "OAuthRefreshToken" ("id") ON DELETE SET NULL ON UPDATE CASCADE,
  "expiresAt" INTEGER NOT NULL,
  "revoked" INTEGER,
  "confirmation" TEXT CHECK ("confirmation" IS NULL OR json_valid("confirmation")),
  "scopes" TEXT NOT NULL CHECK (json_valid("scopes") AND json_type("scopes") = 'array')
) STRICT;
CREATE UNIQUE INDEX "OAuthAccessToken_token_key" ON "OAuthAccessToken" ("token");
CREATE INDEX "OAuthAccessToken_clientId_idx" ON "OAuthAccessToken" ("clientId");
CREATE INDEX "OAuthAccessToken_sessionId_idx" ON "OAuthAccessToken" ("sessionId");
CREATE INDEX "OAuthAccessToken_userId_idx" ON "OAuthAccessToken" ("userId");
CREATE INDEX "OAuthAccessToken_authorizationCodeId_idx" ON "OAuthAccessToken" ("authorizationCodeId");
CREATE INDEX "OAuthAccessToken_refreshId_idx" ON "OAuthAccessToken" ("refreshId");

-- OAuthClient
CREATE TABLE "OAuthClient" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "clientId" TEXT NOT NULL,
  "clientSecret" TEXT,
  "clientDiscoveryId" TEXT,
  "disabled" INTEGER NOT NULL DEFAULT 0 CHECK ("disabled" IN (0, 1)),
  "skipConsent" INTEGER CHECK ("skipConsent" IN (0, 1)),
  "enableEndSession" INTEGER CHECK ("enableEndSession" IN (0, 1)),
  "subjectType" TEXT,
  "scopes" TEXT CHECK ("scopes" IS NULL OR (json_valid("scopes") AND json_type("scopes") = 'array')),
  "clientCredentialsScopes" TEXT NOT NULL DEFAULT '[]' CHECK (json_valid("clientCredentialsScopes") AND json_type("clientCredentialsScopes") = 'array'),
  "userId" INTEGER REFERENCES "User" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "name" TEXT,
  "uri" TEXT,
  "icon" TEXT,
  "contacts" TEXT CHECK ("contacts" IS NULL OR (json_valid("contacts") AND json_type("contacts") = 'array')),
  "tos" TEXT,
  "policy" TEXT,
  "softwareId" TEXT,
  "softwareVersion" TEXT,
  "softwareStatement" TEXT,
  "redirectUris" TEXT NOT NULL CHECK (json_valid("redirectUris") AND json_type("redirectUris") = 'array'),
  "postLogoutRedirectUris" TEXT CHECK ("postLogoutRedirectUris" IS NULL OR (json_valid("postLogoutRedirectUris") AND json_type("postLogoutRedirectUris") = 'array')),
  "backchannelLogoutUri" TEXT,
  "backchannelLogoutSessionRequired" INTEGER CHECK ("backchannelLogoutSessionRequired" IN (0, 1)),
  "tokenEndpointAuthMethod" TEXT,
  "applicationType" TEXT,
  "jwks" TEXT,
  "jwksUri" TEXT,
  "grantTypes" TEXT CHECK ("grantTypes" IS NULL OR (json_valid("grantTypes") AND json_type("grantTypes") = 'array')),
  "responseTypes" TEXT CHECK ("responseTypes" IS NULL OR (json_valid("responseTypes") AND json_type("responseTypes") = 'array')),
  "requirePKCE" INTEGER CHECK ("requirePKCE" IN (0, 1)),
  "dpopBoundAccessTokens" INTEGER NOT NULL DEFAULT 0 CHECK ("dpopBoundAccessTokens" IN (0, 1)),
  "referenceId" TEXT,
  "metadata" TEXT CHECK ("metadata" IS NULL OR json_valid("metadata"))
) STRICT;
CREATE UNIQUE INDEX "OAuthClient_clientId_key" ON "OAuthClient" ("clientId");
CREATE INDEX "OAuthClient_userId_idx" ON "OAuthClient" ("userId");

-- OAuthClientAssertion
CREATE TABLE "OAuthClientAssertion" (
  "id" TEXT PRIMARY KEY NOT NULL,
  "expiresAt" INTEGER NOT NULL
) STRICT;
CREATE INDEX "OAuthClientAssertion_expiresAt_idx" ON "OAuthClientAssertion" ("expiresAt");

-- OAuthClientResource
CREATE TABLE "OAuthClientResource" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "clientId" TEXT NOT NULL REFERENCES "OAuthClient" ("clientId") ON DELETE CASCADE ON UPDATE CASCADE,
  "resourceId" TEXT NOT NULL REFERENCES "OAuthResource" ("identifier") ON DELETE CASCADE ON UPDATE CASCADE,
  "metadata" TEXT CHECK ("metadata" IS NULL OR json_valid("metadata"))
) STRICT;
CREATE INDEX "OAuthClientResource_clientId_idx" ON "OAuthClientResource" ("clientId");
CREATE INDEX "OAuthClientResource_resourceId_idx" ON "OAuthClientResource" ("resourceId");
CREATE UNIQUE INDEX "OAuthClientResource_clientId_resourceId_key" ON "OAuthClientResource" ("clientId", "resourceId");

-- OAuthConsent
CREATE TABLE "OAuthConsent" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "clientId" TEXT NOT NULL REFERENCES "OAuthClient" ("clientId") ON DELETE CASCADE ON UPDATE CASCADE,
  "userId" INTEGER REFERENCES "User" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "referenceId" TEXT,
  "resources" TEXT CHECK ("resources" IS NULL OR (json_valid("resources") AND json_type("resources") = 'array')),
  "requestedUserInfoClaims" TEXT CHECK ("requestedUserInfoClaims" IS NULL OR (json_valid("requestedUserInfoClaims") AND json_type("requestedUserInfoClaims") = 'array')),
  "scopes" TEXT NOT NULL CHECK (json_valid("scopes") AND json_type("scopes") = 'array'),
  "workspaceId" INTEGER REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE
) STRICT;
CREATE INDEX "OAuthConsent_clientId_idx" ON "OAuthConsent" ("clientId");
CREATE INDEX "OAuthConsent_userId_idx" ON "OAuthConsent" ("userId");
CREATE INDEX "OAuthConsent_workspaceId_idx" ON "OAuthConsent" ("workspaceId") WHERE "workspaceId" IS NOT NULL;

-- OAuthRefreshToken
CREATE TABLE "OAuthRefreshToken" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "token" TEXT NOT NULL,
  "clientId" TEXT NOT NULL REFERENCES "OAuthClient" ("clientId") ON DELETE CASCADE ON UPDATE CASCADE,
  "sessionId" INTEGER REFERENCES "Session" ("id") ON DELETE SET NULL ON UPDATE CASCADE,
  "userId" INTEGER NOT NULL REFERENCES "User" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "referenceId" TEXT,
  "authorizationCodeId" TEXT,
  "resources" TEXT CHECK ("resources" IS NULL OR (json_valid("resources") AND json_type("resources") = 'array')),
  "requestedUserInfoClaims" TEXT CHECK ("requestedUserInfoClaims" IS NULL OR (json_valid("requestedUserInfoClaims") AND json_type("requestedUserInfoClaims") = 'array')),
  "expiresAt" INTEGER NOT NULL,
  "revoked" INTEGER,
  "rotatedAt" INTEGER,
  "rotationReplayResponse" TEXT,
  "rotationReplayExpiresAt" INTEGER,
  "authTime" INTEGER,
  "confirmation" TEXT CHECK ("confirmation" IS NULL OR json_valid("confirmation")),
  "scopes" TEXT NOT NULL CHECK (json_valid("scopes") AND json_type("scopes") = 'array')
) STRICT;
CREATE UNIQUE INDEX "OAuthRefreshToken_token_key" ON "OAuthRefreshToken" ("token");
CREATE INDEX "OAuthRefreshToken_clientId_idx" ON "OAuthRefreshToken" ("clientId");
CREATE INDEX "OAuthRefreshToken_sessionId_idx" ON "OAuthRefreshToken" ("sessionId");
CREATE INDEX "OAuthRefreshToken_userId_idx" ON "OAuthRefreshToken" ("userId");
CREATE INDEX "OAuthRefreshToken_authorizationCodeId_idx" ON "OAuthRefreshToken" ("authorizationCodeId");

-- OAuthResource
CREATE TABLE "OAuthResource" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "identifier" TEXT NOT NULL,
  "name" TEXT NOT NULL,
  "accessTokenTtl" INTEGER,
  "refreshTokenTtl" INTEGER,
  "signingAlgorithm" TEXT,
  "signingKeyId" TEXT,
  "allowedScopes" TEXT CHECK ("allowedScopes" IS NULL OR (json_valid("allowedScopes") AND json_type("allowedScopes") = 'array')),
  "customClaims" TEXT CHECK ("customClaims" IS NULL OR json_valid("customClaims")),
  "dpopBoundAccessTokensRequired" INTEGER NOT NULL DEFAULT 0 CHECK ("dpopBoundAccessTokensRequired" IN (0, 1)),
  "disabled" INTEGER NOT NULL DEFAULT 0 CHECK ("disabled" IN (0, 1)),
  "policyVersion" INTEGER NOT NULL DEFAULT 1,
  "metadata" TEXT CHECK ("metadata" IS NULL OR json_valid("metadata"))
) STRICT;
CREATE UNIQUE INDEX "OAuthResource_identifier_key" ON "OAuthResource" ("identifier");
CREATE INDEX "OAuthResource_disabled_idx" ON "OAuthResource" ("disabled");

-- MobileRefreshToken
CREATE TABLE "MobileRefreshToken" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "userId" INTEGER NOT NULL REFERENCES "User" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "deviceId" TEXT NOT NULL,
  "familyId" TEXT NOT NULL,
  "tokenHash" TEXT NOT NULL,
  "expiresAt" INTEGER NOT NULL,
  "rotatedAt" INTEGER,
  "revoked" INTEGER
) STRICT;
CREATE UNIQUE INDEX "MobileRefreshToken_tokenHash_key" ON "MobileRefreshToken" ("tokenHash");
CREATE INDEX "MobileRefreshToken_userId_idx" ON "MobileRefreshToken" ("userId");
CREATE INDEX "MobileRefreshToken_familyId_rotatedAt_idx" ON "MobileRefreshToken" ("familyId", "rotatedAt");
CREATE INDEX "MobileRefreshToken_expiresAt_idx" ON "MobileRefreshToken" ("expiresAt");

-- UserDeviceToken
CREATE TABLE "UserDeviceToken" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "userId" INTEGER NOT NULL REFERENCES "User" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "workspaceId" INTEGER REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "platform" TEXT NOT NULL CHECK ("platform" IN ('ios', 'android')),
  "token" TEXT NOT NULL,
  "lastSeenAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  CONSTRAINT "UserDeviceToken_token_key" UNIQUE ("token")
) STRICT;
CREATE INDEX "UserDeviceToken_userId_idx" ON "UserDeviceToken" ("userId");

-- PlatformCredential
CREATE TABLE "PlatformCredential" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "userId" INTEGER REFERENCES "User" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "type" TEXT NOT NULL,
  "publicConfig" TEXT NOT NULL CHECK (json_valid("publicConfig")),
  "value" TEXT NOT NULL CHECK (json_valid("value")),
  "livemode" INTEGER NOT NULL DEFAULT 0 CHECK ("livemode" IN (0, 1)),
  "usePlatformCredential" INTEGER NOT NULL DEFAULT 0 CHECK ("usePlatformCredential" IN (0, 1)),
  "isVerified" INTEGER NOT NULL DEFAULT 0 CHECK ("isVerified" IN (0, 1)),
  "verifiedAt" INTEGER,
  "lastUsedAt" INTEGER
) STRICT;
CREATE UNIQUE INDEX "PlatformCredential_user_type_livemode_key" ON "PlatformCredential" ("userId", "type", "livemode") WHERE "userId" IS NOT NULL;
CREATE UNIQUE INDEX "PlatformCredential_platform_type_livemode_key" ON "PlatformCredential" ("type", "livemode") WHERE "userId" IS NULL;
CREATE UNIQUE INDEX "PlatformCredential_user_type_key" ON "PlatformCredential" ("userId", "type") WHERE "userId" IS NOT NULL;
CREATE UNIQUE INDEX "PlatformCredential_platform_type_key" ON "PlatformCredential" ("type") WHERE "userId" IS NULL;
CREATE INDEX "PlatformCredential_userId_idx" ON "PlatformCredential" ("userId");

-- Tenant
-- Root row id=1 is inserted at the end of this file. Circular FK with "User"."tenantId" (checked at write time).
CREATE TABLE "Tenant" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "ownerId" INTEGER REFERENCES "User" ("id") ON DELETE RESTRICT ON UPDATE CASCADE,
  "status" TEXT NOT NULL DEFAULT 'active',
  "disabledReason" TEXT,
  "brandName" TEXT,
  "logoLightPath" TEXT,
  "logoDarkPath" TEXT,
  "faviconPath" TEXT,
  "customCss" TEXT,
  "customJs" TEXT,
  "theme" TEXT,
  "storageUrl" TEXT,
  "policyUrl" TEXT,
  "termsOfServiceUrl" TEXT,
  "signupEmailTemplate" TEXT CHECK ("signupEmailTemplate" IS NULL OR json_valid("signupEmailTemplate")),
  "forgotPasswordEmailTemplate" TEXT CHECK ("forgotPasswordEmailTemplate" IS NULL OR json_valid("forgotPasswordEmailTemplate")),
  "magicLinkEmailTemplate" TEXT CHECK ("magicLinkEmailTemplate" IS NULL OR json_valid("magicLinkEmailTemplate")),
  "accountCredentialsEmailTemplate" TEXT CHECK ("accountCredentialsEmailTemplate" IS NULL OR json_valid("accountCredentialsEmailTemplate")),
  "hiddenChannels" TEXT CHECK ("hiddenChannels" IS NULL OR json_valid("hiddenChannels"))
) STRICT;
CREATE UNIQUE INDEX "Tenant_ownerId_key" ON "Tenant" ("ownerId");

-- PlatformAdmin
CREATE TABLE "PlatformAdmin" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "userId" INTEGER NOT NULL REFERENCES "User" ("id") ON DELETE CASCADE ON UPDATE NO ACTION,
  "role" TEXT NOT NULL DEFAULT 'super-admin'
) STRICT;
CREATE UNIQUE INDEX "PlatformAdmin_userId_key" ON "PlatformAdmin" ("userId");

-- License
CREATE TABLE "License" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "licenseId" TEXT NOT NULL,
  "token" TEXT NOT NULL,
  "expiresAt" INTEGER NOT NULL
) STRICT;
CREATE UNIQUE INDEX "License_licenseId_key" ON "License" ("licenseId");

-- UserQuota
CREATE TABLE "UserQuota" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "userId" INTEGER NOT NULL REFERENCES "User" ("id") ON DELETE CASCADE ON UPDATE NO ACTION,
  "contactsLimit" INTEGER,
  "contactsUsed" INTEGER NOT NULL DEFAULT 0,
  "workspacesLimit" INTEGER,
  "workspacesUsed" INTEGER NOT NULL DEFAULT 0,
  "channelsLimit" INTEGER,
  "channelsUsed" INTEGER NOT NULL DEFAULT 0,
  "teamMembersLimit" INTEGER,
  "teamMembersUsed" INTEGER NOT NULL DEFAULT 0,
  "macLimit" INTEGER,
  "macUsed" INTEGER NOT NULL DEFAULT 0,
  "botMessagesLimit" INTEGER,
  "botMessagesUsed" INTEGER NOT NULL DEFAULT 0,
  "monthlyBotMessagesLimit" INTEGER,
  "monthlyBotMessagesUsed" INTEGER NOT NULL DEFAULT 0,
  "monthlyBotMessagesPeriodStart" INTEGER,
  "botMessagesTopUpGranted" INTEGER NOT NULL DEFAULT 0,
  "whiteLabel" INTEGER NOT NULL DEFAULT 0 CHECK ("whiteLabel" IN (0, 1)),
  "ssoSaml" INTEGER NOT NULL DEFAULT 0 CHECK ("ssoSaml" IN (0, 1)),
  "saasMode" INTEGER NOT NULL DEFAULT 0 CHECK ("saasMode" IN (0, 1)),
  "customAppCredentials" INTEGER NOT NULL DEFAULT 0 CHECK ("customAppCredentials" IN (0, 1)),
  "planName" TEXT,
  "planStatus" TEXT,
  "selectedTrialPlanId" INTEGER,
  "periodStart" INTEGER,
  "periodEnd" INTEGER,
  "channelsTornDownAt" INTEGER,
  "syncedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  CONSTRAINT "UserQuota_userId_key" UNIQUE ("userId")
) STRICT;
CREATE INDEX "UserQuota_channelsTornDownAt_idx" ON "UserQuota" ("channelsTornDownAt");
CREATE INDEX "UserQuota_due_expired_trial_idx" ON "UserQuota" ("userId") WHERE "channelsTornDownAt" IS NULL AND "planStatus" = 'trial';

-- WorkspaceUsage
CREATE TABLE "WorkspaceUsage" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE NO ACTION,
  "contactsUsed" INTEGER NOT NULL DEFAULT 0,
  "channelsUsed" INTEGER NOT NULL DEFAULT 0,
  "teamMembersUsed" INTEGER NOT NULL DEFAULT 0,
  "botMessagesUsed" INTEGER NOT NULL DEFAULT 0,
  "macUsed" INTEGER NOT NULL DEFAULT 0,
  "syncedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  CONSTRAINT "WorkspaceUsage_workspaceId_key" UNIQUE ("workspaceId")
) STRICT;

-- CustomDomain
CREATE TABLE "CustomDomain" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "tenantId" INTEGER NOT NULL REFERENCES "Tenant" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "domain" TEXT NOT NULL,
  "status" TEXT NOT NULL DEFAULT 'pending',
  "verifiedAt" INTEGER,
  "cfHostnameId" TEXT,
  "cfOwnershipValue" TEXT,
  "cfAcmeValue" TEXT
) STRICT;
CREATE UNIQUE INDEX "CustomDomain_tenantId_key" ON "CustomDomain" ("tenantId");
CREATE UNIQUE INDEX "CustomDomain_domain_key" ON "CustomDomain" ("domain");
CREATE INDEX "CustomDomain_status_idx" ON "CustomDomain" ("status");

-- AuditLog
CREATE TABLE "AuditLog" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "action" TEXT NOT NULL,
  "detail" TEXT NOT NULL,
  "ipAddress" TEXT,
  "userAgent" TEXT,
  "source" TEXT,
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "userId" INTEGER REFERENCES "User" ("id") ON DELETE SET NULL ON UPDATE CASCADE
) STRICT;
CREATE INDEX "AuditLog_workspaceId_createdAt_id_idx" ON "AuditLog" ("workspaceId", "createdAt" DESC, "id" DESC);
CREATE INDEX "AuditLog_workspaceId_userId_createdAt_id_idx" ON "AuditLog" ("workspaceId", "userId", "createdAt" DESC, "id" DESC) WHERE "userId" IS NOT NULL;
CREATE INDEX "AuditLog_createdAt_id_idx" ON "AuditLog" ("createdAt" DESC, "id" DESC);

-- TenantHelpItem
CREATE TABLE "TenantHelpItem" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "tenantId" INTEGER NOT NULL REFERENCES "Tenant" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "name" TEXT NOT NULL,
  "url" TEXT NOT NULL,
  "icon" TEXT,
  "position" INTEGER NOT NULL DEFAULT 0
) STRICT;
CREATE INDEX "TenantHelpItem_tenantId_position_idx" ON "TenantHelpItem" ("tenantId", "position");

-- Workspace
CREATE TABLE "Workspace" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "name" TEXT NOT NULL,
  "defaultReply" TEXT,
  "defaultReplyFrequency" TEXT NOT NULL DEFAULT 'allTime' CHECK ("defaultReplyFrequency" IN ('allTime', 'oncePerHour', 'oncePerDay')),
  "targetCountry" TEXT,
  "language" TEXT NOT NULL DEFAULT 'en',
  "timezone" TEXT NOT NULL DEFAULT 'UTC',
  "brandColor" TEXT NOT NULL DEFAULT '#016DFF',
  "developmentMode" INTEGER NOT NULL DEFAULT 0 CHECK ("developmentMode" IN (0, 1)),
  "smartResponseDelaySeconds" INTEGER,
  "isActive" INTEGER NOT NULL DEFAULT 1 CHECK ("isActive" IN (0, 1)),
  "startTime" TEXT,
  "endTime" TEXT,
  "logo" TEXT,
  "scheduledDeletionAt" INTEGER,
  "deletionRequestedById" INTEGER,
  "deletionRequestedAt" INTEGER,
  "purgeStartedAt" INTEGER,
  "supportAccessUntil" INTEGER,
  "capiLimitedDataUse" INTEGER NOT NULL DEFAULT 0 CHECK ("capiLimitedDataUse" IN (0, 1)),
  "token" TEXT,
  "ownerId" INTEGER NOT NULL REFERENCES "User" ("id") ON DELETE RESTRICT ON UPDATE CASCADE,
  "tenantId" INTEGER NOT NULL DEFAULT 1 REFERENCES "Tenant" ("id") ON DELETE RESTRICT ON UPDATE CASCADE
) STRICT;
CREATE INDEX "Workspace_ownerId_idx" ON "Workspace" ("ownerId");
CREATE INDEX "Workspace_tenantId_idx" ON "Workspace" ("tenantId");
CREATE INDEX "Workspace_scheduledDeletionAt_idx" ON "Workspace" ("scheduledDeletionAt");
CREATE INDEX "Workspace_supportAccessUntil_idx" ON "Workspace" ("supportAccessUntil");

-- WorkspaceMember
CREATE TABLE "WorkspaceMember" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "userId" INTEGER NOT NULL REFERENCES "User" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "role" TEXT NOT NULL CHECK ("role" IN ('owner', 'agent')),
  "notificationChannels" TEXT NOT NULL DEFAULT '{}' CHECK (json_valid("notificationChannels")),
  "notificationTypes" TEXT NOT NULL DEFAULT '{}' CHECK (json_valid("notificationTypes")),
  "permissions" TEXT NOT NULL DEFAULT '{}' CHECK (json_valid("permissions")),
  "onlineSince" INTEGER
) STRICT;
CREATE INDEX "WorkspaceMember_workspaceId_userId_idx" ON "WorkspaceMember" ("workspaceId", "userId");

-- WorkspaceApiToken
CREATE TABLE "WorkspaceApiToken" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "name" TEXT NOT NULL,
  "permission" TEXT NOT NULL CHECK ("permission" IN ('full', 'read_only')),
  "tokenHash" TEXT NOT NULL,
  "tokenPrefix" TEXT,
  "isDefault" INTEGER NOT NULL DEFAULT 0 CHECK ("isDefault" IN (0, 1)),
  "encryptedToken" TEXT CHECK ("encryptedToken" IS NULL OR json_valid("encryptedToken")),
  "scopes" TEXT CHECK ("scopes" IS NULL OR (json_valid("scopes") AND json_type("scopes") = 'array')),
  CONSTRAINT "WorkspaceApiToken_tokenHash_key" UNIQUE ("tokenHash")
) STRICT;
CREATE INDEX "WorkspaceApiToken_workspaceId_idx" ON "WorkspaceApiToken" ("workspaceId");
CREATE UNIQUE INDEX "WorkspaceApiToken_workspaceId_default_key" ON "WorkspaceApiToken" ("workspaceId") WHERE "isDefault";

-- WorkspaceDeletionRecord
CREATE TABLE "WorkspaceDeletionRecord" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "workspaceId" INTEGER NOT NULL,
  "workspaceName" TEXT NOT NULL,
  "ownerId" INTEGER NOT NULL,
  "ownerEmail" TEXT,
  "tenantId" INTEGER NOT NULL,
  "deletionRequestedById" INTEGER,
  "deletionRequestedByEmail" TEXT,
  "deletionRequestedAt" INTEGER,
  "scheduledDeletionAt" INTEGER,
  "purgedAt" INTEGER,
  "usageSnapshot" TEXT NOT NULL CHECK (json_valid("usageSnapshot")),
  "details" TEXT NOT NULL CHECK (json_valid("details")),
  CONSTRAINT "WorkspaceDeletionRecord_workspaceId_key" UNIQUE ("workspaceId")
) STRICT;
CREATE INDEX "WorkspaceDeletionRecord_ownerEmail_idx" ON "WorkspaceDeletionRecord" ("ownerEmail");

-- Folder
CREATE TABLE "Folder" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "name" TEXT NOT NULL,
  "folderType" TEXT NOT NULL CHECK ("folderType" IN ('tag', 'flow', 'customField', 'automatedResponse', 'trigger', 'webhook', 'sequence', 'emailTopic', 'fbComment', 'igComment', 'igStory', 'outboundAutomatedResponse')),
  "parentId" INTEGER,
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "isTrash" INTEGER NOT NULL DEFAULT 0 CHECK ("isTrash" IN (0, 1)),
  "paths" TEXT NOT NULL CHECK (json_valid("paths") AND json_type("paths") = 'array')  -- NO DB default (Postgres parity: drizzle .default(sql`[]`) is app-side only). Rust always writes it.
) STRICT;
CREATE INDEX "Folder_workspaceId_idx" ON "Folder" ("workspaceId");
CREATE INDEX "Folder_parentId_idx" ON "Folder" ("parentId");

-- Inbox
CREATE TABLE "Inbox" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "name" TEXT NOT NULL,
  "channel" TEXT NOT NULL,
  "sourceId" TEXT NOT NULL,
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "status" TEXT NOT NULL DEFAULT 'connected',
  "markReadOnOutbound" INTEGER NOT NULL DEFAULT 0 CHECK ("markReadOnOutbound" IN (0, 1)),
  "disconnectedAt" INTEGER,
  "disconnectReason" TEXT,
  "threadControlSeenAt" INTEGER
) STRICT;
CREATE INDEX "Inbox_workspaceId_idx" ON "Inbox" ("workspaceId");
CREATE UNIQUE INDEX "Inbox_workspaceId_channel_sourceId_key" ON "Inbox" ("workspaceId", "channel", "sourceId");

-- Integration
CREATE TABLE "Integration" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "integrationType" TEXT NOT NULL
) STRICT;
CREATE INDEX "Integration_workspaceId_idx" ON "Integration" ("workspaceId");
CREATE INDEX "Integration_workspaceId_integrationType_key" ON "Integration" ("workspaceId", "integrationType");

-- IntegrationWhatsapp
-- "handoverResumeFlowId": added in 0002_messaging.sql (P2.4) by ALTER TABLE ADD COLUMN (FK to "Flow").
CREATE TABLE "IntegrationWhatsapp" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "auth" TEXT NOT NULL CHECK (json_valid("auth")),
  "phoneNumberId" TEXT NOT NULL,
  "wabaId" TEXT NOT NULL,
  "businessId" TEXT NOT NULL,
  "name" TEXT NOT NULL,
  "displayPhoneNumber" TEXT NOT NULL DEFAULT '',
  "coexistEnabled" INTEGER NOT NULL DEFAULT 0 CHECK ("coexistEnabled" IN (0, 1)),
  "callingEnabled" INTEGER CHECK ("callingEnabled" IN (0, 1)),
  "inboundCallsEnabled" INTEGER NOT NULL DEFAULT 1 CHECK ("inboundCallsEnabled" IN (0, 1)),
  "callHours" TEXT CHECK ("callHours" IS NULL OR json_valid("callHours")),
  "callRecordingEnabled" INTEGER NOT NULL DEFAULT 0 CHECK ("callRecordingEnabled" IN (0, 1)),
  "callRecordingRetentionDays" INTEGER NOT NULL DEFAULT 90,
  "callTranscriptionEnabled" INTEGER NOT NULL DEFAULT 0 CHECK ("callTranscriptionEnabled" IN (0, 1)),
  "callRecordingMode" TEXT NOT NULL DEFAULT 'metaNative' CHECK ("callRecordingMode" IN ('metaNative', 'browserWhisper')),
  "callTranscriptionMode" TEXT NOT NULL DEFAULT 'metaNative' CHECK ("callTranscriptionMode" IN ('metaNative', 'browserWhisper')),
  "callAnnouncementLanguage" TEXT,
  "callRecordingPurpose" TEXT,
  "pixRedirectEnabled" INTEGER NOT NULL DEFAULT 0 CHECK ("pixRedirectEnabled" IN (0, 1)),
  "coexistAiReadsSyncedHistory" INTEGER NOT NULL DEFAULT 0 CHECK ("coexistAiReadsSyncedHistory" IN (0, 1)),
  "isCoexist" INTEGER NOT NULL DEFAULT 0 CHECK ("isCoexist" IN (0, 1)),
  "platformType" TEXT NOT NULL DEFAULT '',
  "historyDeclined" INTEGER NOT NULL DEFAULT 0 CHECK ("historyDeclined" IN (0, 1)),
  "hasCapiScope" INTEGER NOT NULL DEFAULT 0 CHECK ("hasCapiScope" IN (0, 1)),
  "capiScopeCheckedAt" INTEGER,
  "datasetId" TEXT,
  "capiAccessToken" TEXT CHECK ("capiAccessToken" IS NULL OR json_valid("capiAccessToken")),
  "capiDisconnectedAt" INTEGER,
  "capiTestEventCode" TEXT,
  "registrationStatus" TEXT NOT NULL DEFAULT 'pending_verification' CHECK ("registrationStatus" IN ('pending_verification', 'registered', 'failed')),
  "registrationError" TEXT CHECK ("registrationError" IS NULL OR json_valid("registrationError")),
  "verificationCodeRequestedAt" INTEGER,
  "tokenRefreshError" TEXT,
  "routingRoles" TEXT CHECK ("routingRoles" IS NULL OR json_valid("routingRoles")),
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "inboxId" INTEGER NOT NULL REFERENCES "Inbox" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  CONSTRAINT "IntegrationWhatsapp_registrationStatus_error_consistent" CHECK (("registrationStatus" <> 'failed' OR "registrationError" IS NOT NULL) AND ("registrationStatus" <> 'registered' OR "registrationError" IS NULL))
) STRICT;
CREATE UNIQUE INDEX "IntegrationWhatsapp_inboxId_key" ON "IntegrationWhatsapp" ("inboxId");
CREATE INDEX "IntegrationWhatsapp_workspaceId_idx" ON "IntegrationWhatsapp" ("workspaceId");
CREATE INDEX "IntegrationWhatsapp_wabaId_idx" ON "IntegrationWhatsapp" ("wabaId");
CREATE UNIQUE INDEX "IntegrationWhatsapp_phoneNumberId_key" ON "IntegrationWhatsapp" ("phoneNumberId");

-- IntegrationMessenger
-- "welcomeFlowId", "handoverResumeFlowId": added in 0002_messaging.sql (P2.4) by ALTER TABLE ADD COLUMN (FK to "Flow").
CREATE TABLE "IntegrationMessenger" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "auth" TEXT NOT NULL CHECK (json_valid("auth")),
  "userInfo" TEXT CHECK ("userInfo" IS NULL OR json_valid("userInfo")),
  "pageId" TEXT NOT NULL,
  "name" TEXT NOT NULL,
  "conversationStarters" TEXT NOT NULL CHECK (json_valid("conversationStarters")),  -- NO DB default (Postgres parity: drizzle .default(sql`[]`) is app-side only). Rust always writes it.
  "persistentMenus" TEXT NOT NULL CHECK (json_valid("persistentMenus")),  -- NO DB default (Postgres parity: drizzle .default(sql`[]`) is app-side only). Rust always writes it.
  "personas" TEXT NOT NULL CHECK (json_valid("personas")),  -- NO DB default (Postgres parity: drizzle .default(sql`[]`) is app-side only). Rust always writes it.
  "personaId" TEXT,
  "coexistEnabled" INTEGER NOT NULL DEFAULT 0 CHECK ("coexistEnabled" IN (0, 1)),
  "coexistAiReadsSyncedHistory" INTEGER NOT NULL DEFAULT 0 CHECK ("coexistAiReadsSyncedHistory" IN (0, 1)),
  "hasCapiScope" INTEGER NOT NULL DEFAULT 0 CHECK ("hasCapiScope" IN (0, 1)),
  "capiScopeCheckedAt" INTEGER,
  "datasetId" TEXT,
  "capiAccessToken" TEXT CHECK ("capiAccessToken" IS NULL OR json_valid("capiAccessToken")),
  "capiDisconnectedAt" INTEGER,
  "capiTestEventCode" TEXT,
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "inboxId" INTEGER NOT NULL REFERENCES "Inbox" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "syncTagEnabledAt" INTEGER,
  "tokenRefreshError" TEXT
) STRICT;
CREATE INDEX "IntegrationMessenger_workspaceId_idx" ON "IntegrationMessenger" ("workspaceId");
CREATE UNIQUE INDEX "IntegrationMessenger_inboxId_key" ON "IntegrationMessenger" ("inboxId");
CREATE UNIQUE INDEX "IntegrationMessenger_pageId_key" ON "IntegrationMessenger" ("pageId");

-- IntegrationInstagram
-- "welcomeFlowId": added in 0002_messaging.sql (P2.4) by ALTER TABLE ADD COLUMN (FK to "Flow").
CREATE TABLE "IntegrationInstagram" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "auth" TEXT NOT NULL CHECK (json_valid("auth")),
  "userInfo" TEXT CHECK ("userInfo" IS NULL OR json_valid("userInfo")),
  "igId" TEXT NOT NULL,
  "pageId" TEXT NOT NULL,
  "name" TEXT NOT NULL,
  "username" TEXT NOT NULL,
  "coexistEnabled" INTEGER NOT NULL DEFAULT 0 CHECK ("coexistEnabled" IN (0, 1)),
  "coexistAiReadsSyncedHistory" INTEGER NOT NULL DEFAULT 0 CHECK ("coexistAiReadsSyncedHistory" IN (0, 1)),
  "hasCapiScope" INTEGER NOT NULL DEFAULT 0 CHECK ("hasCapiScope" IN (0, 1)),
  "capiScopeCheckedAt" INTEGER,
  "datasetId" TEXT,
  "capiAccessToken" TEXT CHECK ("capiAccessToken" IS NULL OR json_valid("capiAccessToken")),
  "capiDisconnectedAt" INTEGER,
  "capiTestEventCode" TEXT,
  "conversationStarters" TEXT NOT NULL CHECK (json_valid("conversationStarters") AND json_type("conversationStarters") = 'array'),  -- NO DB default (Postgres parity: drizzle .default(sql`[]`) is app-side only). Rust always writes it.
  "persistentMenus" TEXT NOT NULL CHECK (json_valid("persistentMenus") AND json_type("persistentMenus") = 'array'),  -- NO DB default (Postgres parity: drizzle .default(sql`[]`) is app-side only). Rust always writes it.
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "inboxId" INTEGER NOT NULL REFERENCES "Inbox" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "type" TEXT NOT NULL DEFAULT 'instagram',
  "tokenRefreshError" TEXT
) STRICT;
CREATE INDEX "IntegrationInstagram_workspaceId_idx" ON "IntegrationInstagram" ("workspaceId");
CREATE UNIQUE INDEX "IntegrationInstagram_inboxId_key" ON "IntegrationInstagram" ("inboxId");
CREATE UNIQUE INDEX "IntegrationInstagram_igId_key" ON "IntegrationInstagram" ("igId");

-- IntegrationTelegram
CREATE TABLE "IntegrationTelegram" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "auth" TEXT NOT NULL CHECK (json_valid("auth")),
  "botId" TEXT NOT NULL,
  "name" TEXT NOT NULL,
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "inboxId" INTEGER NOT NULL REFERENCES "Inbox" ("id") ON DELETE CASCADE ON UPDATE CASCADE
) STRICT;
CREATE INDEX "IntegrationTelegram_workspaceId_idx" ON "IntegrationTelegram" ("workspaceId");
CREATE UNIQUE INDEX "IntegrationTelegram_inboxId_key" ON "IntegrationTelegram" ("inboxId");
CREATE UNIQUE INDEX "IntegrationTelegram_botId_key" ON "IntegrationTelegram" ("botId");

-- IntegrationWebchat
-- "welcomeFlowId": added in 0002_messaging.sql (P2.4) by ALTER TABLE ADD COLUMN (FK to "Flow").
CREATE TABLE "IntegrationWebchat" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "auth" TEXT NOT NULL CHECK (json_valid("auth")),
  "name" TEXT NOT NULL,
  "enable" INTEGER NOT NULL DEFAULT 1 CHECK ("enable" IN (0, 1)),
  "authorizedDomains" TEXT NOT NULL CHECK (json_valid("authorizedDomains") AND json_type("authorizedDomains") = 'array'),  -- NO DB default (Postgres parity: drizzle .default(sql`[]`) is app-side only). Rust always writes it.
  "conversationStarters" TEXT NOT NULL CHECK (json_valid("conversationStarters") AND json_type("conversationStarters") = 'array'),  -- NO DB default (Postgres parity: drizzle .default(sql`[]`) is app-side only). Rust always writes it.
  "persistentMenus" TEXT NOT NULL CHECK (json_valid("persistentMenus") AND json_type("persistentMenus") = 'array'),  -- NO DB default (Postgres parity: drizzle .default(sql`[]`) is app-side only). Rust always writes it.
  "brandColor" TEXT NOT NULL,
  "hideHeader" INTEGER NOT NULL DEFAULT 0 CHECK ("hideHeader" IN (0, 1)),
  "showLogo" INTEGER NOT NULL DEFAULT 1 CHECK ("showLogo" IN (0, 1)),
  "hideMessageInput" INTEGER NOT NULL DEFAULT 0 CHECK ("hideMessageInput" IN (0, 1)),
  "customCss" TEXT,
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "inboxId" INTEGER NOT NULL REFERENCES "Inbox" ("id") ON DELETE CASCADE ON UPDATE CASCADE
) STRICT;
CREATE INDEX "IntegrationWebchat_workspaceId_idx" ON "IntegrationWebchat" ("workspaceId");
CREATE INDEX "IntegrationWebchat_inboxId_idx" ON "IntegrationWebchat" ("inboxId");
CREATE UNIQUE INDEX "IntegrationWebchat_inboxId_key" ON "IntegrationWebchat" ("inboxId");

-- IntegrationTiktok
CREATE TABLE "IntegrationTiktok" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "auth" TEXT NOT NULL CHECK (json_valid("auth")),
  "openId" TEXT NOT NULL,
  "name" TEXT NOT NULL,
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "inboxId" INTEGER NOT NULL REFERENCES "Inbox" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "tokenRefreshError" TEXT
) STRICT;
CREATE INDEX "IntegrationTiktok_workspaceId_idx" ON "IntegrationTiktok" ("workspaceId");
CREATE UNIQUE INDEX "IntegrationTiktok_inboxId_key" ON "IntegrationTiktok" ("inboxId");
CREATE UNIQUE INDEX "IntegrationTiktok_openId_key" ON "IntegrationTiktok" ("openId");

-- IntegrationZalo
-- "fallbackFlowId": added in 0002_messaging.sql (P2.4) by ALTER TABLE ADD COLUMN (FK to "Flow").
CREATE TABLE "IntegrationZalo" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "auth" TEXT NOT NULL CHECK (json_valid("auth")),
  "oaId" TEXT NOT NULL,
  "name" TEXT NOT NULL,
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "inboxId" INTEGER NOT NULL REFERENCES "Inbox" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "syncTagEnabledAt" INTEGER,
  "tokenRefreshError" TEXT
) STRICT;
CREATE INDEX "IntegrationZalo_workspaceId_idx" ON "IntegrationZalo" ("workspaceId");
CREATE INDEX "IntegrationZalo_oaId_idx" ON "IntegrationZalo" ("oaId");
CREATE UNIQUE INDEX "IntegrationZalo_inboxId_key" ON "IntegrationZalo" ("inboxId");

-- IntegrationThreads
CREATE TABLE "IntegrationThreads" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "auth" TEXT NOT NULL CHECK (json_valid("auth")),
  "threadsUserId" TEXT NOT NULL,
  "username" TEXT NOT NULL,
  "name" TEXT NOT NULL,
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "inboxId" INTEGER NOT NULL REFERENCES "Inbox" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "tokenRefreshError" TEXT
) STRICT;
CREATE INDEX "IntegrationThreads_workspaceId_idx" ON "IntegrationThreads" ("workspaceId");
CREATE UNIQUE INDEX "IntegrationThreads_inboxId_key" ON "IntegrationThreads" ("inboxId");
CREATE UNIQUE INDEX "IntegrationThreads_threadsUserId_key" ON "IntegrationThreads" ("threadsUserId");

-- IntegrationSmtp
CREATE TABLE "IntegrationSmtp" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "auth" TEXT NOT NULL CHECK (json_valid("auth")),
  "name" TEXT NOT NULL,
  "fromAddress" TEXT NOT NULL,
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "inboxId" INTEGER NOT NULL REFERENCES "Inbox" ("id") ON DELETE CASCADE ON UPDATE CASCADE
) STRICT;
CREATE INDEX "IntegrationSmtp_workspaceId_idx" ON "IntegrationSmtp" ("workspaceId");
CREATE UNIQUE INDEX "IntegrationSmtp_inboxId_key" ON "IntegrationSmtp" ("inboxId");

-- IntegrationApi
CREATE TABLE "IntegrationApi" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "auth" TEXT NOT NULL CHECK (json_valid("auth")),
  "name" TEXT NOT NULL,
  "tokenHash" TEXT NOT NULL,
  "tokenPrefix" TEXT NOT NULL,
  "callbackUrl" TEXT,
  "enabled" INTEGER NOT NULL DEFAULT 1 CHECK ("enabled" IN (0, 1)),
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "inboxId" INTEGER NOT NULL REFERENCES "Inbox" ("id") ON DELETE CASCADE ON UPDATE CASCADE
) STRICT;
CREATE INDEX "IntegrationApi_workspaceId_idx" ON "IntegrationApi" ("workspaceId");
CREATE UNIQUE INDEX "IntegrationApi_inboxId_key" ON "IntegrationApi" ("inboxId");
CREATE UNIQUE INDEX "IntegrationApi_tokenHash_key" ON "IntegrationApi" ("tokenHash");

-- WhatsappBusinessAccount
CREATE TABLE "WhatsappBusinessAccount" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "wabaId" TEXT NOT NULL,
  "businessId" TEXT NOT NULL,
  "credential" TEXT NOT NULL CHECK (json_valid("credential")),
  "grantedScopes" TEXT NOT NULL DEFAULT '[]' CHECK (json_valid("grantedScopes") AND json_type("grantedScopes") = 'array'),
  "scopeCheckedAt" INTEGER,
  "provisionedAt" INTEGER,
  "creditLineId" TEXT,
  "revision" INTEGER NOT NULL DEFAULT 0
) STRICT;
CREATE UNIQUE INDEX "WhatsappBusinessAccount_workspaceId_wabaId_key" ON "WhatsappBusinessAccount" ("workspaceId", "wabaId");

-- WhatsappSignupSession
CREATE TABLE "WhatsappSignupSession" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "userId" INTEGER NOT NULL REFERENCES "User" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "ownerId" INTEGER NOT NULL REFERENCES "User" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "workspaceId" INTEGER REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "wabaId" TEXT NOT NULL,
  "businessId" TEXT NOT NULL,
  "encryptedAccessToken" TEXT NOT NULL CHECK (json_valid("encryptedAccessToken")),
  "apiVersion" TEXT NOT NULL,
  "candidatePhoneNumberIds" TEXT NOT NULL CHECK (json_valid("candidatePhoneNumberIds") AND json_type("candidatePhoneNumberIds") = 'array'),
  "claimedPhoneNumberIds" TEXT NOT NULL DEFAULT '[]' CHECK (json_valid("claimedPhoneNumberIds") AND json_type("claimedPhoneNumberIds") = 'array'),
  "expiresAt" INTEGER NOT NULL,
  "consumedAt" INTEGER,
  CONSTRAINT "WhatsappSignupSession_candidates_not_empty" CHECK (json_array_length("candidatePhoneNumberIds") > 0)
) STRICT;
CREATE INDEX "WhatsappSignupSession_userId_idx" ON "WhatsappSignupSession" ("userId");
CREATE INDEX "WhatsappSignupSession_expiresAt_idx" ON "WhatsappSignupSession" ("expiresAt");

-- Connection
CREATE TABLE "Connection" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "provider" TEXT NOT NULL,
  "kind" TEXT NOT NULL,
  "channel" TEXT,
  "inboxId" INTEGER REFERENCES "Inbox" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "integrationId" INTEGER REFERENCES "Integration" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "sourceId" TEXT NOT NULL,
  "displayName" TEXT NOT NULL,
  "status" TEXT NOT NULL DEFAULT 'connected',
  "statusReason" TEXT,
  "lastError" TEXT,
  "authExpiresAt" INTEGER,
  "createdBy" INTEGER REFERENCES "User" ("id") ON DELETE SET NULL ON UPDATE CASCADE,
  "connectedAt" INTEGER,
  "disconnectedAt" INTEGER,
  "providerEventAt" INTEGER,
  "providerEventKind" TEXT,
  CONSTRAINT "Connection_kind_relation_check" CHECK (( ("kind" = 'channel' AND "inboxId" IS NOT NULL AND "channel" IS NOT NULL) OR ("kind" = 'integration' AND "inboxId" IS NULL AND "channel" IS NULL AND "integrationId" IS NOT NULL) )),
  CONSTRAINT "Connection_inbox_integration_exclusive_check" CHECK (NOT ("inboxId" IS NOT NULL AND "integrationId" IS NOT NULL)),
  CONSTRAINT "Connection_kind_check" CHECK ("kind" IN ('channel', 'integration')),
  CONSTRAINT "Connection_status_check" CHECK ("status" IN ('connected', 'degraded', 'needs_reauth', 'paused', 'disconnected')),
  CONSTRAINT "Connection_statusReason_check" CHECK ("statusReason" IN ('manual', 'workspace_purge', 'trial_expired', 'tenant_suspended', 'token_revoked', 'provider_revoked', 'provider_offboarded', 'refresh_failed', 'verify_failed', 'quota_exceeded', 'orphaned_webhook')),
  CONSTRAINT "Connection_providerEventKind_check" CHECK ("providerEventKind" IS NULL OR "providerEventKind" IN ('revoked', 'offboarded', 'reconnected')),
  CONSTRAINT "Connection_status_reason_check" CHECK (("status" = 'connected') = ("statusReason" IS NULL)),
  CONSTRAINT "Connection_disconnectedAt_check" CHECK (("status" = 'disconnected') = ("disconnectedAt" IS NOT NULL))
) STRICT;
CREATE UNIQUE INDEX "Connection_workspaceId_provider_sourceId_key" ON "Connection" ("workspaceId", "provider", "sourceId");
CREATE INDEX "Connection_provider_sourceId_idx" ON "Connection" ("provider", "sourceId");
CREATE UNIQUE INDEX "Connection_inboxId_key" ON "Connection" ("inboxId");
CREATE UNIQUE INDEX "Connection_integrationId_key" ON "Connection" ("integrationId");

-- ConnectSession
CREATE TABLE "ConnectSession" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "provider" TEXT NOT NULL,
  "purpose" TEXT NOT NULL,
  "targetConnectionId" INTEGER REFERENCES "Connection" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "actorUserId" INTEGER REFERENCES "User" ("id") ON DELETE SET NULL ON UPDATE CASCADE,
  "actorTokenId" INTEGER REFERENCES "WorkspaceApiToken" ("id") ON DELETE SET NULL ON UPDATE CASCADE,
  "platformOwnerId" TEXT,
  "originHost" TEXT,
  "returnUrl" TEXT,
  "stateNonceHash" TEXT NOT NULL,
  "status" TEXT NOT NULL DEFAULT 'pending',
  "step" TEXT NOT NULL DEFAULT 'authorize',
  "nextAction" TEXT CHECK ("nextAction" IS NULL OR json_valid("nextAction")),
  "encryptedAuth" TEXT CHECK ("encryptedAuth" IS NULL OR json_valid("encryptedAuth")),
  "targets" TEXT NOT NULL DEFAULT '[]' CHECK (json_valid("targets")),
  "targetClaims" TEXT NOT NULL DEFAULT '{}' CHECK (json_valid("targetClaims")),
  "resultConnectionIds" TEXT NOT NULL DEFAULT '[]' CHECK (json_valid("resultConnectionIds") AND json_type("resultConnectionIds") = 'array'),
  "results" TEXT NOT NULL DEFAULT '[]' CHECK (json_valid("results")),
  "errorCode" TEXT,
  "expiresAt" INTEGER NOT NULL,
  "consumedAt" INTEGER,
  CONSTRAINT "ConnectSession_actor_at_most_one" CHECK ((("actorUserId" IS NOT NULL) + ("actorTokenId" IS NOT NULL)) <= 1),
  CONSTRAINT "ConnectSession_status_check" CHECK ("status" IN ('pending', 'authorized', 'awaiting_selection', 'completed', 'failed', 'expired', 'cancelled')),
  CONSTRAINT "ConnectSession_purpose_check" CHECK ("purpose" IN ('connect', 'reconnect', 'facebook_ads', 'messaging_ads', 'lead_ads', 'meta_catalog')),
  CONSTRAINT "ConnectSession_errorCode_check" CHECK ("errorCode" IN ('state_mismatch', 'expired', 'provider_denied', 'exchange_failed', 'provider_error', 'no_candidates', 'already_connected', 'quota_exceeded', 'trial_expired', 'internal_error')),
  CONSTRAINT "ConnectSession_errorCode_terminal_failure_check" CHECK ("errorCode" IS NULL OR "status" IN ('failed', 'expired', 'cancelled')),
  CONSTRAINT "ConnectSession_terminal_consumedAt_check" CHECK (("status" IN ('completed', 'failed', 'expired', 'cancelled')) = ("consumedAt" IS NOT NULL)),
  CONSTRAINT "ConnectSession_terminal_clears_encryptedAuth_check" CHECK ("status" NOT IN ('completed', 'failed', 'expired', 'cancelled') OR "encryptedAuth" IS NULL),
  CONSTRAINT "ConnectSession_active_reconnect_requires_target_check" CHECK ("purpose" <> 'reconnect' OR "status" IN ('completed', 'failed', 'expired', 'cancelled') OR "targetConnectionId" IS NOT NULL)
) STRICT;
CREATE INDEX "ConnectSession_workspaceId_idx" ON "ConnectSession" ("workspaceId");
CREATE INDEX "ConnectSession_expiresAt_idx" ON "ConnectSession" ("expiresAt");
CREATE INDEX "ConnectSession_consumedAt_idx" ON "ConnectSession" ("consumedAt") WHERE "consumedAt" IS NOT NULL;
CREATE UNIQUE INDEX "ConnectSession_stateNonceHash_key" ON "ConnectSession" ("stateNonceHash");

-- InboxTeam
CREATE TABLE "InboxTeam" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "name" TEXT NOT NULL
) STRICT;
CREATE INDEX "InboxTeam_workspaceId_idx" ON "InboxTeam" ("workspaceId");

-- InboxTeamMember
CREATE TABLE "InboxTeamMember" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "inboxTeamId" INTEGER NOT NULL REFERENCES "InboxTeam" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "userId" INTEGER NOT NULL REFERENCES "User" ("id") ON DELETE CASCADE ON UPDATE CASCADE
) STRICT;
CREATE INDEX "InboxTeamMember_userId_inboxTeamId_idx" ON "InboxTeamMember" ("userId", "inboxTeamId");

-- InboxContactStat
-- Written only by the "ContactInbox_stats_*" triggers (below).
CREATE TABLE "InboxContactStat" (
  "inboxId" INTEGER PRIMARY KEY NOT NULL REFERENCES "Inbox" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "totalContacts" INTEGER NOT NULL DEFAULT 0,
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER))
) STRICT;

-- TelegramChat
CREATE TABLE "TelegramChat" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "inboxId" INTEGER NOT NULL REFERENCES "Inbox" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "chatId" TEXT NOT NULL,
  "title" TEXT,
  "type" TEXT NOT NULL CHECK ("type" IN ('private', 'group', 'supergroup', 'channel')),
  "botStatus" TEXT NOT NULL CHECK ("botStatus" IN ('creator', 'administrator', 'member', 'restricted', 'left', 'kicked'))
) STRICT;
CREATE UNIQUE INDEX "TelegramChat_inboxId_chatId_key" ON "TelegramChat" ("inboxId", "chatId");
CREATE INDEX "TelegramChat_workspaceId_idx" ON "TelegramChat" ("workspaceId");

-- Contact
-- "fullName": STORED generated column, same expression as Postgres. Trigram GIN indexes -> "Contact_fts" (below).
CREATE TABLE "Contact" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "avatar" TEXT,
  "phoneNumber" TEXT,
  "email" TEXT,
  "emailVerified" INTEGER NOT NULL DEFAULT 0 CHECK ("emailVerified" IN (0, 1)),
  "emailOptIn" INTEGER NOT NULL DEFAULT 1 CHECK ("emailOptIn" IN (0, 1)),
  "firstName" TEXT,
  "lastName" TEXT,
  "fullName" TEXT GENERATED ALWAYS AS (CASE WHEN "firstName" IS NULL AND "lastName" IS NULL THEN NULL WHEN "firstName" IS NULL THEN "lastName" WHEN "lastName" IS NULL THEN "firstName" ELSE "firstName" || ' ' || "lastName" END) STORED,
  "gender" TEXT CHECK ("gender" IN ('male', 'female', 'unknown')),
  "lastReadAt" INTEGER,
  "ref" TEXT,
  "country" TEXT,
  "state" TEXT,
  "city" TEXT,
  "location" TEXT CHECK ("location" IS NULL OR json_valid("location")),
  "locale" TEXT,
  "timezone" TEXT,
  "subscribedAt" INTEGER,
  "broadcastSubscribedAt" INTEGER,
  "blockedAt" INTEGER,
  "totalTagged" INTEGER NOT NULL DEFAULT 0,
  "totalNewTagged" INTEGER NOT NULL DEFAULT 0,
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE
) STRICT;
CREATE INDEX "idx_contact_broadcast_subscribed_at" ON "Contact" ("broadcastSubscribedAt");
CREATE INDEX "idx_contact_workspace_created_at" ON "Contact" ("workspaceId", "createdAt");
CREATE INDEX "idx_contact_workspace_email" ON "Contact" ("workspaceId", "email");
CREATE INDEX "idx_contact_workspace_phone_number" ON "Contact" ("workspaceId", "phoneNumber");
-- Postgres: md5(btrim(regexp_replace("fullName", '[ \t\n\v\f\r]+', ' ', 'g'))). md5 only kept the btree key small;
-- SQLite has no key-size limit, so the index keys the normalized name itself. regexp_replace is an app function (section 3).
CREATE INDEX "idx_contact_workspace_full_name_md5" ON "Contact" ("workspaceId", trim(regexp_replace("fullName", '[ \t\n\v\f\r]+', ' ', 'g')));

-- ContactInbox
CREATE TABLE "ContactInbox" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "originalContactId" INTEGER NOT NULL,
  "contactId" INTEGER NOT NULL REFERENCES "Contact" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "inboxId" INTEGER NOT NULL REFERENCES "Inbox" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "channel" TEXT NOT NULL,
  "source" TEXT NOT NULL,
  "sourceId" TEXT NOT NULL,
  "followsBusiness" INTEGER CHECK ("followsBusiness" IN (0, 1)),
  "businessFollowsContact" INTEGER CHECK ("businessFollowsContact" IN (0, 1)),
  "accountVerified" INTEGER CHECK ("accountVerified" IN (0, 1)),
  "followerCount" INTEGER,
  "profileSnapshotState" TEXT CHECK ("profileSnapshotState" IN ('pending', 'captured', 'unavailable', 'failed')),
  "profileSnapshotAttempts" INTEGER,
  "profileSnapshotNextAttemptAt" INTEGER,
  "language" TEXT,
  "personaId" TEXT,
  "contactLastReadAt" INTEGER,
  "firstInteractionAt" INTEGER,
  "lastMessageAt" INTEGER,
  "lastIncomingMessageAt" INTEGER,
  "lastOutboundMessageAt" INTEGER,
  "referral" TEXT CHECK ("referral" IS NULL OR json_valid("referral")),
  "lastCommentMessageId" TEXT,
  "lastCommentMessageAt" INTEGER,
  "lastStoryId" TEXT,
  "consecutiveFailedReply" INTEGER NOT NULL DEFAULT 0,
  "lastInputFailure" TEXT,
  "lastErrorLog" TEXT,
  "lastBtnTitle" TEXT,
  "lastUserInput" TEXT,
  "lastUserInputType" TEXT CHECK ("lastUserInputType" IN ('text', 'location', 'refLink', 'image', 'video', 'audio', 'gif', 'file', 'storyMention')),
  "webchatParentUrl" TEXT,
  "sourceUserId" TEXT,
  "sourceParentUserId" TEXT,
  "sourceUsername" TEXT,
  "sourceIdentityHistory" TEXT CHECK ("sourceIdentityHistory" IS NULL OR json_valid("sourceIdentityHistory")),
  "threadControlState" TEXT CHECK ("threadControlState" IN ('owned', 'standby', 'idle')),
  "threadOwnerRole" TEXT,
  "threadControlUpdatedAt" INTEGER,
  "threadControlLastEvent" TEXT CHECK ("threadControlLastEvent" IN ('inboundReceived', 'standbyReceived', 'controlPassed', 'controlTaken', 'taken', 'released', 'passed', 'serviceSent', 'serviceRejected')),
  "threadOwnerAppId" TEXT,
  "threadOwnerExpiresAt" INTEGER,
  "threadPreviousOwnerAppId" TEXT
) STRICT;
CREATE UNIQUE INDEX "ContactInbox_inboxId_sourceId_key" ON "ContactInbox" ("inboxId", "sourceId");
CREATE UNIQUE INDEX "ContactInbox_inboxId_sourceUserId_key" ON "ContactInbox" ("inboxId", "sourceUserId") WHERE "sourceUserId" IS NOT NULL;
CREATE UNIQUE INDEX "ContactInbox_inboxId_sourceParentUserId_key" ON "ContactInbox" ("inboxId", "sourceParentUserId") WHERE "sourceParentUserId" IS NOT NULL;
CREATE INDEX "ContactInbox_profileSnapshot_pending_idx" ON "ContactInbox" ("profileSnapshotNextAttemptAt", "id") WHERE "profileSnapshotState" = 'pending';
CREATE INDEX "ContactInbox_inboxId_id_idx" ON "ContactInbox" ("inboxId", "id");
CREATE INDEX "ContactInbox_inboxId_id_ai_held_idx" ON "ContactInbox" ("inboxId", "id") WHERE "threadControlState" = 'standby' AND "threadOwnerRole" = 'ai_agent';
CREATE INDEX "ContactInbox_contactId_lastIncomingMessageAt_idx" ON "ContactInbox" ("contactId", "lastIncomingMessageAt");
CREATE INDEX "ContactInbox_contactId_lastOutboundMessageAt_idx" ON "ContactInbox" ("contactId", "lastOutboundMessageAt");
CREATE INDEX "ContactInbox_referral_ctwaClid_idx" ON "ContactInbox" (("referral"->>'ctwaClid')) WHERE "referral"->>'ctwaClid' IS NOT NULL;
CREATE INDEX "ContactInbox_referral_adId_idx" ON "ContactInbox" (("referral"->>'adId')) WHERE "referral"->>'adId' IS NOT NULL AND "referral"->>'source' = 'ADS';

-- ContactNote
CREATE TABLE "ContactNote" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "text" TEXT NOT NULL,
  "contactId" INTEGER NOT NULL REFERENCES "Contact" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "createdById" INTEGER REFERENCES "User" ("id") ON DELETE CASCADE ON UPDATE CASCADE
) STRICT;
CREATE INDEX "ContactNote_contactId_idx" ON "ContactNote" ("contactId");

-- ContactCustomField
CREATE TABLE "ContactCustomField" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "value" TEXT NOT NULL,
  "contactId" INTEGER NOT NULL REFERENCES "Contact" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "customFieldId" INTEGER NOT NULL REFERENCES "CustomField" ("id") ON DELETE CASCADE ON UPDATE CASCADE
) STRICT;
CREATE UNIQUE INDEX "ContactCustomField_contactId_customFieldId_key" ON "ContactCustomField" ("contactId", "customFieldId");
CREATE INDEX "ContactCustomField_customFieldId_id_idx" ON "ContactCustomField" ("customFieldId", "id");

-- ContactToTag
CREATE TABLE "ContactToTag" (
  "contactId" INTEGER NOT NULL REFERENCES "Contact" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "tagId" INTEGER NOT NULL REFERENCES "Tag" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  CONSTRAINT "ContactToTag_pkey" PRIMARY KEY ("contactId", "tagId")
) STRICT;
CREATE INDEX "ContactToTag_tagId_idx" ON "ContactToTag" ("tagId");

-- ContactToTagChannel
CREATE TABLE "ContactToTagChannel" (
  "tagId" INTEGER NOT NULL REFERENCES "Tag" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "tagChannelId" INTEGER NOT NULL REFERENCES "TagChannel" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "contactInboxId" INTEGER NOT NULL REFERENCES "ContactInbox" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  CONSTRAINT "ContactToTagChannel_pkey" PRIMARY KEY ("tagChannelId", "contactInboxId")
) STRICT;
CREATE INDEX "ContactToTagChannel_contactInboxId_idx" ON "ContactToTagChannel" ("contactInboxId");
CREATE INDEX "ContactToTagChannel_tagId_idx" ON "ContactToTagChannel" ("tagId");

-- ContactState
-- Postgres: HASH(workspaceId) x64 in Postgres, fillfactor 80. SQLite: one table, same PK (DECISIONS P2.2).
CREATE TABLE "ContactState" (
  "workspaceId" INTEGER NOT NULL,
  "contactId" INTEGER NOT NULL,
  "lastDeliveredAt" INTEGER,
  CONSTRAINT "ContactState_pkey" PRIMARY KEY ("workspaceId", "contactId")
) STRICT;
CREATE INDEX "ContactState_lastDeliveredAt_idx" ON "ContactState" ("workspaceId", "lastDeliveredAt") WHERE "lastDeliveredAt" IS NOT NULL;

-- ContactActivity
-- Postgres: HASH(workspaceId) x64 in Postgres. SQLite: one table, same PK (DECISIONS P2.2).
CREATE TABLE "ContactActivity" (
  "workspaceId" INTEGER NOT NULL,
  "contactId" INTEGER NOT NULL,
  "activityType" TEXT NOT NULL CHECK ("activityType" IN ('flowExecuted', 'flowNodeExecuted', 'automatedResponseMatched', 'formCompleted', 'messageSent', 'messageDelivered', 'messageOpened', 'messageClicked', 'adClicked')),
  "referenceType" TEXT NOT NULL CHECK ("referenceType" IN ('flow', 'flowNode', 'automatedResponse', 'whatsappFlow', 'emailTopic', 'ad')),
  "parentReferenceId" INTEGER NOT NULL,
  "referenceId" INTEGER NOT NULL,
  "firstOccurredAt" INTEGER NOT NULL,
  CONSTRAINT "ContactActivity_pkey" PRIMARY KEY ("workspaceId", "contactId", "activityType", "referenceType", "parentReferenceId", "referenceId"),
  CONSTRAINT "ContactActivity_allowed_pair_check" CHECK (("activityType" = 'flowExecuted' AND "referenceType" = 'flow' AND "parentReferenceId" = 0) OR ("activityType" = 'flowNodeExecuted' AND "referenceType" = 'flowNode' AND "parentReferenceId" <> 0) OR ("activityType" = 'automatedResponseMatched' AND "referenceType" = 'automatedResponse' AND "parentReferenceId" = 0) OR ("activityType" = 'formCompleted' AND "referenceType" = 'whatsappFlow' AND "parentReferenceId" = 0) OR ("activityType" = 'messageSent' AND "referenceType" = 'emailTopic' AND "parentReferenceId" = 0) OR ("activityType" = 'messageDelivered' AND "referenceType" = 'emailTopic' AND "parentReferenceId" = 0) OR ("activityType" = 'messageOpened' AND "referenceType" = 'emailTopic' AND "parentReferenceId" = 0) OR ("activityType" = 'messageClicked' AND "referenceType" = 'emailTopic' AND "parentReferenceId" = 0) OR ("activityType" = 'adClicked' AND "referenceType" = 'ad' AND "parentReferenceId" = 0))
) STRICT;
CREATE INDEX "ContactActivity_reference_idx" ON "ContactActivity" ("workspaceId", "activityType", "referenceType", "parentReferenceId", "referenceId", "contactId");

-- ContactKeywordMessage
-- Postgres: HASH(workspaceId) x64 in Postgres. SQLite: one table, same PK (DECISIONS P2.2).
CREATE TABLE "ContactKeywordMessage" (
  "workspaceId" INTEGER NOT NULL,
  "contactId" INTEGER NOT NULL,
  "messageId" INTEGER NOT NULL,
  "automatedResponseId" INTEGER NOT NULL,
  "text" TEXT NOT NULL,
  "normalizedText" TEXT NOT NULL,
  "occurredAt" INTEGER NOT NULL,
  CONSTRAINT "ContactKeywordMessage_pkey" PRIMARY KEY ("workspaceId", "contactId", "messageId")
) STRICT;
-- Postgres opclass text_pattern_ops on "normalizedText". SQLite BINARY index; prefix queries must use a range
-- ("normalizedText" >= :p AND "normalizedText" < :p_upper), because the app LIKE function cannot use an index.
CREATE INDEX "ContactKeywordMessage_text_prefix_idx" ON "ContactKeywordMessage" ("workspaceId", "normalizedText");

-- ContactInboxPost
-- Postgres: HASH(workspaceId) x64 in Postgres. SQLite: one table, same PK (DECISIONS P2.2).
CREATE TABLE "ContactInboxPost" (
  "workspaceId" INTEGER NOT NULL,
  "contactInboxId" INTEGER NOT NULL,
  "postId" INTEGER NOT NULL,
  "commentedAt" INTEGER NOT NULL,
  CONSTRAINT "ContactInboxPost_pkey" PRIMARY KEY ("workspaceId", "contactInboxId", "postId")
) STRICT;
CREATE INDEX "ContactInboxPost_workspaceId_postId_idx" ON "ContactInboxPost" ("workspaceId", "postId");

-- CustomField
CREATE TABLE "CustomField" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "name" TEXT NOT NULL,
  "type" TEXT NOT NULL CHECK ("type" IN ('shortText', 'email', 'phoneNumber', 'number', 'date', 'datetime', 'boolean', 'longText')),
  "description" TEXT,
  "folderId" INTEGER REFERENCES "Folder" ("id") ON DELETE SET NULL ON UPDATE CASCADE,
  "showInInbox" INTEGER NOT NULL DEFAULT 0 CHECK ("showInInbox" IN (0, 1)),
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE
) STRICT;
CREATE UNIQUE INDEX "CustomField_workspaceId_type_name_key" ON "CustomField" ("workspaceId", "type", "name");

-- BotField
CREATE TABLE "BotField" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "name" TEXT NOT NULL,
  "type" TEXT NOT NULL CHECK ("type" IN ('shortText', 'email', 'phoneNumber', 'number', 'date', 'datetime', 'boolean', 'longText')),
  "value" TEXT,
  "description" TEXT,
  "folderId" INTEGER REFERENCES "Folder" ("id") ON DELETE SET NULL ON UPDATE CASCADE,
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE
) STRICT;
CREATE UNIQUE INDEX "BotField_workspaceId_type_name_key" ON "BotField" ("workspaceId", "type", "name");

-- SystemField
CREATE TABLE "SystemField" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "type" TEXT NOT NULL CHECK ("type" IN ('me')),
  "payload" TEXT NOT NULL CHECK (json_valid("payload"))
) STRICT;

-- Tag
CREATE TABLE "Tag" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "name" TEXT NOT NULL,
  "deletedAt" INTEGER,
  "folderId" INTEGER REFERENCES "Folder" ("id") ON DELETE SET NULL ON UPDATE CASCADE,
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE
) STRICT;
CREATE UNIQUE INDEX "Tag_workspaceId_name_key" ON "Tag" ("workspaceId", "name") WHERE ("deletedAt" is null);
CREATE INDEX "Tag_folderId_idx" ON "Tag" ("folderId");

-- TagChannel
CREATE TABLE "TagChannel" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "tagId" INTEGER NOT NULL REFERENCES "Tag" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "channelType" TEXT NOT NULL,
  "integrationId" INTEGER NOT NULL,
  "externalLabelId" TEXT NOT NULL
) STRICT;
CREATE UNIQUE INDEX "TagChannel_tag_integration_key" ON "TagChannel" ("tagId", "channelType", "integrationId");
CREATE UNIQUE INDEX "TagChannel_external_key" ON "TagChannel" ("channelType", "integrationId", "externalLabelId");
CREATE INDEX "TagChannel_workspace_channel_idx" ON "TagChannel" ("workspaceId", "channelType");
CREATE INDEX "TagChannel_integration_idx" ON "TagChannel" ("channelType", "integrationId");

-- UserPersistentMenu
CREATE TABLE "UserPersistentMenu" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "name" TEXT NOT NULL,
  "menus" TEXT NOT NULL CHECK (json_valid("menus")),  -- NO DB default (Postgres parity: drizzle .default(sql`[]`) is app-side only). Rust always writes it.
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE
) STRICT;
CREATE INDEX "UserPersistentMenu_workspaceId_idx" ON "UserPersistentMenu" ("workspaceId");

-- ===================================================================================
-- Triggers: InboxContactStat (port of increment_contact_stats / decrement_contact_stats,
-- migration 20260624082115_fix_contact_inbox_stats). Postgres CURRENT_TIMESTAMP -> unix ms.
-- ===================================================================================
CREATE TRIGGER "ContactInbox_stats_ai" AFTER INSERT ON "ContactInbox" BEGIN
  INSERT INTO "InboxContactStat" ("inboxId", "totalContacts", "updatedAt")
  VALUES (NEW."inboxId", 1, CAST(unixepoch('subsec') * 1000 AS INTEGER))
  ON CONFLICT ("inboxId") DO UPDATE SET
    "totalContacts" = "totalContacts" + 1,
    "updatedAt" = excluded."updatedAt";
END;
CREATE TRIGGER "ContactInbox_stats_ad" AFTER DELETE ON "ContactInbox" BEGIN
  UPDATE "InboxContactStat"
     SET "totalContacts" = max("totalContacts" - 1, 0),
         "updatedAt" = CAST(unixepoch('subsec') * 1000 AS INTEGER)
   WHERE "inboxId" = OLD."inboxId";
END;

-- ===================================================================================
-- FTS5 trigram tables (replace the 5 pg_trgm GIN indexes; P2.2 section 1.10).
-- Use MATCH with a quoted phrase for candidates, then re-check with ilike()/like_cs()
-- (section 3). Do NOT use LIKE on the FTS columns: FTS5 LIKE folds ASCII only.
-- ===================================================================================
CREATE VIRTUAL TABLE "Contact_fts" USING fts5(
  "firstName", "lastName", "email", "phoneNumber",
  content='Contact', content_rowid='id', tokenize='trigram case_sensitive 0'
);
CREATE TRIGGER "Contact_fts_ai" AFTER INSERT ON "Contact" BEGIN
  INSERT INTO "Contact_fts"(rowid, "firstName", "lastName", "email", "phoneNumber")
  VALUES (NEW."id", NEW."firstName", NEW."lastName", NEW."email", NEW."phoneNumber");
END;
CREATE TRIGGER "Contact_fts_ad" AFTER DELETE ON "Contact" BEGIN
  INSERT INTO "Contact_fts"("Contact_fts", rowid, "firstName", "lastName", "email", "phoneNumber")
  VALUES ('delete', OLD."id", OLD."firstName", OLD."lastName", OLD."email", OLD."phoneNumber");
END;
CREATE TRIGGER "Contact_fts_au" AFTER UPDATE OF "firstName", "lastName", "email", "phoneNumber" ON "Contact" BEGIN
  INSERT INTO "Contact_fts"("Contact_fts", rowid, "firstName", "lastName", "email", "phoneNumber")
  VALUES ('delete', OLD."id", OLD."firstName", OLD."lastName", OLD."email", OLD."phoneNumber");
  INSERT INTO "Contact_fts"(rowid, "firstName", "lastName", "email", "phoneNumber")
  VALUES (NEW."id", NEW."firstName", NEW."lastName", NEW."email", NEW."phoneNumber");
END;

-- ContactKeywordMessage has a composite PK, so its implicit rowid can change on a full VACUUM.
-- The FTS row id is "messageId" (one row per message, see the schema comment), with a
-- SQLite-only lookup index so FTS5 can fetch content rows by it.
CREATE INDEX "ContactKeywordMessage_messageId_idx" ON "ContactKeywordMessage" ("messageId");
CREATE VIRTUAL TABLE "ContactKeywordMessage_fts" USING fts5(
  "normalizedText",
  content='ContactKeywordMessage', content_rowid='messageId', tokenize='trigram case_sensitive 1'
);
CREATE TRIGGER "ContactKeywordMessage_fts_ai" AFTER INSERT ON "ContactKeywordMessage" BEGIN
  INSERT INTO "ContactKeywordMessage_fts"(rowid, "normalizedText") VALUES (NEW."messageId", NEW."normalizedText");
END;
CREATE TRIGGER "ContactKeywordMessage_fts_ad" AFTER DELETE ON "ContactKeywordMessage" BEGIN
  INSERT INTO "ContactKeywordMessage_fts"("ContactKeywordMessage_fts", rowid, "normalizedText")
  VALUES ('delete', OLD."messageId", OLD."normalizedText");
END;
CREATE TRIGGER "ContactKeywordMessage_fts_au" AFTER UPDATE OF "normalizedText", "messageId" ON "ContactKeywordMessage" BEGIN
  INSERT INTO "ContactKeywordMessage_fts"("ContactKeywordMessage_fts", rowid, "normalizedText")
  VALUES ('delete', OLD."messageId", OLD."normalizedText");
  INSERT INTO "ContactKeywordMessage_fts"(rowid, "normalizedText") VALUES (NEW."messageId", NEW."normalizedText");
END;

-- ===================================================================================
-- Seed: root tenant (migration 20260614163529_add_tenant_tables, verbatim intent).
-- "User"."tenantId", "Account"."tenantId" and "Workspace"."tenantId" default to 1.
-- ===================================================================================
INSERT INTO "Tenant" ("id", "status") VALUES (1, 'active') ON CONFLICT ("id") DO NOTHING;
