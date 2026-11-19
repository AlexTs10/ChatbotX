// `next/script`: adds the script to <head> once.
import { useEffect } from "react"

type ScriptProps = {
  id?: string
  src?: string
  strategy?: string
  children?: string
  dangerouslySetInnerHTML?: { __html: string }
  onLoad?: () => void
  crossOrigin?: string
  type?: string
}

export default function Script({
  id,
  src,
  children,
  dangerouslySetInnerHTML,
  onLoad,
  crossOrigin,
  type,
}: ScriptProps) {
  useEffect(() => {
    if (id && document.getElementById(id)) {
      return
    }
    const script = document.createElement("script")
    if (id) {
      script.id = id
    }
    if (src) {
      script.src = src
      script.async = true
    }
    if (crossOrigin) {
      script.crossOrigin = crossOrigin
    }
    if (type) {
      script.type = type
    }
    if (onLoad) {
      script.addEventListener("load", onLoad)
    }
    const inline = dangerouslySetInnerHTML?.__html ?? children
    if (inline) {
      script.text = inline
    }
    document.head.appendChild(script)
  }, [id, src, children, dangerouslySetInnerHTML, onLoad, crossOrigin, type])
  return null
}
