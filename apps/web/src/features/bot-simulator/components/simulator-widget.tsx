"use client"

import { useEffect } from "react"

type CsmChatWidget = {
  init: (config: {
    webchatId: string
    workspaceId: string
    brandColor: string
    parentUrl: string
  }) => void
}

type SimulatorWidgetProps = {
  webchatId: string
  workspaceId: string
  brandColor: string
  websiteUrl: string
}

/**
 * Loads the same `plugin.js` a customer pastes on their site, so the simulator
 * shows exactly what the real embed would. `parentUrl` is the simulated site,
 * not this page, so the conversation records the website being demoed.
 */
export function SimulatorWidget({
  webchatId,
  workspaceId,
  brandColor,
  websiteUrl,
}: SimulatorWidgetProps) {
  // ponytail: the next/script shim takes no load props, so the tag is made here.
  useEffect(() => {
    const script = document.createElement("script")
    script.type = "module"
    script.crossOrigin = "anonymous"
    script.src = "/chat-widget/plugin.js"
    script.onload = () => {
      const widget = (window as Window & { csmChatWidget?: CsmChatWidget })
        .csmChatWidget
      widget?.init({ webchatId, workspaceId, brandColor, parentUrl: websiteUrl })
    }
    document.body.appendChild(script)
    return () => script.remove()
  }, [webchatId, workspaceId, brandColor, websiteUrl])

  return null
}
