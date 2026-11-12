// Fixture extension of unit P1.11 (public API namespaces `magicLinks`, `me`,
// `mediaLibrary`, `messages`, `messengerChannels`, `messengerPersonas`,
// `messengerTemplates`). Setup that has no public HTTP path only; every
// assertion stays black-box. Each stack implements `create()` in
// `stacks/<stack>/ext/p1-11.ts`. Load with
// `await loadFixtureExt<P111Fixtures>("p1-11")`.
//
// Ids are decimal strings. Rows are the ones the app writes itself (see the
// stack file for the source of each row). No method returns data that a test
// asserts on, except the ids (and timestamps) of the rows it wrote.

/** A Messenger persona as the settings save stores it (`IntegrationMessenger.personas`). */
export type P111StoredPersona = {
  id: string
  name: string
  isDefault: boolean
  /** Set once the persona is registered at Facebook. */
  facebookPersonaId?: string
  profilePicture: { id: string; url: string; mode: "url" | "file" }
}

export type P111MessengerPageInput = {
  workspaceId: string
  /** Facebook page id (numeric string, globally unique). */
  pageId: string
  name: string
  /** Default []. */
  personas?: P111StoredPersona[]
  /** `IntegrationMessenger.personaId` (Facebook id of the default persona). Default null. */
  personaId?: string | null
  /** Default false. */
  hasCapiScope?: boolean
  /** Default null. */
  datasetId?: string | null
  /** Default null. */
  capiTestEventCode?: string | null
  /** Sets `capiDisconnectedAt` to now. Default false. */
  capiDisconnected?: boolean
  /** Default null. */
  tokenRefreshError?: string | null
}

export type P111MessengerPage = {
  integrationId: string
  inboxId: string
  /** Page access token stored in the (plain JSON) oauth2 auth: `page-token-<pageId>`. */
  accessToken: string
}

export type P111WhatsappChannelInput = {
  workspaceId: string
  /** Meta phone number id (Graph calls go to `/<version>/<phoneNumberId>/...`). */
  phoneNumberId: string
  name: string
}

export type P111WhatsappChannel = {
  integrationId: string
  inboxId: string
  /** `wa-token-<phoneNumberId>` (sent as `Bearer`). */
  accessToken: string
}

export type P111ContactInput = {
  workspaceId: string
  firstName: string
  /** One ContactInbox row per entry. */
  inboxes: Array<{
    inboxId: string
    /** `ContactInbox.channel`; must match the inbox channel. */
    channel: string
    /** Channel user id (PSID, WhatsApp digits, ...). Default: unique. */
    sourceId?: string
    /** ISO time. Default null. */
    lastIncomingMessageAt?: string | null
  }>
}

export type P111Contact = {
  contactId: string
  contactInboxIds: string[]
  sourceIds: string[]
  conversationId: string
}

export type P111MessageInput = {
  workspaceId: string
  conversationId: string
  contactInboxId: string
  messageType: "incoming" | "outgoing" | "activity"
  senderType: "bot" | "contact" | "system" | "user" | "api"
  /** `Message.type`. Default "message". */
  type?: "message" | "comment"
  /** Default null. */
  text?: string | null
  /** Channel message id (e.g. a Facebook comment id). Default null. */
  sourceId?: string | null
  /** Default null. */
  contentAttributes?: Record<string, unknown> | null
  /** Default null. */
  parentId?: string | null
  /** ISO time with millisecond precision. Default: now. */
  createdAt?: string
}

export interface P111Fixtures {
  /**
   * A connected Messenger page: `Inbox` (channel "messenger", sourceId =
   * pageId), `IntegrationMessenger` (plain JSON oauth2 auth with clientId
   * "parity-client", clientSecret "parity-secret", version "v23.0", access
   * token `page-token-<pageId>`) and its `Connection` row.
   */
  createMessengerPage(input: P111MessengerPageInput): Promise<P111MessengerPage>
  /**
   * An Instagram-via-Facebook account on `pageId` (`Inbox` +
   * `IntegrationInstagram` type "facebook", auth clientId "parity-client"),
   * the sibling that makes a Messenger disconnect keep the page subscription.
   */
  createInstagramForPage(input: { workspaceId: string; pageId: string; igId: string; name: string }): Promise<{
    integrationId: string
    inboxId: string
  }>
  /** A connected WhatsApp number (`Inbox` + `IntegrationWhatsapp`, plain JSON oauth2 auth). */
  createWhatsappChannel(input: P111WhatsappChannelInput): Promise<P111WhatsappChannel>
  /** A `WhatsappMessageTemplate` row as the template sync writes it. */
  createWhatsappTemplate(input: {
    integrationWhatsappId: string
    name: string
    language: string
    status: "APPROVED" | "PENDING" | "REJECTED"
    components: unknown[]
  }): Promise<{ templateId: string }>
  /** A `MessengerMessageTemplate` row as the template sync writes it. */
  createMessengerTemplate(input: {
    integrationMessengerId: string
    name: string
    language: string
    status: "APPROVED" | "PENDING" | "REJECTED"
    category: string
    components: unknown[]
    /** Meta template id. Default: unique digits. */
    sourceId?: string
  }): Promise<{ templateId: string }>
  /** Contact + its ContactInbox rows + one Conversation. */
  createContact(input: P111ContactInput): Promise<P111Contact>
  /** One `Message` row (as the inbound webhook / send paths write it). */
  createMessage(input: P111MessageInput): Promise<{ messageId: string; createdAt: string }>
  /** Sets `User.mustChangePassword` (the provisioned temporary password flag). */
  setMustChangePassword(input: { userId: string; value: boolean }): Promise<void>
  close(): Promise<void>
}
