// The SPA keeps the type only: the preview runs in the Rust server
// (`previewInstallCollisionsAction`).
import type {
  PredictedAction,
  PredictedCollision,
} from "@chatbotx.io/business"

export type InstallCollisionPreview = {
  collisions: PredictedCollision[]
  summary: Record<PredictedAction, number>
}
