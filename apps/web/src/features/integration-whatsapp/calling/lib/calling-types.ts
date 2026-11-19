// The calling types of integrations/whatsapp/src/api/calling.ts (the SPA has no
// @chatbotx.io/integration-whatsapp dependency).

export type WhatsappCallingWeeklyHours = {
  day_of_week:
    | "MONDAY"
    | "TUESDAY"
    | "WEDNESDAY"
    | "THURSDAY"
    | "FRIDAY"
    | "SATURDAY"
    | "SUNDAY"
  open_time: string
  close_time: string
}

export type WhatsappCallingHolidaySchedule = {
  date: string
  start_time: string
  end_time: string
}

export type WhatsappCallHours = {
  status: "ENABLED" | "DISABLED"
  timezone_id: string
  // Meta omits this entirely when the number has no weekly schedule set (e.g.
  // calling enabled with hours never configured), so it is genuinely optional.
  weekly_operating_hours?: WhatsappCallingWeeklyHours[]
  holiday_schedule?: WhatsappCallingHolidaySchedule[]
}

export type WhatsappCallingAudioSettings = {
  /** Extra codecs offered alongside Opus (Meta calling settings reference). */
  additional_codecs?: ("PCMA" | "PCMU")[]
}

export type WhatsappCallingSettings = {
  status: "ENABLED" | "DISABLED"
  call_icon_visibility?: "DEFAULT" | "DISABLE_ALL"
  callback_permission_status?: "ENABLED" | "DISABLED"
  call_hours?: WhatsappCallHours
  srtp_key_exchange_protocol?: "DTLS" | "SDES"
  audio?: WhatsappCallingAudioSettings
}
