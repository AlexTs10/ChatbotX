// Generated from apps/builder/src/features/templates/actions/uninstall-installation.action.ts by apps/web/scripts/copy-from-builder.mjs.
// The server code runs in the Rust server: POST /rpc/actions/<name>.
import { serverAction } from "@/lib/server-action"

export const previewUninstallInstallationAction = serverAction("previewUninstallInstallationAction", { noInput: true })
export const uninstallInstallationAction = serverAction("uninstallInstallationAction", { noInput: true })
export const cancelUninstallInstallationAction = serverAction("cancelUninstallInstallationAction", { noInput: true })
