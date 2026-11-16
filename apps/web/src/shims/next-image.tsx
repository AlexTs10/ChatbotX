// `next/image`: a plain <img> (no optimizer in the SPA).
import type { CSSProperties, ImgHTMLAttributes } from "react"

type ImageProps = Omit<ImgHTMLAttributes<HTMLImageElement>, "src"> & {
  src: string | { src: string }
  fill?: boolean
  priority?: boolean
  quality?: number
  unoptimized?: boolean
  placeholder?: string
  blurDataURL?: string
}

export default function Image({
  src,
  fill,
  priority,
  quality: _quality,
  unoptimized: _unoptimized,
  placeholder: _placeholder,
  blurDataURL: _blurDataURL,
  style,
  alt,
  ...props
}: ImageProps) {
  const fillStyle: CSSProperties | undefined = fill
    ? {
        position: "absolute",
        inset: 0,
        width: "100%",
        height: "100%",
        objectFit: "cover",
      }
    : undefined
  return (
    // biome-ignore lint/performance/noImgElement: the SPA has no image optimizer; next/image becomes <img>
    // biome-ignore lint/correctness/useImageSize: width/height come from the caller's props, as with next/image
    <img
      alt={alt}
      loading={priority ? "eager" : "lazy"}
      src={typeof src === "string" ? src : src.src}
      style={{ ...fillStyle, ...style }}
      {...props}
    />
  )
}
