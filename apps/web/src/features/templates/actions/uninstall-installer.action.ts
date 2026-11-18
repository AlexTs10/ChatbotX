// Generated from apps/builder/src/features/templates/actions/uninstall-installer.action.ts by apps/web/scripts/copy-from-builder.mjs.
// The server code runs in the Rust server: POST /rpc/actions/<name>.
import { serverAction } from "@/lib/server-action"

export const previewUninstallInstallerAction = serverAction("previewUninstallInstallerAction", { noInput: true })
export const uninstallInstallerAction = serverAction("uninstallInstallerAction", { noInput: true })
