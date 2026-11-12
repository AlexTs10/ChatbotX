// Fixture extension of the batch P1.12–P1.14 (public API namespaces
// miniApps, minigames, openHours, orders, paymentProviders, pipelines,
// placeCategories, placeLocations, polls, productCategories, productVendors,
// products, qrCodes, questionnaires, realtime, reflinks). Setup that has no
// public HTTP path only; every assertion stays black-box. Each stack
// implements `create()` in `stacks/<stack>/ext/p1-12.ts`. Load with
// `await loadFixtureExt<P112Fixtures>("p1-12")`.
//
// Ids are decimal strings. Rows are the ones the app writes itself (see the
// stack file for the source of each row). No method returns data that a test
// asserts on, except the ids (and timestamps) of the rows it wrote.

export type P112WhatsappChannelInput = {
  workspaceId: string
  /** Meta phone number id (Graph calls go to `/<version>/<phoneNumberId>/...`). */
  phoneNumberId: string
  name: string
}

export type P112WhatsappChannel = {
  /** `IntegrationWhatsapp.id` (the "WhatsApp channel id" of the public API). */
  integrationId: string
  inboxId: string
  /** `wa-token-<phoneNumberId>` (sent as `Bearer`). */
  accessToken: string
  /** `waba-<phoneNumberId>`: Flow creates go to `/<version>/<wabaId>/flows`. */
  wabaId: string
}

export type P112MiniAppSubmissionInput = {
  workspaceId: string
  miniAppId: string
  /** Default null (an anonymous web submission). */
  contactId?: string | null
  answers: Record<string, unknown>
  /** ISO time with millisecond precision. Default: now. */
  createdAt?: string
}

export type P112MinigamePlayerInput = {
  minigameId: string
  contactId: string
  /** Default null. */
  contactInboxId?: string | null
  played: number
  remaining: number
  /** Default 0. */
  sharesCount?: number
  /** ISO time. Default: now. */
  openedAt?: string
  /** ISO time of the last play (`MinigameContact.updatedAt`). Default: now. */
  lastPlayedAt?: string
}

export type P112MinigamePlayInput = {
  minigameId: string
  contactId: string
  isWinning: boolean
  /** Default null. */
  prizeId?: string | null
  /** Default null. */
  prizeName?: string | null
  /** ISO time with millisecond precision. Default: now. */
  createdAt?: string
}

export type P112PollVoteInput = {
  workspaceId: string
  pollId: string
  candidateId: string
  contactId: string
  /** Default null. */
  contactInboxId?: string | null
}

export type P112MetaCatalogConnectionInput = {
  workspaceId: string
  /** Stored encrypted as the OAuth connect stores it (`{accessToken, version:"v23.0"}`). */
  accessToken: string
  /** Default null (no catalog bound yet). */
  catalogId?: string | null
  catalogName?: string | null
  businessId?: string | null
}

export type P112QuestionnaireAnswerInput = {
  /** `QuestionnaireQuestion.id` the answer belongs to. */
  questionId: string
  questionTitle: string
  questionType: string
  label: string
  value: string | number | { optionId: string; label: string } | null
  /** Default null. */
  pointsEarned?: number | null
}

export type P112QuestionnaireSubmissionInput = {
  workspaceId: string
  questionnaireId: string
  contactId: string
  /** Default null. */
  conversationId?: string | null
  status: "inProgress" | "completed" | "cancelled" | "failed" | "timeout"
  /** Default null. */
  totalPoints?: number | null
  /** ISO time. Default: now when status is completed, else null. */
  completedAt?: string | null
  /** ISO time with millisecond precision. Default: now. */
  createdAt?: string
  /** One `QuestionnaireAnswer` row each, in this order (answeredAt / createdAt one ms apart). */
  answers?: P112QuestionnaireAnswerInput[]
}

export interface P112Fixtures {
  /** A connected WhatsApp number (`Inbox` + `IntegrationWhatsapp`, plain JSON oauth2 auth, + `Connection`). */
  createWhatsappChannel(input: P112WhatsappChannelInput): Promise<P112WhatsappChannel>
  /**
   * One `MiniAppSubmission` row (source "web") as the public Mini App page
   * writes it, and the `MiniApp.submissionsCount` + 1 of the same write.
   */
  createMiniAppSubmission(input: P112MiniAppSubmissionInput): Promise<{ submissionId: string; createdAt: string }>
  /** One `MinigameContact` row (a player who opened / played the game). */
  createMinigamePlayer(input: P112MinigamePlayerInput): Promise<{ playerId: string }>
  /** One `MinigamePlay` row (one draw). */
  createMinigamePlay(input: P112MinigamePlayInput): Promise<{ playId: string; createdAt: string }>
  /** One `PollVote` row (a contact's vote, as the chat vote path writes it). P1.13. */
  createPollVote(input: P112PollVoteInput): Promise<{ voteId: string }>
  /**
   * A Meta Catalog connection as the OAuth connect writes it (`Integration`
   * type metaCatalog + `IntegrationMetaCatalog`, status active, auth mode
   * oauth, encrypted auth). P1.14.
   */
  createMetaCatalogConnection(
    input: P112MetaCatalogConnectionInput,
  ): Promise<{ connectionId: string; integrationId: string }>
  /** A `QuestionnaireSubmission` with its `QuestionnaireAnswer` rows, as the chat path writes them. P1.14. */
  createQuestionnaireSubmission(input: P112QuestionnaireSubmissionInput): Promise<{ submissionId: string }>
  close(): Promise<void>
}
