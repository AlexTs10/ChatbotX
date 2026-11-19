// The preflight type of apps/builder/src/features/integration-whatsapp/calling/get-whatsapp-calling-preflight.ts.
// The Rust server computes it (`whatsappCallsPage`).
export type WhatsappCallingPreflight = {
  /** Manually connected number: no app credential exists to check/fix. */
  isManual: boolean
  /** Whether the owner has a WhatsApp app credential for the subscription check. */
  hasAppCredential: boolean
  /** null = could not be determined (no credential, or the GET failed). */
  callsSubscribed: boolean | null
  platformType: string | null
  isCloudApiPlatform: boolean | null
  messagingLimitTier: string | null
  callingIneligibleReason: string | null
}
