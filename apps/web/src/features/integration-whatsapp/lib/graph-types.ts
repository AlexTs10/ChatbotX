// Copies of the browser-safe parts of `@chatbotx.io/integration-whatsapp`
// (`constants.ts`, `api/phone-number.ts`, `api/waba.ts`): apps/web does not
// depend on that package (its Graph client is server code).
// ponytail: copied, not shared; add a browser entry to the package if more is needed.

export const WHATSAPP_VERIFICATION_CODE_METHODS = {
  SMS: "SMS",
  VOICE: "VOICE",
} as const

export type WhatsappMarketingMessagesLiteApiStatus =
  | "INELIGIBLE_ON_BEHALF_OF_WABA"
  | "INELIGIBLE_INACTIVE_OR_RESTRICTED"
  | "INELIGIBLE_COUNTRY_NOT_SUPPORTED"
  | "INELIGIBLE_USING_WHATSAPP_BUSINESS_APP"
  | "ELIGIBLE"
  | "PENDING_VALID_PAYMENT_METHOD"
  | "PENDING_INTERNAL_SETUP"
  | "ONBOARDED"

export type WhatsappWabaMMLite = {
  marketing_messages_onboarding_status?: WhatsappMarketingMessagesLiteApiStatus
}

export type WhatsappEntityCanSendMessage = "AVAILABLE" | "LIMITED" | "BLOCKED"

export type WhatsappEntityType =
  | "PHONE_NUMBER"
  | "WABA"
  | "BUSINESS"
  | "MESSAGE_TEMPLATE"
  | "APP"

export type WhatsappHealthError = {
  error_code?: number
  error_description?: string
  possible_solution?: string
}

export type WhatsappHealthEntity = {
  entity_type: WhatsappEntityType
  id?: string
  can_send_message?: WhatsappEntityCanSendMessage
  additional_info?: string[]
  errors?: WhatsappHealthError[]
}

export type WhatsappHealthStatus = {
  can_send_message?: WhatsappEntityCanSendMessage
  entities?: WhatsappHealthEntity[]
}

export type WhatsappPhoneNumberDetail = {
  id: string
  quality_rating?: string
  /** @deprecated read `whatsapp_business_manager_messaging_limit`. */
  messaging_limit_tier?: string
  whatsapp_business_manager_messaging_limit?: string
  code_verification_status?: string
  account_mode?: string
  display_phone_number?: string
  name_status?: string
  verified_name?: string
  webhook_configuration?: {
    application?: string
    waba_application?: string
    smb_app_data?: Record<string, unknown>
  }
  throughput?: { level?: string }
  last_onboarded_time?: string
  platform_type?: string
  certificate?: string
  health_status?: WhatsappHealthStatus
}

export type ConversationalAutomation = {
  enable_welcome_message: boolean
  prompts: string[]
  commands: {
    command_name: string
    command_description: string
  }[]
}
