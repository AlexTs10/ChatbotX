// Fixture extension of unit P1.4 (appointments, broadcasts, bot fields,
// capabilities, carts, channel integrations, channel posts). Setup that has no
// public HTTP path. Stack-neutral types only; each stack writes the same
// logical rows the app writes (see stacks/<stack>/ext/p1-04.ts).

export type GoogleCalendarConnectionInput = {
  workspaceId: string
  /** Google calendar id, e.g. "parity-<id>" (plain characters only). */
  providerCalendarId: string
  email: string | null
  accessToken: string
  refreshToken: string
}

export type ChannelPostInput = {
  workspaceId: string
  inboxId: string
  channel: string
  integrationId: string
  sourceAccountId: string
  externalPostId: string
  caption: string | null
  permalink: string | null
  /** Storage key; the API returns it as a public URL. */
  thumbnail: string | null
  publishedAt: Date | null
}

export type AppointmentInput = {
  workspaceId: string
  calendarId: string
  contactId: string
  startAt: Date
  endAt: Date
  inviteeTimezone: string
  status: "scheduled" | "cancelled"
  locationType: "inPerson" | "phoneCall" | "onlineMeeting"
}

export interface P104FixtureExt {
  /**
   * A Google Calendar connection as the OAuth callback writes it
   * (`appointmentExternalCalendarService.createGoogleFromOAuthCallback`):
   * an `Integration` row (type googleCalendar) plus its
   * `IntegrationGoogleCalendar` row holding the OAuth auth blob.
   */
  createGoogleCalendarConnection(input: GoogleCalendarConnectionInput): Promise<{ integrationId: string }>
  /** An inbox row (no integration behind it), e.g. for channel posts. */
  createInbox(input: {
    workspaceId: string
    channel: string
    name: string
    sourceId: string
  }): Promise<{ inboxId: string }>
  /** A connected Zalo OA: `Inbox` (channel zalo) + `IntegrationZalo`. */
  createZaloIntegration(input: {
    workspaceId: string
    name: string
    oaId: string
  }): Promise<{ integrationId: string; inboxId: string }>
  /** A tracked post, as the comment webhooks write it (`ChannelPost`). */
  createChannelPost(input: ChannelPostInput): Promise<{ channelPostId: string }>
  /** A web shop cart (`Cart`), as the shop or a WhatsApp catalog order writes it. */
  createCart(input: {
    workspaceId: string
    contactId: string
    items: { productId: string; variantId: string | null; quantity: number }[]
  }): Promise<{ cartId: string }>
  /** An appointment row (e.g. one in the past, which the API cannot book). */
  createAppointment(input: AppointmentInput): Promise<{ appointmentId: string }>
  /** Read-only setup lookup: the direct conversation `contacts.create` opened. */
  conversationIdForContact(input: { workspaceId: string; contactId: string }): Promise<string>
  close(): Promise<void>
}
