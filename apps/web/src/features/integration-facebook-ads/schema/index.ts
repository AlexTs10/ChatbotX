// The settings row (`integrationFacebookAdsResource`) comes from the Rust
// server (`facebookAdsSettingsPage`); no `auth` reaches the client.
export type IntegrationFacebookAdsResource = {
  id: string
  workspaceId: string
  integrationId: string
  tokenExpiresAt: string | Date | null
  status: "active" | "invalid"
}
